//! Layout binario de shreds (data vs code) y parseo zero-copy.
//!
//! Fase 1: solo el stub de [`parse`]. El wire format llega en la fase 3.

use crate::Error;

/// Purpose: Interpreta bytes de un slot de arena como shred (stub).
/// Inputs: `bytes` — vista sobre un slot; no se clona ni se mueve a un `Vec`.
/// Returns: `Err(Error::Unimplemented)` hasta la fase 3.
pub fn parse(_bytes: &[u8]) -> Result<(), Error> {
    Err(Error::Unimplemented { module: "shred" })
}

#[cfg(test)]
mod tests {
    use super::parse;
    use crate::Error;

    /// Purpose: El stub de parseo no implementa lógica todavía.
    /// Inputs: none.
    /// Returns: panics si el error no es `Unimplemented { module: "shred" }`.
    #[test]
    fn parse_is_unimplemented() {
        assert_eq!(parse(&[]), Err(Error::Unimplemented { module: "shred" }));
    }
}
