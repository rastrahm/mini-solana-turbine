//! Crate de aprendizaje: ingestión de shreds y fanout estilo Turbine.
//!
//! Fase 2: [`arena`] ofrece slots fijos. El parseo, UDP y FEC siguen siendo stubs.

pub mod arena;
pub mod error;
pub mod fec;
pub mod ingress;
pub mod shred;
pub mod turbine;

pub use arena::{PacketArena, SlotId, PACKET_SIZE};
pub use error::Error;

#[cfg(test)]
mod tests {
    use super::*;

    /// Purpose: Humo de API pública: los stubs existen y usan el mismo `Error`.
    /// Inputs: none.
    /// Returns: panics si algún stub no devuelve `Unimplemented`.
    #[test]
    fn stubs_share_unimplemented_error() {
        assert!(matches!(
            shred::parse(&[]),
            Err(Error::Unimplemented { module: "shred" })
        ));
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
