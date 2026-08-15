//! Encoder/decoder Reed-Solomon (SIMD). Los shards son slices de longitud fija.
//!
//! [`FecEngine::new`] reserva el workspace de `reed-solomon-simd`.
//! [`FecEngine::encode`] / [`FecEngine::decode`] reutilizan ese workspace y
//! copian el resultado a buffers del caller (arena o arrays de test).

use crate::Error;
use core::fmt;
use reed_solomon_simd::{ReedSolomonDecoder, ReedSolomonEncoder};

/// Tamaño de shard por defecto: par, múltiplo de 64 (SIMD) y cabe en un payload de shred.
pub const DEFAULT_SHARD_BYTES: usize = 64;

/// Engine reutilizable: un encoder y un decoder con la misma configuración FEC.
pub struct FecEngine {
    encoder: ReedSolomonEncoder,
    decoder: ReedSolomonDecoder,
    original_count: usize,
    recovery_count: usize,
    shard_bytes: usize,
}

impl FecEngine {
    /// Purpose: Reserva workspace SIMD para un set FEC (`k` originales, `n` recovery).
    /// Inputs: `original_count` — data shreds; `recovery_count` — code shreds;
    ///   `shard_bytes` — longitud par de cada shard (payload, no el paquete UDP).
    /// Returns: Engine listo, o `FecInconsistent` si el crate no soporta la config.
    pub fn new(
        original_count: usize,
        recovery_count: usize,
        shard_bytes: usize,
    ) -> Result<Self, Error> {
        validate_config(original_count, recovery_count, shard_bytes)?;
        let encoder = ReedSolomonEncoder::new(original_count, recovery_count, shard_bytes)
            .map_err(map_simd)?;
        let decoder = ReedSolomonDecoder::new(original_count, recovery_count, shard_bytes)
            .map_err(map_simd)?;
        Ok(Self {
            encoder,
            decoder,
            original_count,
            recovery_count,
            shard_bytes,
        })
    }

    /// Purpose: `k` del código RS (data shards).
    /// Inputs: none.
    /// Returns: valor fijado en [`new`](Self::new).
    #[inline(always)]
    pub const fn original_count(&self) -> usize {
        self.original_count
    }

    /// Purpose: `n` del código RS (recovery shards).
    /// Inputs: none.
    /// Returns: valor fijado en [`new`](Self::new).
    #[inline(always)]
    pub const fn recovery_count(&self) -> usize {
        self.recovery_count
    }

    /// Purpose: Bytes por shard.
    /// Inputs: none.
    /// Returns: longitud par usada por el workspace.
    #[inline(always)]
    pub const fn shard_bytes(&self) -> usize {
        self.shard_bytes
    }

    /// Purpose: Genera shards de paridad a partir de todos los data shards.
    /// Inputs: `original` — exactamente `original_count` slices de `shard_bytes`;
    ///   `recovery_out` — exactamente `recovery_count` destinos de `shard_bytes`.
    /// Returns: `Ok(())` con `recovery_out` rellenados; no asigna heap por shard.
    pub fn encode(
        &mut self,
        original: &[&[u8]],
        recovery_out: &mut [&mut [u8]],
    ) -> Result<(), Error> {
        if original.len() != self.original_count || recovery_out.len() != self.recovery_count {
            return Err(Error::FecInconsistent);
        }
        for shard in original {
            check_shard_len(shard.len(), self.shard_bytes)?;
        }
        for dest in recovery_out.iter() {
            check_shard_len(dest.len(), self.shard_bytes)?;
        }

        self.encoder
            .reset(self.original_count, self.recovery_count, self.shard_bytes)
            .map_err(map_simd)?;
        for shard in original {
            self.encoder.add_original_shard(shard).map_err(map_simd)?;
        }
        let result = self.encoder.encode().map_err(map_simd)?;
        for (i, dest) in recovery_out.iter_mut().enumerate() {
            let src = result.recovery(i).ok_or(Error::FecInconsistent)?;
            dest.copy_from_slice(src);
        }
        Ok(())
    }

    /// Purpose: Reconstruye data shards faltantes (`None`) usando recovery.
    /// Inputs: `original` — `Some` si el data shred llegó, `None` si es erasure;
    ///   `recovery` — igual para code shreds; `restored` — destinos, uno por data
    ///   shard (solo se escriben los índices `None`).
    /// Returns: cuántos originales se restauraron, o `FecTooManyErasures`.
    pub fn decode(
        &mut self,
        original: &[Option<&[u8]>],
        recovery: &[Option<&[u8]>],
        restored: &mut [&mut [u8]],
    ) -> Result<usize, Error> {
        if original.len() != self.original_count
            || recovery.len() != self.recovery_count
            || restored.len() != self.original_count
        {
            return Err(Error::FecInconsistent);
        }

        let mut present = 0usize;
        let mut missing = 0usize;
        for (i, shard) in original.iter().enumerate() {
            match shard {
                Some(bytes) => {
                    check_shard_len(bytes.len(), self.shard_bytes)?;
                    present += 1;
                }
                None => {
                    check_shard_len(restored[i].len(), self.shard_bytes)?;
                    missing += 1;
                }
            }
        }
        for bytes in recovery.iter().flatten() {
            check_shard_len(bytes.len(), self.shard_bytes)?;
            present += 1;
        }

        if missing == 0 {
            return Ok(0);
        }
        if present < self.original_count {
            return Err(Error::FecTooManyErasures);
        }

        self.decoder
            .reset(self.original_count, self.recovery_count, self.shard_bytes)
            .map_err(map_simd)?;
        for (i, shard) in original.iter().enumerate() {
            if let Some(bytes) = shard {
                self.decoder
                    .add_original_shard(i, bytes)
                    .map_err(map_simd)?;
            }
        }
        for (i, shard) in recovery.iter().enumerate() {
            if let Some(bytes) = shard {
                self.decoder
                    .add_recovery_shard(i, bytes)
                    .map_err(map_simd)?;
            }
        }

        let result = self.decoder.decode().map_err(map_simd)?;
        let mut restored_count = 0usize;
        for (i, slot) in original.iter().enumerate() {
            if slot.is_some() {
                continue;
            }
            let src = result.restored_original(i).ok_or(Error::FecInconsistent)?;
            restored[i].copy_from_slice(src);
            restored_count += 1;
        }
        Ok(restored_count)
    }
}

/// Purpose: Rechaza k/n/tamaño que el crate SIMD no puede usar.
/// Inputs: conteos y `shard_bytes`.
/// Returns: `Ok(())` o `FecInconsistent`.
#[inline(always)]
fn validate_config(
    original_count: usize,
    recovery_count: usize,
    shard_bytes: usize,
) -> Result<(), Error> {
    if original_count == 0 || recovery_count == 0 {
        return Err(Error::FecInconsistent);
    }
    if shard_bytes == 0 || !shard_bytes.is_multiple_of(2) {
        return Err(Error::FecInconsistent);
    }
    if !ReedSolomonEncoder::supports(original_count, recovery_count) {
        return Err(Error::FecInconsistent);
    }
    Ok(())
}

/// Purpose: Todos los shards de un round deben medir exactamente `shard_bytes`.
/// Inputs: `got` — longitud vista; `want` — configurada.
/// Returns: `Ok(())` o `FecInconsistent`.
#[inline(always)]
fn check_shard_len(got: usize, want: usize) -> Result<(), Error> {
    if got != want {
        return Err(Error::FecInconsistent);
    }
    Ok(())
}

/// Purpose: Compacta errores de `reed-solomon-simd` a las dos variantes del crate.
/// Inputs: `err` — error Copy del crate SIMD.
/// Returns: `FecTooManyErasures` o `FecInconsistent` (sin heap).
fn map_simd(err: reed_solomon_simd::Error) -> Error {
    match err {
        reed_solomon_simd::Error::NotEnoughShards { .. }
        | reed_solomon_simd::Error::TooFewOriginalShards { .. } => Error::FecTooManyErasures,
        _ => Error::FecInconsistent,
    }
}

impl fmt::Debug for FecEngine {
    /// Purpose: Debug de la config, sin volcar el workspace SIMD.
    /// Inputs: `f` — formatter.
    /// Returns: `fmt::Result`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FecEngine")
            .field("original_count", &self.original_count)
            .field("recovery_count", &self.recovery_count)
            .field("shard_bytes", &self.shard_bytes)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::{FecEngine, DEFAULT_SHARD_BYTES};
    use crate::Error;

    /// Purpose: Rellena `dest` con un patrón determinista distinto por índice.
    /// Inputs: `dest` — shard; `tag` — byte base.
    /// Returns: none.
    fn fill(dest: &mut [u8], tag: u8) {
        for (i, b) in dest.iter_mut().enumerate() {
            *b = tag.wrapping_add(i as u8);
        }
    }

    /// Purpose: Round-trip sin pérdidas: encode no corrompe los originales.
    /// Inputs: none.
    /// Returns: panics si la paridad no permite decode trivial (0 restaurados).
    #[test]
    fn encode_round_trip_no_loss() {
        let mut engine = FecEngine::new(3, 2, DEFAULT_SHARD_BYTES).expect("engine");
        let mut d0 = [0u8; DEFAULT_SHARD_BYTES];
        let mut d1 = [0u8; DEFAULT_SHARD_BYTES];
        let mut d2 = [0u8; DEFAULT_SHARD_BYTES];
        fill(&mut d0, 1);
        fill(&mut d1, 2);
        fill(&mut d2, 3);
        let mut r0 = [0u8; DEFAULT_SHARD_BYTES];
        let mut r1 = [0u8; DEFAULT_SHARD_BYTES];
        engine
            .encode(&[&d0, &d1, &d2], &mut [&mut r0, &mut r1])
            .expect("encode");

        let mut unused0 = [0u8; DEFAULT_SHARD_BYTES];
        let mut unused1 = [0u8; DEFAULT_SHARD_BYTES];
        let mut unused2 = [0u8; DEFAULT_SHARD_BYTES];
        let n = engine
            .decode(
                &[
                    Some(d0.as_slice()),
                    Some(d1.as_slice()),
                    Some(d2.as_slice()),
                ],
                &[Some(r0.as_slice()), Some(r1.as_slice())],
                &mut [&mut unused0, &mut unused1, &mut unused2],
            )
            .expect("decode");
        assert_eq!(n, 0);
    }

    /// Purpose: Un data shred erased se reconstruye con recovery.
    /// Inputs: none.
    /// Returns: panics si el shard restaurado no coincide.
    #[test]
    fn reconstruct_one_erasure() {
        let mut engine = FecEngine::new(3, 2, DEFAULT_SHARD_BYTES).expect("engine");
        let mut d0 = [0u8; DEFAULT_SHARD_BYTES];
        let mut d1 = [0u8; DEFAULT_SHARD_BYTES];
        let mut d2 = [0u8; DEFAULT_SHARD_BYTES];
        fill(&mut d0, 10);
        fill(&mut d1, 20);
        fill(&mut d2, 30);
        let mut r0 = [0u8; DEFAULT_SHARD_BYTES];
        let mut r1 = [0u8; DEFAULT_SHARD_BYTES];
        engine
            .encode(&[&d0, &d1, &d2], &mut [&mut r0, &mut r1])
            .expect("encode");

        let mut restored0 = [0u8; DEFAULT_SHARD_BYTES];
        let mut skip1 = [0u8; DEFAULT_SHARD_BYTES];
        let mut skip2 = [0u8; DEFAULT_SHARD_BYTES];
        let n = engine
            .decode(
                &[None, Some(d1.as_slice()), Some(d2.as_slice())],
                &[Some(r0.as_slice()), Some(r1.as_slice())],
                &mut [&mut restored0, &mut skip1, &mut skip2],
            )
            .expect("decode");
        assert_eq!(n, 1);
        assert_eq!(restored0, d0);
    }

    /// Purpose: 2 erasures con 2 recovery (capacidad justita) reconstruye ambos.
    /// Inputs: none.
    /// Returns: panics si algún original no vuelve.
    #[test]
    fn reconstruct_two_erasures() {
        let mut engine = FecEngine::new(3, 2, DEFAULT_SHARD_BYTES).expect("engine");
        let mut d0 = [0u8; DEFAULT_SHARD_BYTES];
        let mut d1 = [0u8; DEFAULT_SHARD_BYTES];
        let mut d2 = [0u8; DEFAULT_SHARD_BYTES];
        fill(&mut d0, 40);
        fill(&mut d1, 50);
        fill(&mut d2, 60);
        let mut r0 = [0u8; DEFAULT_SHARD_BYTES];
        let mut r1 = [0u8; DEFAULT_SHARD_BYTES];
        engine
            .encode(&[&d0, &d1, &d2], &mut [&mut r0, &mut r1])
            .expect("encode");

        let mut restored0 = [0u8; DEFAULT_SHARD_BYTES];
        let mut restored1 = [0u8; DEFAULT_SHARD_BYTES];
        let mut skip2 = [0u8; DEFAULT_SHARD_BYTES];
        let n = engine
            .decode(
                &[None, None, Some(d2.as_slice())],
                &[Some(r0.as_slice()), Some(r1.as_slice())],
                &mut [&mut restored0, &mut restored1, &mut skip2],
            )
            .expect("decode");
        assert_eq!(n, 2);
        assert_eq!(restored0, d0);
        assert_eq!(restored1, d1);
    }

    /// Purpose: 3 erasures con solo 2 recovery supera la distancia del código.
    /// Inputs: none.
    /// Returns: panics si no es `FecTooManyErasures`.
    #[test]
    fn too_many_erasures() {
        let mut engine = FecEngine::new(3, 2, DEFAULT_SHARD_BYTES).expect("engine");
        let mut d0 = [0u8; DEFAULT_SHARD_BYTES];
        let mut d1 = [0u8; DEFAULT_SHARD_BYTES];
        let mut d2 = [0u8; DEFAULT_SHARD_BYTES];
        fill(&mut d0, 1);
        fill(&mut d1, 2);
        fill(&mut d2, 3);
        let mut r0 = [0u8; DEFAULT_SHARD_BYTES];
        let mut r1 = [0u8; DEFAULT_SHARD_BYTES];
        engine
            .encode(&[&d0, &d1, &d2], &mut [&mut r0, &mut r1])
            .expect("encode");

        let mut a = [0u8; DEFAULT_SHARD_BYTES];
        let mut b = [0u8; DEFAULT_SHARD_BYTES];
        let mut c = [0u8; DEFAULT_SHARD_BYTES];
        let err = engine.decode(
            &[None, None, None],
            &[Some(r0.as_slice()), Some(r1.as_slice())],
            &mut [&mut a, &mut b, &mut c],
        );
        assert_eq!(err, Err(Error::FecTooManyErasures));
    }

    /// Purpose: Shards de longitudes distintas no forman un set FEC.
    /// Inputs: none.
    /// Returns: panics si no es `FecInconsistent`.
    #[test]
    fn mismatched_shard_len_is_inconsistent() {
        let mut engine = FecEngine::new(2, 1, DEFAULT_SHARD_BYTES).expect("engine");
        let d0 = [1u8; DEFAULT_SHARD_BYTES];
        let d1 = [2u8; 32];
        let mut r0 = [0u8; DEFAULT_SHARD_BYTES];
        let err = engine.encode(&[&d0, &d1], &mut [&mut r0]);
        assert_eq!(err, Err(Error::FecInconsistent));
    }

    /// Purpose: `shard_bytes` impar lo rechaza el crate (y nuestra validación).
    /// Inputs: none.
    /// Returns: panics si `new` no falla.
    #[test]
    fn odd_shard_bytes_rejected() {
        assert_eq!(FecEngine::new(2, 1, 63).err(), Some(Error::FecInconsistent));
        assert_eq!(FecEngine::new(0, 1, 64).err(), Some(Error::FecInconsistent));
    }

    /// Purpose: El engine se puede reutilizar (segundo encode no exige realloc visible).
    /// Inputs: none.
    /// Returns: panics si el segundo round no reconstruye.
    #[test]
    fn engine_reuse_second_round() {
        let mut engine = FecEngine::new(2, 1, DEFAULT_SHARD_BYTES).expect("engine");
        for tag in [7u8, 9u8] {
            let mut d0 = [0u8; DEFAULT_SHARD_BYTES];
            let mut d1 = [0u8; DEFAULT_SHARD_BYTES];
            fill(&mut d0, tag);
            fill(&mut d1, tag.wrapping_add(1));
            let mut r0 = [0u8; DEFAULT_SHARD_BYTES];
            engine.encode(&[&d0, &d1], &mut [&mut r0]).expect("encode");
            let mut restored = [0u8; DEFAULT_SHARD_BYTES];
            let mut skip = [0u8; DEFAULT_SHARD_BYTES];
            let n = engine
                .decode(
                    &[None, Some(d1.as_slice())],
                    &[Some(r0.as_slice())],
                    &mut [&mut restored, &mut skip],
                )
                .expect("decode");
            assert_eq!(n, 1);
            assert_eq!(restored, d0);
        }
    }
}
