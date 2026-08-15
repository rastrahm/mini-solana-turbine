//! UDP de shreds: parseo de bind addr siempre; `io_uring` detrás del feature `uring`.

use crate::Error;
use std::net::SocketAddr;

pub mod datagram;

pub use datagram::RecvDatagram;

#[cfg(feature = "uring")]
pub mod uring_udp;
#[cfg(feature = "uring")]
pub use uring_udp::UdpIngress;

/// Purpose: Interpreta `host:port` sin I/O ni `io_uring`.
/// Inputs: `addr` — p. ej. `127.0.0.1:0`.
/// Returns: [`SocketAddr`] o [`Error::IngressBind`].
pub fn parse_addr(addr: &str) -> Result<SocketAddr, Error> {
    addr.parse().map_err(|_| Error::IngressBind)
}

#[cfg(test)]
mod tests {
    use super::parse_addr;
    use crate::Error;

    /// Purpose: Un addr que no es `host:port` no se acepta (sin feature `uring`).
    /// Inputs: none.
    /// Returns: panics si no es `IngressBind`.
    #[test]
    fn parse_addr_rejects_garbage() {
        assert_eq!(parse_addr("nope"), Err(Error::IngressBind));
    }
}
