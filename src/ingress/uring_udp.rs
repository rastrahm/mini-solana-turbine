//! Receptor UDP sobre `io_uring`.
//!
//! Debe ejecutarse dentro de [`tokio_uring::start`]. El loop de ingestión es:
//!
//! 1. [`PacketArena::acquire`] — un slot libre, sin heap.
//! 2. [`UdpIngress::recv_into`] — el kernel escribe en ese slot (`IORING_OP_RECVMSG`).
//! 3. Entregar el [`SlotId`] al pipeline (parse/FEC). No clonar el payload.
//! 4. [`PacketArena::release`] cuando el shred ya no hace falta.

use crate::arena::{PacketArena, SlotId, PACKET_SIZE};
use crate::ingress::RecvDatagram;
use crate::Error;
use std::net::SocketAddr;
use tokio_uring::buf::{IoBuf, IoBufMut};
use tokio_uring::net::UdpSocket;

/// Socket de ingress. No es `Copy`: posee el fd uring.
pub struct UdpIngress {
    socket: UdpSocket,
}

/// Buffer `IoBufMut` que apunta a un slot. El puntero no se mueve al mover este struct.
struct RecvSlot {
    ptr: *mut u8,
    cap: usize,
    init: usize,
    slot: SlotId,
}

// SAFETY: el runtime uring es de un hilo; el puntero solo se usa mientras `PacketArena`
// vive y el slot sigue ocupado. No hay alias `&mut [u8]` concurrente sobre ese rango.
unsafe impl Send for RecvSlot {}

/// Buffer `IoBuf` de envío: mismos bytes del slot, solo lectura.
struct SendSlot {
    ptr: *const u8,
    len: usize,
}

// SAFETY: igual que `RecvSlot`: el slot permanece ocupado y `arena` no se muta
// durante el await de `send_to`.
unsafe impl Send for SendSlot {}

impl UdpIngress {
    /// Purpose: Interpreta un bind addr (`host:port`) sin I/O.
    /// Inputs: `addr` — p. ej. `127.0.0.1:0`.
    /// Returns: `SocketAddr` o `IngressBind` si el parseo falla.
    pub fn parse_addr(addr: &str) -> Result<SocketAddr, Error> {
        crate::ingress::parse_addr(addr)
    }

    /// Purpose: Bind UDP vía `io_uring`.
    /// Inputs: `addr` — texto `ip:puerto` (`:0` pide puerto efímero).
    /// Returns: socket bound, o `IngressBind`.
    pub async fn bind(addr: &str) -> Result<Self, Error> {
        Self::bind_addr(Self::parse_addr(addr)?).await
    }

    /// Purpose: Bind UDP a un [`SocketAddr`] ya parseado.
    /// Inputs: `addr` — dirección local.
    /// Returns: socket bound, o `IngressBind`.
    pub async fn bind_addr(addr: SocketAddr) -> Result<Self, Error> {
        let socket = UdpSocket::bind(addr)
            .await
            .map_err(|_| Error::IngressBind)?;
        Ok(Self { socket })
    }

    /// Purpose: Dirección local real (útil tras bind a puerto 0).
    /// Inputs: none (`&self`).
    /// Returns: `SocketAddr` o `IngressBind` si el kernel no informa el puerto.
    pub fn local_addr(&self) -> Result<SocketAddr, Error> {
        self.socket.local_addr().map_err(|_| Error::IngressBind)
    }

    /// Purpose: Recibe un datagrama en un slot libre. Sin `Vec` de payload.
    /// Inputs: `arena` — pool de slots; se adquiere uno y se rellena.
    /// Returns: [`RecvDatagram`]; `ArenaExhausted`, `IngressRecv` o errores de slot.
    ///
    /// Si el recv falla, el slot se libera.
    pub async fn recv_into<const SLOTS: usize>(
        &self,
        arena: &mut PacketArena<SLOTS>,
    ) -> Result<RecvDatagram, Error> {
        let slot = arena.acquire()?;
        let ptr = match arena.slot_mut_ptr(slot) {
            Ok(ptr) => ptr,
            Err(err) => {
                let _ = arena.release(slot);
                return Err(err);
            }
        };
        let buf = RecvSlot {
            ptr,
            cap: PACKET_SIZE,
            init: 0,
            slot,
        };
        let (res, buf) = self.socket.recv_from(buf).await;
        match res {
            Ok((n, src)) => match arena.set_len(buf.slot, n) {
                Ok(()) => Ok(RecvDatagram {
                    slot: buf.slot,
                    src,
                    len: n,
                }),
                Err(err) => {
                    let _ = arena.release(buf.slot);
                    Err(err)
                }
            },
            Err(_) => {
                let _ = arena.release(buf.slot);
                Err(Error::IngressRecv)
            }
        }
    }

    /// Purpose: Envía los bytes comprometidos del slot a `dest` (sin `Vec` ni clone).
    /// Inputs: `arena` — debe vivir hasta que termine el await; `slot` — handle
    ///   con `len` fijado; `dest` — UDP destino.
    /// Returns: bytes enviados, o `Arena*` / [`Error::IngressSend`].
    pub async fn send_slot<const SLOTS: usize>(
        &self,
        arena: &PacketArena<SLOTS>,
        slot: SlotId,
        dest: SocketAddr,
    ) -> Result<usize, Error> {
        let bytes = arena.slot(slot)?;
        let buf = SendSlot {
            ptr: bytes.as_ptr(),
            len: bytes.len(),
        };
        let (res, _) = self.socket.send_to(buf, dest).await;
        res.map_err(|_| Error::IngressSend)
    }

    /// Purpose: Reenvía el mismo slot a cada addr de un [`crate::ForwardPlan`].
    /// Inputs: `arena` / `slot` — payload ya ingerido; `dests` — salida de
    ///   [`crate::Pipeline::dest_addrs`].
    /// Returns: datagramas enviados (`dests.len()` si todos ok), o el primer
    ///   `IngressSend`. Un plan vacío no hace I/O.
    pub async fn forward_slot<const SLOTS: usize>(
        &self,
        arena: &PacketArena<SLOTS>,
        slot: SlotId,
        dests: &[SocketAddr],
    ) -> Result<usize, Error> {
        let mut sent = 0usize;
        for dest in dests {
            let _n = self.send_slot(arena, slot, *dest).await?;
            sent += 1;
        }
        Ok(sent)
    }
}

// SAFETY: `ptr` es el backing heap de `PacketArena` (Box), estable si el struct
// RecvSlot se mueve. `cap` es PACKET_SIZE. Solo se llama `set_init` con bytes
// que el kernel acaba de escribir.
unsafe impl IoBuf for RecvSlot {
    /// Purpose: Puntero estable para el SQE de recv.
    /// Inputs: none (`&self`).
    /// Returns: inicio del slot.
    fn stable_ptr(&self) -> *const u8 {
        self.ptr.cast_const()
    }

    /// Purpose: Bytes ya inicializados (0 hasta que el kernel llene).
    /// Inputs: none.
    /// Returns: `init`.
    fn bytes_init(&self) -> usize {
        self.init
    }

    /// Purpose: Capacidad presentada a `recv`.
    /// Inputs: none.
    /// Returns: [`PACKET_SIZE`].
    fn bytes_total(&self) -> usize {
        self.cap
    }
}

// SAFETY: misma región que `IoBuf`; `set_init` solo después del CQE.
unsafe impl IoBufMut for RecvSlot {
    /// Purpose: Puntero mutable para el kernel.
    /// Inputs: none (`&mut self`).
    /// Returns: inicio del slot.
    fn stable_mut_ptr(&mut self) -> *mut u8 {
        self.ptr
    }

    /// Purpose: Marca los `pos` primeros bytes como escritos por el kernel.
    /// Inputs: `pos` — bytes del CQE (`<= cap`).
    /// Returns: none.
    unsafe fn set_init(&mut self, pos: usize) {
        self.init = pos;
    }
}

// SAFETY: `ptr` apunta al backing de `PacketArena` mientras el slot está ocupado;
// `len` es `arena.len(slot)`. El kernel solo lee esos bytes en `send_to`.
unsafe impl IoBuf for SendSlot {
    /// Purpose: Puntero estable para el SQE de send.
    /// Inputs: none (`&self`).
    /// Returns: inicio del payload del slot.
    #[inline(always)]
    fn stable_ptr(&self) -> *const u8 {
        self.ptr
    }

    /// Purpose: Bytes a transmitir (el `len` comprometido).
    /// Inputs: none.
    /// Returns: `len`.
    #[inline(always)]
    fn bytes_init(&self) -> usize {
        self.len
    }

    /// Purpose: Capacidad presentada a `send_to` (solo el payload válido).
    /// Inputs: none.
    /// Returns: `len`.
    #[inline(always)]
    fn bytes_total(&self) -> usize {
        self.len
    }
}

#[cfg(test)]
mod tests {
    use super::UdpIngress;
    use crate::arena::PacketArena;
    use crate::Error;
    use std::net::UdpSocket;

    /// Purpose: Un addr que no es `host:port` no abre socket.
    /// Inputs: none.
    /// Returns: panics si no es `IngressBind`.
    #[test]
    fn parse_addr_rejects_garbage() {
        assert_eq!(UdpIngress::parse_addr("nope"), Err(Error::IngressBind));
    }

    /// Purpose: Loopback: send std → recv uring en un slot de arena.
    /// Inputs: none.
    /// Returns: panics si los bytes del slot no coinciden con el datagrama.
    #[cfg(target_os = "linux")]
    #[test]
    fn recv_loopback_fills_slot() {
        let result = tokio_uring::start(async { recv_loopback_body().await });
        result.expect("loopback recv");
    }

    /// Purpose: Cuerpo async del test de loopback (runtime uring).
    /// Inputs: none.
    /// Returns: `Ok` si el slot contiene `hello-uring`.
    #[cfg(target_os = "linux")]
    async fn recv_loopback_body() -> Result<(), Error> {
        let ingress = UdpIngress::bind("127.0.0.1:0").await?;
        let dest = ingress.local_addr()?;
        let sender = UdpSocket::bind("127.0.0.1:0").map_err(|_| Error::IngressBind)?;
        let payload = b"hello-uring";
        sender
            .send_to(payload, dest)
            .map_err(|_| Error::IngressRecv)?;

        let mut arena = PacketArena::<4>::new();
        let recvd = ingress.recv_into(&mut arena).await?;
        assert_eq!(recvd.len, payload.len());
        assert_eq!(
            arena.slot(recvd.slot).map_err(|_| Error::IngressRecv)?,
            payload
        );
        let _ = arena.release(recvd.slot);
        Ok(())
    }

    /// Purpose: Loopback: send uring desde un slot → recv std, mismos bytes.
    /// Inputs: none.
    /// Returns: panics si el datagrama no coincide con el slot.
    #[cfg(target_os = "linux")]
    #[test]
    fn send_slot_loopback_reuses_arena() {
        let result = tokio_uring::start(async { send_slot_loopback_body().await });
        result.expect("loopback send");
    }

    /// Purpose: Cuerpo async del send loopback.
    /// Inputs: none.
    /// Returns: `Ok` si std recibe `hello-send`.
    #[cfg(target_os = "linux")]
    async fn send_slot_loopback_body() -> Result<(), Error> {
        use std::time::Duration;
        let receiver = UdpSocket::bind("127.0.0.1:0").map_err(|_| Error::IngressBind)?;
        receiver
            .set_read_timeout(Some(Duration::from_secs(2)))
            .map_err(|_| Error::IngressBind)?;
        let dest = receiver.local_addr().map_err(|_| Error::IngressBind)?;
        let ingress = UdpIngress::bind("127.0.0.1:0").await?;

        let payload = b"hello-send";
        let mut arena = PacketArena::<4>::new();
        let slot = arena.acquire()?;
        {
            let buf = arena.slot_mut(slot)?;
            buf[..payload.len()].copy_from_slice(payload);
        }
        arena.set_len(slot, payload.len())?;

        let n = ingress.send_slot(&arena, slot, dest).await?;
        assert_eq!(n, payload.len());

        let mut got = [0u8; 32];
        let (got_n, _) = receiver
            .recv_from(&mut got)
            .map_err(|_| Error::IngressRecv)?;
        assert_eq!(&got[..got_n], payload);
        let _ = arena.release(slot);
        Ok(())
    }

    /// Purpose: `forward_slot` con destinos vacíos no envía.
    /// Inputs: none.
    /// Returns: panics si el conteo no es 0.
    #[cfg(target_os = "linux")]
    #[test]
    fn forward_slot_empty_is_noop() {
        let result = tokio_uring::start(async { forward_empty_body().await });
        result.expect("empty forward");
    }

    /// Purpose: Cuerpo async de forward vacío.
    /// Inputs: none.
    /// Returns: `Ok` si `forward_slot` da 0.
    #[cfg(target_os = "linux")]
    async fn forward_empty_body() -> Result<(), Error> {
        let ingress = UdpIngress::bind("127.0.0.1:0").await?;
        let mut arena = PacketArena::<1>::new();
        let slot = arena.acquire()?;
        arena.set_len(slot, 0)?;
        let sent = ingress.forward_slot(&arena, slot, &[]).await?;
        assert_eq!(sent, 0);
        let _ = arena.release(slot);
        Ok(())
    }
}
