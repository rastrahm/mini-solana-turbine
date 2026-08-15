//! Recepción UDP de shreds (`io_uring` en la fase 5).

pub mod datagram;
pub mod uring_udp;

pub use datagram::RecvDatagram;
pub use uring_udp::UdpIngress;
