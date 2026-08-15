//! Crate de aprendizaje: ingestión de shreds y fanout estilo Turbine.
//!
//! Fase 7: [`pipeline::Pipeline`] une parse, FEC y destinos Turbine.

pub mod arena;
pub mod error;
pub mod fec;
pub mod ingress;
pub mod pipeline;
pub mod shred;
pub mod turbine;

pub use arena::{PacketArena, SlotId, PACKET_SIZE};
pub use error::Error;
pub use fec::{FecEngine, DEFAULT_SHARD_BYTES};
pub use ingress::{RecvDatagram, UdpIngress};
pub use pipeline::{slot_queue, ForwardPlan, IngestResult, Pipeline};
pub use shred::{CodeShred, DataShred, Shred, ShredHeader};
pub use turbine::{Node, NodeId, Stake, TurbineTree};

#[cfg(test)]
mod tests {
    use super::*;

    /// Purpose: Humo de API tras unir el pipeline.
    /// Inputs: none.
    /// Returns: panics si parse o cluster vacío no dan los errores esperados.
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
