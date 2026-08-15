//! UDP de shreds (`io_uring`): recv a slots y send/forward sin clonar payload.

pub mod datagram;
pub mod uring_udp;

pub use datagram::RecvDatagram;
pub use uring_udp::UdpIngress;
