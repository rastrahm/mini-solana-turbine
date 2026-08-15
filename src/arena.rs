//! Arena de paquetes de capacidad fija: un backing, muchos slots, cero heap en acquire/release.
//!
//! La única asignación ocurre en [`PacketArena::new`]. El hot path indexa ese bloque
//! y devuelve slices; no hay `Vec<u8>` por paquete.

use crate::Error;
use core::fmt;

/// Bytes útiles por slot (tamaño típico de shred Solana: 1228).
pub const PACKET_SIZE: usize = 1228;

/// Slots por defecto (~1.2 MiB de payloads: `1024 * PACKET_SIZE`).
pub const DEFAULT_SLOT_COUNT: usize = 1024;

/// Arena con el fan-in típico de la fase 5 (`DEFAULT_SLOT_COUNT` slots).
pub type DefaultArena = PacketArena<DEFAULT_SLOT_COUNT>;

/// Handle de un slot ocupado. Opaco: el índice crudo no se usa en APIs públicas.
///
/// `generation` invalida handles después de [`PacketArena::release`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SlotId {
    index: u16,
    generation: u16,
}

impl SlotId {
    /// Purpose: Fabrica un id sin consultar la arena (tests de error).
    /// Inputs: `index` — offset del slot; `generation` — época del handle.
    /// Returns: Un `SlotId` que la arena puede rechazar como fuera de rango o stale.
    pub const fn from_raw(index: u16, generation: u16) -> Self {
        Self { index, generation }
    }

    /// Purpose: Expone el índice interno (debug / tests).
    /// Inputs: `self` — handle copiado.
    /// Returns: Offset `0..SLOTS` si el id nació de una arena de ese tamaño.
    #[inline(always)]
    pub const fn index(self) -> u16 {
        self.index
    }

    /// Purpose: Expone la generación anti use-after-release.
    /// Inputs: `self` — handle copiado.
    /// Returns: Época vigente cuando se hizo `acquire`.
    #[inline(always)]
    pub const fn generation(self) -> u16 {
        self.generation
    }
}

/// Pool de `SLOTS` buffers de [`PACKET_SIZE`] bytes.
///
/// `SLOTS` debe estar en `1..=u16::MAX` y `SLOTS * PACKET_SIZE` no debe desbordar `usize`.
pub struct PacketArena<const SLOTS: usize> {
    storage: Box<[u8]>,
    lengths: [u16; SLOTS],
    occupied: [bool; SLOTS],
    generations: [u16; SLOTS],
    free: [u16; SLOTS],
    free_top: u16,
}

impl<const SLOTS: usize> PacketArena<SLOTS> {
    /// Purpose: Reserva el backing y deja todos los slots libres.
    /// Inputs: none (`SLOTS` es const generic).
    /// Returns: Arena lista; panics en compile time si `SLOTS` es 0, cabe en `u16` o el producto desborda.
    pub fn new() -> Self {
        const {
            assert!(SLOTS > 0, "PacketArena needs at least one slot");
            assert!(
                SLOTS <= u16::MAX as usize,
                "PacketArena slot count must fit in u16"
            );
            assert!(
                SLOTS.checked_mul(PACKET_SIZE).is_some(),
                "PacketArena backing size overflows usize"
            );
        }

        let mut free = [0u16; SLOTS];
        let mut i = 0;
        while i < SLOTS {
            free[i] = (SLOTS - 1 - i) as u16;
            i += 1;
        }

        Self {
            storage: vec![0u8; SLOTS * PACKET_SIZE].into_boxed_slice(),
            lengths: [0; SLOTS],
            occupied: [false; SLOTS],
            generations: [1; SLOTS],
            free,
            free_top: SLOTS as u16,
        }
    }

    /// Purpose: Número de slots compilados en este tipo.
    /// Inputs: none.
    /// Returns: `SLOTS`.
    #[inline(always)]
    pub const fn slot_count() -> usize {
        SLOTS
    }

    /// Purpose: Capacidad de cada slot en bytes.
    /// Inputs: none.
    /// Returns: [`PACKET_SIZE`].
    #[inline(always)]
    pub const fn packet_capacity() -> usize {
        PACKET_SIZE
    }

    /// Purpose: Cuántos slots se pueden adquirir ahora.
    /// Inputs: none (usa `&self`).
    /// Returns: `0..=SLOTS`.
    #[inline(always)]
    pub fn free_count(&self) -> usize {
        self.free_top as usize
    }

    /// Purpose: Toma un slot libre. No asigna heap.
    /// Inputs: none (`&mut self`).
    /// Returns: `Ok(SlotId)` con `len == 0`; `Err(ArenaExhausted)` si no queda libre.
    #[must_use = "the slot must be released"]
    #[inline(always)]
    pub fn acquire(&mut self) -> Result<SlotId, Error> {
        if self.free_top == 0 {
            return Err(Error::ArenaExhausted);
        }
        self.free_top -= 1;
        let index = self.free[self.free_top as usize];
        let i = index as usize;
        self.occupied[i] = true;
        self.lengths[i] = 0;
        Ok(SlotId {
            index,
            generation: self.generations[i],
        })
    }

    /// Purpose: Devuelve un slot a la free list. No asigna heap.
    /// Inputs: `id` — handle vigente de `acquire`.
    /// Returns: `Ok(())` o `Err(ArenaSlotOutOfRange)` si el id es inválido, stale o ya liberado.
    #[inline(always)]
    pub fn release(&mut self, id: SlotId) -> Result<(), Error> {
        let i = self.index(id)?;
        self.occupied[i] = false;
        self.lengths[i] = 0;
        let next_gen = self.generations[i].wrapping_add(1);
        self.generations[i] = if next_gen == 0 { 1 } else { next_gen };
        self.free[self.free_top as usize] = id.index;
        self.free_top += 1;
        Ok(())
    }

    /// Purpose: Slice de lectura con la longitud comprometida (`0..len`).
    /// Inputs: `id` — slot ocupado.
    /// Returns: Payload real; no incluye los bytes de capacidad sobrante.
    #[inline(always)]
    pub fn slot(&self, id: SlotId) -> Result<&[u8], Error> {
        let i = self.index(id)?;
        let start = i * PACKET_SIZE;
        let len = self.lengths[i] as usize;
        Ok(&self.storage[start..start + len])
    }

    /// Purpose: Slice mutable de **capacidad** completa (`PACKET_SIZE`) para rellenar.
    /// Inputs: `id` — slot ocupado.
    /// Returns: Buffer de escritura; hay que llamar a [`set_len`] después de copiar.
    #[inline(always)]
    pub fn slot_mut(&mut self, id: SlotId) -> Result<&mut [u8], Error> {
        let i = self.index(id)?;
        let start = i * PACKET_SIZE;
        Ok(&mut self.storage[start..start + PACKET_SIZE])
    }

    /// Purpose: Fija cuántos bytes del slot son payload válido.
    /// Inputs: `id` — slot ocupado; `len` — `0..=PACKET_SIZE`.
    /// Returns: `Ok(())`; `Err(ArenaLenOutOfRange)` si `len > PACKET_SIZE`.
    #[inline(always)]
    pub fn set_len(&mut self, id: SlotId, len: usize) -> Result<(), Error> {
        let i = self.index(id)?;
        if len > PACKET_SIZE {
            return Err(Error::ArenaLenOutOfRange);
        }
        self.lengths[i] = len as u16;
        Ok(())
    }

    /// Purpose: Longitud comprometida del payload.
    /// Inputs: `id` — slot ocupado.
    /// Returns: `0..=PACKET_SIZE`.
    #[inline(always)]
    pub fn len(&self, id: SlotId) -> Result<usize, Error> {
        let i = self.index(id)?;
        Ok(self.lengths[i] as usize)
    }

    /// Purpose: Traduce un handle a índice validado.
    /// Inputs: `id` — posiblemente stale o fuera de rango.
    /// Returns: Offset `0..SLOTS` o `ArenaSlotOutOfRange`.
    #[inline(always)]
    fn index(&self, id: SlotId) -> Result<usize, Error> {
        let i = id.index as usize;
        if i >= SLOTS || !self.occupied[i] || self.generations[i] != id.generation {
            return Err(Error::ArenaSlotOutOfRange);
        }
        Ok(i)
    }
}

impl<const SLOTS: usize> Default for PacketArena<SLOTS> {
    /// Purpose: Igual que [`PacketArena::new`].
    /// Inputs: none.
    /// Returns: Arena vacía (todos los slots libres).
    fn default() -> Self {
        Self::new()
    }
}

impl<const SLOTS: usize> fmt::Debug for PacketArena<SLOTS> {
    /// Purpose: Debug sin volcar el backing de 1 MiB.
    /// Inputs: `f` — formatter.
    /// Returns: `Ok` si se escribió `slots` / `free`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PacketArena")
            .field("slots", &SLOTS)
            .field("packet_size", &PACKET_SIZE)
            .field("free", &self.free_count())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::{PacketArena, SlotId, PACKET_SIZE};
    use crate::Error;

    type TinyArena = PacketArena<4>;

    /// Purpose: `new` deja la capacidad completa libre y fija `PACKET_SIZE`.
    /// Inputs: none.
    /// Returns: panics si el inventario inicial es incorrecto.
    #[test]
    fn new_has_all_slots_free() {
        let arena = TinyArena::new();
        assert_eq!(TinyArena::slot_count(), 4);
        assert_eq!(TinyArena::packet_capacity(), PACKET_SIZE);
        assert_eq!(arena.free_count(), 4);
    }

    /// Purpose: El cuarto acquire extra agota la arena.
    /// Inputs: none.
    /// Returns: panics si no aparece `ArenaExhausted`.
    #[test]
    fn acquire_exhausts() {
        let mut arena = TinyArena::new();
        for _ in 0..4 {
            assert!(arena.acquire().is_ok());
        }
        assert_eq!(arena.acquire(), Err(Error::ArenaExhausted));
        assert_eq!(arena.free_count(), 0);
    }

    /// Purpose: `release` reintegra el slot y el siguiente `acquire` reusa el índice.
    /// Inputs: none.
    /// Returns: panics si no se reutiliza el mismo índice.
    #[test]
    fn release_then_acquire_reuses_index() {
        let mut arena = TinyArena::new();
        let first = arena.acquire().expect("slot");
        let idx = first.index();
        arena.release(first).expect("release");
        let again = arena.acquire().expect("reuse");
        assert_eq!(again.index(), idx);
        assert_ne!(again.generation(), first.generation());
        assert_eq!(arena.free_count(), 3);
    }

    /// Purpose: Índice mayor que `SLOTS` no indexa storage.
    /// Inputs: none.
    /// Returns: panics si no es `ArenaSlotOutOfRange`.
    #[test]
    fn slot_index_out_of_range() {
        let mut arena = TinyArena::new();
        let bogus = SlotId::from_raw(99, 1);
        assert_eq!(arena.slot(bogus), Err(Error::ArenaSlotOutOfRange));
        assert_eq!(arena.release(bogus), Err(Error::ArenaSlotOutOfRange));
        assert_eq!(arena.set_len(bogus, 1), Err(Error::ArenaSlotOutOfRange));
    }

    /// Purpose: Tras `release`, el handle viejo (misma generación) ya no vale.
    /// Inputs: none.
    /// Returns: panics si un id stale sigue leyendo el slot.
    #[test]
    fn stale_id_after_release() {
        let mut arena = TinyArena::new();
        let id = arena.acquire().expect("slot");
        arena.release(id).expect("release");
        assert_eq!(arena.slot(id), Err(Error::ArenaSlotOutOfRange));
        assert_eq!(arena.release(id), Err(Error::ArenaSlotOutOfRange));
    }

    /// Purpose: Doble `release` del mismo handle vivo falla a la segunda.
    /// Inputs: none.
    /// Returns: panics si la segunda liberación es `Ok`.
    #[test]
    fn double_release_is_out_of_range() {
        let mut arena = TinyArena::new();
        let id = arena.acquire().expect("slot");
        assert!(arena.release(id).is_ok());
        assert_eq!(arena.release(id), Err(Error::ArenaSlotOutOfRange));
    }

    /// Purpose: `slot` proyecta `len`; `slot_mut` expone la capacidad completa.
    /// Inputs: none.
    /// Returns: panics si len/capacidad se confunden o quedan basura visible.
    #[test]
    fn committed_len_vs_capacity() {
        let mut arena = TinyArena::new();
        let id = arena.acquire().expect("slot");
        assert_eq!(arena.len(id), Ok(0));
        assert_eq!(arena.slot(id).expect("empty").len(), 0);
        {
            let buf = arena.slot_mut(id).expect("cap");
            assert_eq!(buf.len(), PACKET_SIZE);
            buf[0] = 0xAB;
            buf[1] = 0xCD;
            buf[63] = 0xEF;
        }
        arena.set_len(id, 2).expect("len");
        assert_eq!(arena.len(id), Ok(2));
        assert_eq!(arena.slot(id).expect("payload"), &[0xAB, 0xCD]);
    }

    /// Purpose: `set_len` no deja pasar más de `PACKET_SIZE`.
    /// Inputs: none.
    /// Returns: panics si no es `ArenaLenOutOfRange`.
    #[test]
    fn set_len_overflow() {
        let mut arena = TinyArena::new();
        let id = arena.acquire().expect("slot");
        assert_eq!(
            arena.set_len(id, PACKET_SIZE + 1),
            Err(Error::ArenaLenOutOfRange)
        );
        assert!(arena.set_len(id, PACKET_SIZE).is_ok());
        assert_eq!(arena.len(id), Ok(PACKET_SIZE));
    }

    /// Purpose: Un slot reusado no publica bytes del inquilino anterior.
    /// Inputs: none.
    /// Returns: panics si el `len` viejo sobrevive al segundo `acquire`.
    #[test]
    fn reuse_resets_len_not_capacity() {
        let mut arena = TinyArena::new();
        let id = arena.acquire().expect("slot");
        {
            let buf = arena.slot_mut(id).expect("cap");
            buf[0] = 1;
        }
        arena.set_len(id, 1).expect("len");
        arena.release(id).expect("release");
        let id = arena.acquire().expect("reuse");
        assert_eq!(arena.len(id), Ok(0));
        assert_eq!(arena.slot(id).expect("hidden").len(), 0);
        assert_eq!(arena.slot_mut(id).expect("cap").len(), PACKET_SIZE);
    }

    /// Purpose: `Debug` no recorre el backing.
    /// Inputs: none.
    /// Returns: panics si el formato no incluye `free`.
    #[test]
    fn debug_is_short() {
        let arena = TinyArena::new();
        let text = format!("{arena:?}");
        assert!(text.contains("free"));
        assert!(text.contains('4'));
    }
}
