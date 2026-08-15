//! Cálculo del árbol Turbine (stake-weighted fanout).
//!
//! Fase 1: stub de [`build`]. Identidades, stake y fanout llegan en la fase 6.

use crate::Error;

/// Árbol de reenvío. Vacío hasta la fase 6.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TurbineTree;

/// Purpose: Construye el árbol de fanout a partir del cluster (stub).
/// Inputs: `fanout` — hijos por nodo cuando exista implementación (ignorado aquí).
/// Returns: `Err(Error::Unimplemented)` hasta la fase 6.
pub fn build(_fanout: u8) -> Result<TurbineTree, Error> {
    Err(Error::Unimplemented { module: "turbine" })
}

#[cfg(test)]
mod tests {
    use super::build;
    use crate::Error;

    /// Purpose: El stub de construcción no calcula vecinos.
    /// Inputs: none.
    /// Returns: panics si el error no es `Unimplemented { module: "turbine" }`.
    #[test]
    fn build_is_unimplemented() {
        assert_eq!(build(2), Err(Error::Unimplemented { module: "turbine" }));
    }
}
