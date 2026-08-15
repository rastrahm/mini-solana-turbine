//! Erasure coding SIMD (Reed-Solomon). Feature `simd`.

pub mod reed_solomon;

pub use reed_solomon::{FecEngine, DEFAULT_SHARD_BYTES};
