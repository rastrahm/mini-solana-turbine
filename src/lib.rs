//! Crate de aprendizaje: ingestión de shreds y fanout estilo Turbine.
//!
//! Fase 5: [`ingress::UdpIngress`] recibe UDP en slots de arena; Turbine sigue stub.

pub mod arena;
pub mod error;
pub mod fec;
pub mod ingress;
pub mod shred;
pub mod turbine;

pub use arena::{PacketArena, SlotId, PACKET_SIZE};
pub use error::Error;
pub use fec::FecEngine;
pub use ingress::{RecvDatagram, UdpIngress};
pub use shred::{CodeShred, DataShred, Shred, ShredHeader};

#[cfg(test)]
mod tests {
    use super::*;

    /// Purpose: Humo de API: shred/fec/ingress parse; turbine sigue stub.
    /// Inputs: none.
    /// Returns: panics si parse([]) no trunca o el stub de turbine no es `Unimplemented`.
    #[test]
    fn stubs_share_unimplemented_error() {
        assert_eq!(shred::parse(&[]), Err(Error::ShredTruncated));
        assert!(FecEngine::new(2, 1, 64).is_ok());
        assert_eq!(
            ingress::uring_udp::UdpIngress::parse_addr("nope"),
            Err(Error::IngressBind)
        );
        assert!(matches!(
            turbine::tree::build(2),
            Err(Error::Unimplemented { module: "turbine" })
        ));
    }
}
