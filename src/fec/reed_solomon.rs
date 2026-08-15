//! Decoder/encoder Reed-Solomon acelerado por SIMD.
//!
//! Fase 1: stubs de [`encode`] y [`decode`]. La crate `reed-solomon-simd` llega en la fase 4.

use crate::Error;

/// Purpose: Genera code shreds a partir de data shreds (stub).
/// Inputs: none; la firma real se fija en la fase 4 junto a la arena.
/// Returns: `Err(Error::Unimplemented)` hasta la fase 4.
pub fn encode() -> Result<(), Error> {
    Err(Error::Unimplemented { module: "fec" })
}

/// Purpose: Reconstruye data shreds faltantes (stub).
/// Inputs: none; la firma real se fija en la fase 4.
/// Returns: `Err(Error::Unimplemented)` hasta la fase 4.
pub fn decode() -> Result<(), Error> {
    Err(Error::Unimplemented { module: "fec" })
}

#[cfg(test)]
mod tests {
    use super::{decode, encode};
    use crate::Error;

    /// Purpose: Ambos stubs FEC reportan el mismo módulo.
    /// Inputs: none.
    /// Returns: panics si no devuelven `Unimplemented { module: "fec" }`.
    #[test]
    fn encode_and_decode_are_unimplemented() {
        let expected = Err(Error::Unimplemented { module: "fec" });
        assert_eq!(encode(), expected);
        assert_eq!(decode(), expected);
    }
}
