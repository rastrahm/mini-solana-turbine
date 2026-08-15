//! Árbol de fanout ponderado por stake y routing de shreds.

pub mod tree;

pub use tree::{build, Node, NodeId, Stake, TurbineTree, DEFAULT_FANOUT};
