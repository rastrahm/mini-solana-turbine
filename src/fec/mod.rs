//! Erasure coding SIMD (Reed-Solomon) para reconstruir shreds.

pub mod reed_solomon;

pub use reed_solomon::{FecEngine, DEFAULT_SHARD_BYTES};
