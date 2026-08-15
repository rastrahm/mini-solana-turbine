//! Crate de aprendizaje: ingestión de shreds y fanout estilo Turbine.
//!
//! Fase 6: [`turbine::TurbineTree`] calcula el fanout; el envío UDP queda para la fase 8.

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
pub use turbine::{Node, NodeId, Stake, TurbineTree};

#[cfg(test)]
mod tests {
    use super::*;

    /// Purpose: Humo de API: módulos vivos; cluster vacío sigue siendo error de Turbine.
    /// Inputs: none.
    /// Returns: panics si parse falla o el cluster vacío no da `TurbineEmptyCluster`.
    #[test]
    fn stubs_share_unimplemented_error() {
        assert_eq!(shred::parse(&[]), Err(Error::ShredTruncated));
        assert!(FecEngine::new(2, 1, 64).is_ok());
        assert_eq!(
            ingress::uring_udp::UdpIngress::parse_addr("nope"),
            Err(Error::IngressBind)
        );
        assert_eq!(
            turbine::tree::build(&[], 2).err(),
            Some(Error::TurbineEmptyCluster)
        );
    }
}
