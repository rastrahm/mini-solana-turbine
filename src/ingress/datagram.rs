//! Datagrama recibido: índice de arena y origen UDP.

use crate::arena::SlotId;
use std::net::SocketAddr;

/// Resultado de un recv: handle de arena + origen. El payload está en el slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecvDatagram {
    /// Slot donde el kernel escribió el datagrama.
    pub slot: SlotId,
    /// Dirección fuente UDP.
    pub src: SocketAddr,
    /// Bytes válidos (`== arena.len(slot)`).
    pub len: usize,
}
