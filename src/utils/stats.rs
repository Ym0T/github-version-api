//! Request and cache counters.
use std::sync::atomic::{AtomicU64, Ordering};

/// Counters are lifetime totals (restored from disk at startup), except
/// `session_requests` which only counts since the process started.
#[derive(Debug, Default)]
pub struct Stats {
    total_requests: AtomicU64,
    cache_hits: AtomicU64,
    cache_misses: AtomicU64,
    session_requests: AtomicU64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatsSnapshot {
    pub total_requests: u64,
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub session_requests: u64,
}

impl Stats {
    pub fn with_totals(total_requests: u64, cache_hits: u64, cache_misses: u64) -> Self {
        Self {
            total_requests: AtomicU64::new(total_requests),
            cache_hits: AtomicU64::new(cache_hits),
            cache_misses: AtomicU64::new(cache_misses),
            session_requests: AtomicU64::new(0),
        }
    }

    pub fn record_request(&self) {
        self.total_requests.fetch_add(1, Ordering::Relaxed);
        self.session_requests.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_cache_hit(&self) {
        self.cache_hits.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_cache_miss(&self) {
        self.cache_misses.fetch_add(1, Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> StatsSnapshot {
        StatsSnapshot {
            total_requests: self.total_requests.load(Ordering::Relaxed),
            cache_hits: self.cache_hits.load(Ordering::Relaxed),
            cache_misses: self.cache_misses.load(Ordering::Relaxed),
            session_requests: self.session_requests.load(Ordering::Relaxed),
        }
    }
}

impl StatsSnapshot {
    /// Cache hit rate in percent with one decimal.
    pub fn hit_rate(&self) -> f64 {
        let lookups = self.cache_hits + self.cache_misses;
        if lookups == 0 {
            return 0.0;
        }
        (self.cache_hits as f64 / lookups as f64 * 1000.0).round() / 10.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_and_hit_rate() {
        let stats = Stats::with_totals(10, 1, 1);
        stats.record_request();
        stats.record_cache_hit();
        stats.record_cache_hit();
        let snapshot = stats.snapshot();
        assert_eq!(snapshot.total_requests, 11);
        assert_eq!(snapshot.session_requests, 1);
        assert_eq!(snapshot.hit_rate(), 75.0);
        assert_eq!(Stats::default().snapshot().hit_rate(), 0.0);
    }
}
