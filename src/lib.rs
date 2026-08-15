//! Crate de aprendizaje: ingestión de shreds y fanout estilo Turbine.
//!
//! Fase 3: [`shred::parse`] proyecta headers packed y payloads sobre slots de [`arena`].

pub mod arena;
pub mod error;
pub mod fec;
pub mod ingress;
pub mod shred;
pub mod turbine;

pub use arena::{PacketArena, SlotId, PACKET_SIZE};
pub use error::Error;
pub use shred::{CodeShred, DataShred, Shred, ShredHeader};

#[cfg(test)]
mod tests {
    use super::*;

    /// Purpose: Humo de API: shred ya parsea; el resto de módulos siguen stub.
    /// Inputs: none.
    /// Returns: panics si un stub no devuelve `Unimplemented` o si parse([]) no trunca.
    #[test]
    fn stubs_share_unimplemented_error() {
        assert_eq!(shred::parse(&[]), Err(Error::ShredTruncated));
        assert!(matches!(
            ingress::uring_udp::UdpIngress::bind("127.0.0.1:0"),
            Err(Error::Unimplemented { module: "ingress" })
        ));
        assert!(matches!(
            fec::reed_solomon::encode(),
            Err(Error::Unimplemented { module: "fec" })
        ));
        assert!(matches!(
            turbine::tree::build(2),
            Err(Error::Unimplemented { module: "turbine" })
        ));
    }
}
