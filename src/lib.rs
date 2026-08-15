//! Crate de aprendizaje: ingestión de shreds y fanout estilo Turbine.
//!
//! Extra: firmas Ed25519 educativas, [`metrics::Metrics`], features `uring` / `simd`.

pub mod arena;
pub mod error;
#[cfg(feature = "simd")]
pub mod fec;
pub mod ingress;
pub mod metrics;
#[cfg(feature = "simd")]
pub mod pipeline;
pub mod shred;
pub mod turbine;

pub use arena::{PacketArena, SlotId, PACKET_SIZE};
pub use error::Error;
#[cfg(feature = "simd")]
pub use fec::{FecEngine, DEFAULT_SHARD_BYTES};
#[cfg(feature = "uring")]
pub use ingress::UdpIngress;
pub use ingress::{parse_addr, RecvDatagram};
pub use metrics::{Metrics, MetricsSnapshot};
#[cfg(feature = "simd")]
pub use pipeline::{slot_queue, ForwardPlan, IngestResult, Pipeline};
pub use shred::{
    CodeShred, DataShred, Shred, ShredHeader, ShredPublicKey, ShredSecretKey, SIGNATURE_BYTES,
};
pub use turbine::{Node, NodeId, Stake, TurbineTree};

#[cfg(test)]
mod tests {
    use super::*;

    /// Purpose: Humo de API: parse, bind addr, árbol; FEC si `simd`.
    /// Inputs: none.
    /// Returns: panics si parse o cluster vacío no dan los errores esperados.
    #[test]
    fn stubs_share_unimplemented_error() {
        assert_eq!(shred::parse(&[]), Err(Error::ShredTruncated));
        #[cfg(feature = "simd")]
        assert!(FecEngine::new(2, 1, 64).is_ok());
        assert_eq!(parse_addr("nope"), Err(Error::IngressBind));
        assert_eq!(
            turbine::tree::build(&[], 2).err(),
            Some(Error::TurbineEmptyCluster)
        );
    }
}
