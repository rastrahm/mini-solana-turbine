//! Crate de aprendizaje: ingestión de shreds y fanout estilo Turbine.
//!
//! Fase 4: [`fec::FecEngine`] reconstruye shards; UDP y Turbine siguen stub.

pub mod arena;
pub mod error;
pub mod fec;
pub mod ingress;
pub mod shred;
pub mod turbine;

pub use arena::{PacketArena, SlotId, PACKET_SIZE};
pub use error::Error;
pub use fec::FecEngine;
pub use shred::{CodeShred, DataShred, Shred, ShredHeader};

#[cfg(test)]
mod tests {
    use super::*;

    /// Purpose: Humo de API: shred/fec vivos; ingress y turbine siguen stub.
    /// Inputs: none.
    /// Returns: panics si parse([]) no trunca o un stub no es `Unimplemented`.
    #[test]
    fn stubs_share_unimplemented_error() {
        assert_eq!(shred::parse(&[]), Err(Error::ShredTruncated));
        assert!(FecEngine::new(2, 1, 64).is_ok());
        assert!(matches!(
            ingress::uring_udp::UdpIngress::bind("127.0.0.1:0"),
            Err(Error::Unimplemented { module: "ingress" })
        ));
        assert!(matches!(
            turbine::tree::build(2),
            Err(Error::Unimplemented { module: "turbine" })
        ));
    }
}
