//! Statistical anomaly detection forcing failing nodes into 1-hour hardware diagnostics.
//!
//! Tensors / job results undergo Statistical Anomaly Detection. Nodes failing
//! 3 checks are forced into Quarantine to run a 1-hour hardware diagnostic
//! benchmark checking for silent CPU decay.

use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Number of consecutive anomaly strikes before hardware quarantine.
pub const STRIKE_THRESHOLD: u32 = 3;

/// Production diagnostic duration (1 hour).
pub const DIAGNOSTIC_DURATION_SECS: u64 = 3600;

/// Short diagnostic duration used by unit tests (progress ticks).
pub const TEST_DIAGNOSTIC_TICKS: u32 = 5;

/// Relative deviation (basis points of 10_000) above which a sample is anomalous.
/// 500 bps = 5% away from the reference mean.
pub const ANOMALY_BPS_THRESHOLD: u32 = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaultType {
    CrashFault,
    ByzantineFault,
    /// Silent statistical drift (wrong tensor / result values without a crash).
    StatisticalAnomaly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeHealth {
    Healthy,
    /// Accumulating strikes; still eligible for work.
    Warned { strikes: u32 },
    /// Forced offline for the diagnostic benchmark window.
    Quarantined,
    /// Diagnostic failed — node is banned from compute jobs.
    Banned,
}

#[derive(Debug, Clone)]
pub struct NodeRecord {
    pub peer_id: String,
    pub strikes: u32,
    pub health: NodeHealth,
    pub last_fault: Option<FaultType>,
    /// When set, quarantine ends at this instant (production path).
    pub quarantine_until: Option<Instant>,
    /// Remaining synthetic ticks for the test diagnostic stub.
    pub diagnostic_ticks_remaining: u32,
}

impl NodeRecord {
    pub fn new(peer_id: impl Into<String>) -> Self {
        Self {
            peer_id: peer_id.into(),
            strikes: 0,
            health: NodeHealth::Healthy,
            last_fault: None,
            quarantine_until: None,
            diagnostic_ticks_remaining: 0,
        }
    }

    pub fn is_eligible_for_work(&self) -> bool {
        matches!(self.health, NodeHealth::Healthy | NodeHealth::Warned { .. })
    }
}

/// Tracks per-node anomaly strikes and drives quarantine / diagnostic lifecycle.
#[derive(Debug, Default)]
pub struct QuarantineManager {
    nodes: HashMap<String, NodeRecord>,
    /// When true, diagnostic runs for `TEST_DIAGNOSTIC_TICKS` instead of 1 hour.
    pub test_mode: bool,
}

impl QuarantineManager {
    pub fn new() -> Self {
        Self {
            nodes: HashMap::new(),
            test_mode: false,
        }
    }

    /// Test-only constructor: short diagnostic with progress ticks.
    pub fn new_for_tests() -> Self {
        Self {
            nodes: HashMap::new(),
            test_mode: true,
        }
    }

    pub fn get(&self, peer_id: &str) -> Option<&NodeRecord> {
        self.nodes.get(peer_id)
    }

    fn ensure(&mut self, peer_id: &str) -> &mut NodeRecord {
        self.nodes
            .entry(peer_id.to_string())
            .or_insert_with(|| NodeRecord::new(peer_id))
    }

    /// Record a statistical (or other) fault against a peer.
    /// Returns the updated health after applying the strike.
    pub fn record_strike(&mut self, peer_id: &str, fault: FaultType) -> NodeHealth {
        let test_mode = self.test_mode;
        let record = self.ensure(peer_id);
        if matches!(record.health, NodeHealth::Banned | NodeHealth::Quarantined) {
            return record.health;
        }

        record.strikes = record.strikes.saturating_add(1);
        record.last_fault = Some(fault);

        if record.strikes >= STRIKE_THRESHOLD {
            record.health = NodeHealth::Quarantined;
            if test_mode {
                record.diagnostic_ticks_remaining = TEST_DIAGNOSTIC_TICKS;
                record.quarantine_until = None;
            } else {
                record.quarantine_until =
                    Some(Instant::now() + Duration::from_secs(DIAGNOSTIC_DURATION_SECS));
                record.diagnostic_ticks_remaining = 0;
            }
        } else {
            record.health = NodeHealth::Warned {
                strikes: record.strikes,
            };
        }
        record.health
    }

    /// Reset strikes after a clean result (honest compute recovered).
    pub fn record_clean(&mut self, peer_id: &str) {
        let record = self.ensure(peer_id);
        if matches!(record.health, NodeHealth::Banned | NodeHealth::Quarantined) {
            return;
        }
        record.strikes = 0;
        record.health = NodeHealth::Healthy;
        record.last_fault = None;
    }

    /// Advance one diagnostic progress tick (test path) or check wall-clock expiry.
    /// Returns true if the node left quarantine successfully.
    pub fn tick_diagnostic(&mut self, peer_id: &str, diagnostic_passed: bool) -> bool {
        let record = match self.nodes.get_mut(peer_id) {
            Some(r) => r,
            None => return false,
        };
        if !matches!(record.health, NodeHealth::Quarantined) {
            return false;
        }

        if record.diagnostic_ticks_remaining > 0 {
            record.diagnostic_ticks_remaining =
                record.diagnostic_ticks_remaining.saturating_sub(1);
            if record.diagnostic_ticks_remaining > 0 {
                return false;
            }
            return self.finish_diagnostic(peer_id, diagnostic_passed);
        }

        if let Some(until) = record.quarantine_until {
            if Instant::now() < until {
                return false;
            }
            return self.finish_diagnostic(peer_id, diagnostic_passed);
        }

        false
    }

    fn finish_diagnostic(&mut self, peer_id: &str, diagnostic_passed: bool) -> bool {
        let record = match self.nodes.get_mut(peer_id) {
            Some(r) => r,
            None => return false,
        };
        record.quarantine_until = None;
        record.diagnostic_ticks_remaining = 0;
        if diagnostic_passed {
            record.strikes = 0;
            record.health = NodeHealth::Healthy;
            record.last_fault = None;
            true
        } else {
            record.health = NodeHealth::Banned;
            false
        }
    }

    /// Progress percentage of the current diagnostic (0–100).
    pub fn diagnostic_progress_pct(&self, peer_id: &str) -> Option<u32> {
        let record = self.nodes.get(peer_id)?;
        if !matches!(record.health, NodeHealth::Quarantined) {
            return None;
        }
        if self.test_mode {
            let remaining = record.diagnostic_ticks_remaining;
            let done = TEST_DIAGNOSTIC_TICKS.saturating_sub(remaining);
            return Some((done * 100) / TEST_DIAGNOSTIC_TICKS.max(1));
        }
        if let Some(until) = record.quarantine_until {
            let total = DIAGNOSTIC_DURATION_SECS as f64;
            let left = until
                .saturating_duration_since(Instant::now())
                .as_secs_f64();
            let elapsed = (total - left).clamp(0.0, total);
            Some(((elapsed / total) * 100.0) as u32)
        } else {
            Some(0)
        }
    }
}

/// Integer mean of i64 samples (floor division). Empty → 0.
pub fn mean_i64(samples: &[i64]) -> i64 {
    if samples.is_empty() {
        return 0;
    }
    let sum: i128 = samples.iter().map(|&v| v as i128).sum();
    (sum / samples.len() as i128) as i64
}

/// Population variance × 1000 (milli-units) to avoid floats.
pub fn variance_milli(samples: &[i64]) -> u64 {
    if samples.len() < 2 {
        return 0;
    }
    let m = mean_i64(samples) as i128;
    let mut acc: i128 = 0;
    for &s in samples {
        let d = s as i128 - m;
        acc += d * d;
    }
    ((acc * 1000) / samples.len() as i128) as u64
}

/// Returns true when `value` deviates from `reference_mean` by more than
/// `ANOMALY_BPS_THRESHOLD` basis points (or absolute unit floor of 1 when mean is 0).
///
/// DOC: Tensor / result statistical anomaly check used before strike accounting.
pub fn is_statistical_anomaly(value: i64, reference_mean: i64) -> bool {
    if reference_mean == 0 {
        return value != 0;
    }
    let diff = (value as i128 - reference_mean as i128).unsigned_abs();
    let denom = reference_mean.unsigned_abs() as u128;
    let bps = (diff * 10_000) / denom;
    bps > ANOMALY_BPS_THRESHOLD as u128
}

/// Compare a submitted result tensor against a consensus reference tensor element-wise.
/// Any anomalous element counts as a failed check for the submitting peer.
pub fn check_tensor_anomaly(submitted: &[i64], reference: &[i64]) -> bool {
    if submitted.len() != reference.len() || submitted.is_empty() {
        return true; // shape mismatch is anomalous
    }
    let ref_mean = mean_i64(reference);
    for &v in submitted {
        if is_statistical_anomaly(v, ref_mean) {
            return true;
        }
    }
    let sub_mean = mean_i64(submitted);
    is_statistical_anomaly(sub_mean, ref_mean)
}

/// Stub for the 1-hour hardware diagnostic benchmark.
/// In production this would run a known CPU workload and verify cycle counts.
/// Returns a simple deterministic score; callers decide pass/fail thresholds.
pub fn run_diagnostic_benchmark_stub(seed: u64, iterations: u32) -> u64 {
    let mut x = seed ^ 0x9E37_79B9_7F4A_7C15;
    for i in 0..iterations {
        x = x
            .wrapping_mul(6364136223846793005)
            .wrapping_add(i as u64 + 1);
        x ^= x >> 17;
    }
    x
}

pub fn init_quarantine() {
    println!("Initializing Statistical Anomaly Detection & Quarantine logic.");
    let mut mgr = QuarantineManager::new();
    let _ = mgr.record_strike("demo-peer", FaultType::StatisticalAnomaly);
    println!(
        "Quarantine demo: peer strikes after 1 anomaly = {:?}",
        mgr.get("demo-peer").map(|r| r.health)
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_three_strikes_quarantine() {
        let mut mgr = QuarantineManager::new_for_tests();
        assert_eq!(
            mgr.record_strike("n1", FaultType::StatisticalAnomaly),
            NodeHealth::Warned { strikes: 1 }
        );
        assert_eq!(
            mgr.record_strike("n1", FaultType::StatisticalAnomaly),
            NodeHealth::Warned { strikes: 2 }
        );
        assert_eq!(
            mgr.record_strike("n1", FaultType::StatisticalAnomaly),
            NodeHealth::Quarantined
        );
        assert!(!mgr.get("n1").unwrap().is_eligible_for_work());
        assert_eq!(
            mgr.get("n1").unwrap().diagnostic_ticks_remaining,
            TEST_DIAGNOSTIC_TICKS
        );
    }

    #[test]
    fn test_clean_result_resets_strikes() {
        let mut mgr = QuarantineManager::new_for_tests();
        mgr.record_strike("n2", FaultType::ByzantineFault);
        mgr.record_strike("n2", FaultType::ByzantineFault);
        mgr.record_clean("n2");
        assert_eq!(mgr.get("n2").unwrap().strikes, 0);
        assert_eq!(mgr.get("n2").unwrap().health, NodeHealth::Healthy);
    }

    #[test]
    fn test_diagnostic_progress_ticks() {
        let mut mgr = QuarantineManager::new_for_tests();
        mgr.record_strike("n3", FaultType::StatisticalAnomaly);
        mgr.record_strike("n3", FaultType::StatisticalAnomaly);
        mgr.record_strike("n3", FaultType::StatisticalAnomaly);

        assert_eq!(mgr.diagnostic_progress_pct("n3"), Some(0));

        for _ in 0..(TEST_DIAGNOSTIC_TICKS - 1) {
            assert!(!mgr.tick_diagnostic("n3", true));
        }
        assert!(mgr.tick_diagnostic("n3", true));
        assert_eq!(mgr.get("n3").unwrap().health, NodeHealth::Healthy);
        assert_eq!(mgr.diagnostic_progress_pct("n3"), None);
    }

    #[test]
    fn test_failed_diagnostic_bans_node() {
        let mut mgr = QuarantineManager::new_for_tests();
        for _ in 0..STRIKE_THRESHOLD {
            mgr.record_strike("n4", FaultType::CrashFault);
        }
        for _ in 0..(TEST_DIAGNOSTIC_TICKS - 1) {
            mgr.tick_diagnostic("n4", false);
        }
        assert!(!mgr.tick_diagnostic("n4", false));
        assert_eq!(mgr.get("n4").unwrap().health, NodeHealth::Banned);
    }

    #[test]
    fn test_tensor_anomaly_detection() {
        let reference = vec![100, 102, 98, 101, 99];
        let honest = vec![100, 101, 99, 100, 100];
        let malicious = vec![100, 100, 100, 100, 500];

        assert!(!check_tensor_anomaly(&honest, &reference));
        assert!(check_tensor_anomaly(&malicious, &reference));
        assert!(check_tensor_anomaly(&[1, 2], &reference));
    }

    #[test]
    fn test_is_statistical_anomaly_bps() {
        assert!(!is_statistical_anomaly(100, 100));
        assert!(!is_statistical_anomaly(104, 100)); // 4% < 5%
        assert!(is_statistical_anomaly(106, 100)); // 6% > 5%
        assert!(is_statistical_anomaly(1, 0));
        assert!(!is_statistical_anomaly(0, 0));
    }

    #[test]
    fn test_diagnostic_benchmark_stub_deterministic() {
        let a = run_diagnostic_benchmark_stub(42, 100);
        let b = run_diagnostic_benchmark_stub(42, 100);
        let c = run_diagnostic_benchmark_stub(43, 100);
        assert_eq!(a, b);
        assert_ne!(a, c);
    }
}
