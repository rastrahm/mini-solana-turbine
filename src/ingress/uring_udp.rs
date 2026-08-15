//! Receptor UDP sobre `io_uring`.
//!
//! Fase 1: stub de [`UdpIngress::bind`]. Sin sockets ni dependencias async.

use crate::Error;

/// Handle del socket de ingress. Vacío hasta la fase 5.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UdpIngress;

impl UdpIngress {
    /// Purpose: Abre un socket UDP bound a `addr` (stub).
    /// Inputs: `addr` — dirección de bind, p. ej. `0.0.0.0:8001` (ignorada aquí).
    /// Returns: `Err(Error::Unimplemented)` hasta la fase 5.
    pub fn bind(_addr: &str) -> Result<Self, Error> {
        Err(Error::Unimplemented { module: "ingress" })
    }
}

#[cfg(test)]
mod tests {
    use super::UdpIngress;
    use crate::Error;

    /// Purpose: El stub de bind no abre sockets.
    /// Inputs: none.
    /// Returns: panics si el error no es `Unimplemented { module: "ingress" }`.
    #[test]
    fn bind_is_unimplemented() {
        assert_eq!(
            UdpIngress::bind("127.0.0.1:0"),
            Err(Error::Unimplemented { module: "ingress" })
        );
    }
}
