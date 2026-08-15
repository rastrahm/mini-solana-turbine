//! Errores zero-cost del crate (`thiserror`, sin `String` ni `anyhow`).
//!
//! [`Error::ArenaExhausted`], [`Error::ArenaSlotOutOfRange`] y
//! [`Error::ArenaLenOutOfRange`] están vivos desde la fase 2. El resto son
//! variantes placeholder para fases posteriores; los stubs siguen devolviendo
//! [`Error::Unimplemented`].

use thiserror::Error;

/// Fallos recuperables del pipeline (parseo, arena, FEC, red, routing).
///
/// Tipo `Copy` y sin heap: los mensajes son `&'static str` o unit variants.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// El módulo todavía es un stub de fase 1.
    #[error("not implemented: {module}")]
    Unimplemented {
        /// Nombre del módulo stub (`shred`, `ingress`, `fec`, `turbine`).
        module: &'static str,
    },

    /// El slice no cubre el header de shred (fase 3).
    #[error("shred header is truncated")]
    ShredTruncated,

    /// El flag data/code del shred no es válido (fase 3).
    #[error("shred type flag is invalid")]
    ShredInvalidType,

    /// La arena no tiene slots libres (fase 2).
    #[error("packet arena has no free slots")]
    ArenaExhausted,

    /// El índice de slot no pertenece a la arena (fase 2).
    #[error("slot index is out of range")]
    ArenaSlotOutOfRange,

    /// `set_len` pidió más bytes que `PACKET_SIZE` (fase 2).
    #[error("packet length exceeds slot capacity")]
    ArenaLenOutOfRange,

    /// Hay más erasures que paridad disponible (fase 4).
    #[error("fec: too many erasures to reconstruct")]
    FecTooManyErasures,

    /// El conjunto de shards FEC es inconsistente (fase 4).
    #[error("fec: shard set is inconsistent")]
    FecInconsistent,

    /// Falló el bind del socket UDP (fase 5).
    #[error("UDP ingress failed to bind")]
    IngressBind,

    /// Falló un recv UDP (fase 5).
    #[error("UDP ingress receive failed")]
    IngressRecv,

    /// El node id no está en el cluster (fase 6).
    #[error("turbine: unknown node")]
    TurbineUnknownNode,

    /// El cluster no tiene nodos (fase 6).
    #[error("turbine: empty cluster")]
    TurbineEmptyCluster,
}

#[cfg(test)]
mod tests {
    use super::Error;

    /// Purpose: Comprueba que `Error` cumple `std::error::Error`.
    /// Inputs: none.
    /// Returns: panics si el bound no se satisface (no compilaría).
    #[test]
    fn implements_std_error() {
        fn assert_is_error<E: std::error::Error>() {}
        assert_is_error::<Error>();
    }

    /// Purpose: Comprueba que el enum es `Copy` (cero heap al propagar).
    /// Inputs: none.
    /// Returns: panics si la copia no preserva el valor.
    #[test]
    fn is_copy() {
        let err = Error::ArenaExhausted;
        let cloned = err;
        assert_eq!(err, cloned);
    }

    /// Purpose: Fija el texto de `Display` de cada variante placeholder.
    /// Inputs: none.
    /// Returns: panics si un mensaje cambia sin actualizar el test.
    #[test]
    fn display_messages_are_stable() {
        let cases = [
            (
                Error::Unimplemented { module: "shred" },
                "not implemented: shred",
            ),
            (Error::ShredTruncated, "shred header is truncated"),
            (Error::ShredInvalidType, "shred type flag is invalid"),
            (Error::ArenaExhausted, "packet arena has no free slots"),
            (Error::ArenaSlotOutOfRange, "slot index is out of range"),
            (
                Error::ArenaLenOutOfRange,
                "packet length exceeds slot capacity",
            ),
            (
                Error::FecTooManyErasures,
                "fec: too many erasures to reconstruct",
            ),
            (Error::FecInconsistent, "fec: shard set is inconsistent"),
            (Error::IngressBind, "UDP ingress failed to bind"),
            (Error::IngressRecv, "UDP ingress receive failed"),
            (Error::TurbineUnknownNode, "turbine: unknown node"),
            (Error::TurbineEmptyCluster, "turbine: empty cluster"),
        ];

        for (err, expected) in cases {
            assert_eq!(err.to_string(), expected);
        }
    }
}
