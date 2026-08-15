//! Contadores lock-free del pipeline (`AtomicU64`, sin `Mutex`).
//!
//! `received` incluye paquetes que luego se descartan. `dropped` son ingestiones
//! que devolvieron `Err`. `reconstructed` acumula data shards restaurados por FEC.

use core::sync::atomic::{AtomicU64, Ordering};

/// Snapshot de tres contadores (copia, no atómico).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MetricsSnapshot {
    received: u64,
    reconstructed: u64,
    dropped: u64,
}

/// Contadores del validador. Seguros para lecturas concurrentes (`Relaxed`).
#[derive(Debug)]
pub struct Metrics {
    received: AtomicU64,
    reconstructed: AtomicU64,
    dropped: AtomicU64,
}

impl MetricsSnapshot {
    /// Purpose: Paquetes que pasaron por ingest (ok o error).
    /// Inputs: none (`self`).
    /// Returns: valor copiado.
    #[inline(always)]
    pub const fn received(self) -> u64 {
        self.received
    }

    /// Purpose: Data shards reconstruidos (suma de `IngestResult::reconstructed`).
    /// Inputs: none.
    /// Returns: valor copiado.
    #[inline(always)]
    pub const fn reconstructed(self) -> u64 {
        self.reconstructed
    }

    /// Purpose: Ingestiones que fallaron (parse, firma, FEC, etc.).
    /// Inputs: none.
    /// Returns: valor copiado.
    #[inline(always)]
    pub const fn dropped(self) -> u64 {
        self.dropped
    }
}

impl Metrics {
    /// Purpose: Ceros.
    /// Inputs: none.
    /// Returns: contadores en 0.
    pub const fn new() -> Self {
        Self {
            received: AtomicU64::new(0),
            reconstructed: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
        }
    }

    /// Purpose: `received += 1`.
    /// Inputs: none (`&self`).
    /// Returns: none.
    #[inline(always)]
    pub fn record_received(&self) {
        self.received.fetch_add(1, Ordering::Relaxed);
    }

    /// Purpose: Suma data shards restaurados en un ingest.
    /// Inputs: `n` — `0` es no-op.
    /// Returns: none.
    #[inline(always)]
    pub fn record_reconstructed(&self, n: u64) {
        if n != 0 {
            self.reconstructed.fetch_add(n, Ordering::Relaxed);
        }
    }

    /// Purpose: `dropped += 1`.
    /// Inputs: none.
    /// Returns: none.
    #[inline(always)]
    pub fn record_dropped(&self) {
        self.dropped.fetch_add(1, Ordering::Relaxed);
    }

    /// Purpose: Lee `received`.
    /// Inputs: none.
    /// Returns: carga `Relaxed`.
    #[inline(always)]
    pub fn received(&self) -> u64 {
        self.received.load(Ordering::Relaxed)
    }

    /// Purpose: Lee `reconstructed`.
    /// Inputs: none.
    /// Returns: carga `Relaxed`.
    #[inline(always)]
    pub fn reconstructed(&self) -> u64 {
        self.reconstructed.load(Ordering::Relaxed)
    }

    /// Purpose: Lee `dropped`.
    /// Inputs: none.
    /// Returns: carga `Relaxed`.
    #[inline(always)]
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    /// Purpose: Copia los tres contadores de una vez (cada carga es `Relaxed`).
    /// Inputs: none.
    /// Returns: [`MetricsSnapshot`].
    #[inline(always)]
    pub fn snapshot(&self) -> MetricsSnapshot {
        MetricsSnapshot {
            received: self.received(),
            reconstructed: self.reconstructed(),
            dropped: self.dropped(),
        }
    }
}

impl Default for Metrics {
    /// Purpose: Igual que [`Metrics::new`].
    /// Inputs: none.
    /// Returns: ceros.
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::Metrics;

    /// Purpose: Los tres contadores arrancan en 0.
    /// Inputs: none.
    /// Returns: panics si alguno no es 0.
    #[test]
    fn starts_at_zero() {
        let m = Metrics::new();
        let snap = m.snapshot();
        assert_eq!(snap.received(), 0);
        assert_eq!(snap.reconstructed(), 0);
        assert_eq!(snap.dropped(), 0);
    }

    /// Purpose: `record_*` incrementa de forma visible.
    /// Inputs: none.
    /// Returns: panics si los valores no coinciden.
    #[test]
    fn records_three_counters() {
        let m = Metrics::new();
        m.record_received();
        m.record_received();
        m.record_reconstructed(0);
        m.record_reconstructed(3);
        m.record_dropped();
        assert_eq!(m.received(), 2);
        assert_eq!(m.reconstructed(), 3);
        assert_eq!(m.dropped(), 1);
    }
}
