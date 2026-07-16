//! Measured nodes/sec (Mnps) over a fixed workload.
//!
//! Target ~836 Mnps is aspirational for optimized native bitboard kernels on
//! high-end hosts. This crate measures real throughput via perft + search nodes;
//! do not hard-code 836. WASM (wasm32-wasip1) is expected to report lower values.

use std::time::Instant;

use shakmaty::Chess;

use crate::search::{negamax_search, perft};

/// Fixed benchmark workload description (deterministic).
pub struct BenchmarkReport {
    /// Million nodes per second (nodes / seconds / 1e6).
    pub mnps: f64,
    /// Total nodes counted across the workload.
    pub nodes: u64,
    /// Wall time in seconds.
    pub seconds: f64,
    /// Human-readable workload label.
    pub workload: &'static str,
}

/// Run a fixed, deterministic workload and report measured Mnps.
///
/// Workload:
/// 1. `perft(startpos, 4)` — classic move-gen tree (197_281 nodes)
/// 2. `negamax_search(startpos, 3)` — eval + make/unmake search nodes
///
/// Nodes from both phases are summed. Timing uses `std::time::Instant`
/// (available on native and modern WASI targets).
pub fn benchmark_mnps() -> f64 {
    benchmark_mnps_detailed().mnps
}

/// Same as [`benchmark_mnps`] but returns full metrics for logging/tests.
pub fn benchmark_mnps_detailed() -> BenchmarkReport {
    let pos = Chess::default();
    let start = Instant::now();

    // Phase A: perft depth 4 (d1=20, d2=400, d3=8902, d4=197281).
    let perft_nodes = perft(&pos, 4);

    // Phase B: shallow search accumulates its own node counter.
    let search = negamax_search(&pos, 3);

    let elapsed = start.elapsed();
    let seconds = elapsed.as_secs_f64().max(1e-12);
    let nodes = perft_nodes.saturating_add(search.nodes);
    let mnps = (nodes as f64) / seconds / 1_000_000.0;

    BenchmarkReport {
        mnps,
        nodes,
        seconds,
        workload: "perft(d4)+negamax(d3) startpos",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn benchmark_returns_finite_positive() {
        let r = benchmark_mnps_detailed();
        assert!(r.nodes > 100_000, "expected substantial fixed workload");
        assert!(r.seconds > 0.0);
        assert!(r.mnps.is_finite());
        assert!(r.mnps > 0.0);
        let m = benchmark_mnps();
        assert!(m.is_finite() && m > 0.0);
    }
}
