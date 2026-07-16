//! Commit-and-reveal VRF-like task routing with dynamic primary/standby assignment.
//!
//! # Protocol
//! 1. **Commit**: each coordinator (or job poster) publishes `H(seed || domain_sep)`.
//! 2. **Reveal**: after the commit window, the seed is opened; anyone can recompute the
//!    binding hash and reject mismatches.
//! 3. **Assign**: workers are ranked by a deterministic score mixing the revealed seed
//!    with node metrics (load, thermal, battery). Top 3 become **primary**, next 2
//!    **standby** (Dynamic Triangulation: standbys replace dropping primaries).
//!
//! Pure functions only — callers supply metric snapshots; no I/O.

use sha2::{Digest, Sha256};

/// Fixed assignment geometry for Dynamic Triangulation.
pub const PRIMARY_COUNT: usize = 3;
pub const STANDBY_COUNT: usize = 2;
pub const TOTAL_ASSIGNED: usize = PRIMARY_COUNT + STANDBY_COUNT;

/// Thermal ceiling (°C) above which nodes are deprioritized (matches thermal_aware policy).
pub const THERMAL_SOFT_LIMIT_C: u32 = 75;

/// Metrics snapshot used by pure scoring hooks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkerMetrics {
    /// Unique worker identifier bytes (PeerID / public key hash prefix, etc.).
    pub worker_id: Vec<u8>,
    /// Active job load (0 = idle). Lower is better (least-loaded-first).
    pub active_jobs: u32,
    /// CPU temperature in whole degrees Celsius.
    pub temperature_c: u32,
    /// Battery percent 0–100. AC-powered nodes may report 100.
    pub battery_percent: u8,
    /// Optional stake/reputation weight for sortition (defaults treated as 1).
    pub stake_weight: u64,
}

/// Outcome of a commit phase.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Commitment {
    pub hash: [u8; 32],
}

/// Opened seed after the reveal window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reveal {
    pub seed: Vec<u8>,
}

/// Assigned worker roles for a single job.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkerAssignment {
    pub primaries: Vec<Vec<u8>>,
    pub standbys: Vec<Vec<u8>>,
}

impl WorkerAssignment {
    pub fn primary_count(&self) -> usize {
        self.primaries.len()
    }

    pub fn standby_count(&self) -> usize {
        self.standbys.len()
    }

    pub fn total_count(&self) -> usize {
        self.primaries.len() + self.standbys.len()
    }
}

/// Domain separation for commit hashes.
const COMMIT_DOMAIN: &[u8] = b"mossymesh/vrf-commit/v1";

/// Create a commit binding: `H(domain || seed)`.
pub fn commit_seed(seed: &[u8]) -> Commitment {
    let mut hasher = Sha256::new();
    hasher.update(COMMIT_DOMAIN);
    hasher.update(seed);
    let digest = hasher.finalize();
    let mut hash = [0u8; 32];
    hash.copy_from_slice(&digest);
    Commitment { hash }
}

/// Verify that a reveal opens the prior commitment.
pub fn verify_reveal(commitment: &Commitment, reveal: &Reveal) -> bool {
    let recomputed = commit_seed(&reveal.seed);
    recomputed.hash == commitment.hash
}

/// Bind a job identifier to the revealed seed (prevents seed reuse across jobs).
pub fn bind_seed_to_job(seed: &[u8], job_id: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"mossymesh/vrf-job-bind/v1");
    hasher.update(seed);
    hasher.update(job_id);
    let digest = hasher.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    out
}

/// Evaluates if a node is selected for a task based on its VRF hash output and its weight.
///
/// Sortition: `Hash_Value < (Max_Hash * weight) / total_network_weight`
/// Uses the top 64 bits of the hash; `u128` intermediates avoid overflow.
pub fn is_selected_for_task(vrf_hash_top_64: u64, node_weight: u64, total_network_weight: u64) -> bool {
    if total_network_weight == 0 {
        return false;
    }
    let threshold =
        ((u64::MAX as u128 * node_weight as u128) / total_network_weight as u128) as u64;
    vrf_hash_top_64 < threshold
}

/// Pure thermal hook: penalty score 0 (cool) .. 1000 (hot/over limit).
/// Nodes at or above [`THERMAL_SOFT_LIMIT_C`] receive maximum penalty.
pub fn thermal_penalty(temperature_c: u32) -> u32 {
    if temperature_c >= THERMAL_SOFT_LIMIT_C {
        return 1000;
    }
    // Linear ramp from 0°C → 0 penalty, limit → 999.
    (temperature_c as u64 * 999 / THERMAL_SOFT_LIMIT_C as u64) as u32
}

/// Pure battery hook: capacity score 0 (dead) .. 1000 (full / AC).
/// Mirrors battery_tracker cliff around 20% without floating point.
pub fn battery_capacity_score(battery_percent: u8) -> u32 {
    let threshold: i32 = 20;
    let b = battery_percent as i32;
    let diff = b - threshold;
    if diff <= -10 {
        return 0;
    }
    if diff >= 10 {
        return 1000;
    }
    let numerator = diff * 1000;
    let denominator = 2 * (1 + diff.abs());
    let mut weight = 500 + (numerator / denominator);
    if weight < 0 {
        weight = 0;
    } else if weight > 1000 {
        weight = 1000;
    }
    weight as u32
}

/// Pure load hook: higher active job count → higher penalty.
pub fn load_penalty(active_jobs: u32) -> u32 {
    // Cap so a single very busy node does not overflow ranking math.
    active_jobs.saturating_mul(100).min(10_000)
}

/// Composite rank key: **lower is better** (least-loaded + cool + high battery).
///
/// Incorporates a deterministic VRF-derived tie-break from `(bound_seed || worker_id)`.
pub fn worker_rank_key(bound_seed: &[u8; 32], metrics: &WorkerMetrics) -> u128 {
    let load = load_penalty(metrics.active_jobs) as u128;
    let thermal = thermal_penalty(metrics.temperature_c) as u128;
    let battery = battery_capacity_score(metrics.battery_percent) as u128;
    // Invert battery so higher capacity lowers the key.
    let battery_penalty = 1000u128.saturating_sub(battery);

    let mut hasher = Sha256::new();
    hasher.update(bound_seed);
    hasher.update(&metrics.worker_id);
    let digest = hasher.finalize();
    let mut tie = [0u8; 8];
    tie.copy_from_slice(&digest[..8]);
    let tie_break = u64::from_be_bytes(tie) as u128;

    // Weighted sum in high bits; VRF tie-break in low bits for deterministic uniqueness.
    let score = load * 1_000_000 + thermal * 1_000 + battery_penalty;
    (score << 64) | tie_break
}

/// Assign 3 primary + 2 standby workers using least-loaded / thermal / battery scoring
/// mixed with the commit-reveal bound seed.
///
/// If fewer than 5 candidates exist, fills primaries first, then standbys, as available.
pub fn assign_workers(bound_seed: &[u8; 32], candidates: &[WorkerMetrics]) -> WorkerAssignment {
    let mut ranked: Vec<(u128, usize)> = candidates
        .iter()
        .enumerate()
        .map(|(idx, m)| (worker_rank_key(bound_seed, m), idx))
        .collect();
    // Stable deterministic order: rank key ascending, then index.
    ranked.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));

    let mut primaries = Vec::new();
    let mut standbys = Vec::new();
    for (_, idx) in ranked {
        let id = candidates[idx].worker_id.clone();
        if primaries.len() < PRIMARY_COUNT {
            primaries.push(id);
        } else if standbys.len() < STANDBY_COUNT {
            standbys.push(id);
        } else {
            break;
        }
    }

    WorkerAssignment {
        primaries,
        standbys,
    }
}

/// End-to-end helper: verify reveal, bind to job, assign workers.
pub fn assign_from_commit_reveal(
    commitment: &Commitment,
    reveal: &Reveal,
    job_id: &[u8],
    candidates: &[WorkerMetrics],
) -> Result<WorkerAssignment, &'static str> {
    if !verify_reveal(commitment, reveal) {
        return Err("reveal does not open commitment");
    }
    let bound = bind_seed_to_job(&reveal.seed, job_id);
    Ok(assign_workers(&bound, candidates))
}

/// VRF-style proof placeholder (hash + ed25519 signature slot).
pub struct VrfProof {
    pub hash: [u8; 32],
    pub signature: [u8; 64],
}

pub fn init_vrf_assigner() {
    println!("Initializing VRF Assigner for dynamic primary/standby worker allocation.");
    let sample_hash = 0x0FFFFFFFFFFFFFFF;
    let weight = 100;
    let total_weight = 1000;
    let selected = is_selected_for_task(sample_hash, weight, total_weight);
    println!("VRF Sortition test -> Selected: {}", selected);

    let seed = b"demo-seed";
    let commitment = commit_seed(seed);
    let reveal = Reveal {
        seed: seed.to_vec(),
    };
    assert!(verify_reveal(&commitment, &reveal));
    let workers = demo_workers();
    let bound = bind_seed_to_job(seed, b"job-1");
    let assignment = assign_workers(&bound, &workers);
    println!(
        "Assignment demo -> primaries: {}, standbys: {}",
        assignment.primary_count(),
        assignment.standby_count()
    );
}

fn demo_workers() -> Vec<WorkerMetrics> {
    (0..6)
        .map(|i| WorkerMetrics {
            worker_id: vec![i],
            active_jobs: i as u32,
            temperature_c: 40 + i as u32 * 5,
            battery_percent: 100 - i * 10,
            stake_weight: 1,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn worker(id: u8, jobs: u32, temp: u32, batt: u8) -> WorkerMetrics {
        WorkerMetrics {
            worker_id: vec![id],
            active_jobs: jobs,
            temperature_c: temp,
            battery_percent: batt,
            stake_weight: 1,
        }
    }

    #[test]
    fn test_vrf_sortition_selection() {
        let max_hash = u64::MAX;
        let vrf_hash = max_hash / 4;
        assert!(is_selected_for_task(vrf_hash, 100, 200));
        assert!(!is_selected_for_task(vrf_hash, 20, 200));
    }

    #[test]
    fn test_vrf_overflow_protection() {
        // High weights that would overflow naive u64 scaling (u128 intermediate required).
        // Threshold ≈ 99.9999% of u64::MAX; a mid-range hash must still be selected.
        let vrf_hash = u64::MAX / 2;
        assert!(is_selected_for_task(vrf_hash, 999_999, 1_000_000));
        // Zero total weight is a safe reject (no panic / no division by zero).
        assert!(!is_selected_for_task(vrf_hash, 1, 0));
    }

    #[test]
    fn test_commit_reveal_binding() {
        let seed = b"secret-seed-bytes-42";
        let commitment = commit_seed(seed);
        let good = Reveal {
            seed: seed.to_vec(),
        };
        let bad = Reveal {
            seed: b"other-seed".to_vec(),
        };
        assert!(verify_reveal(&commitment, &good));
        assert!(!verify_reveal(&commitment, &bad));

        // Binding to different jobs yields different streams.
        let b1 = bind_seed_to_job(seed, b"job-1");
        let b2 = bind_seed_to_job(seed, b"job-2");
        assert_ne!(b1, b2);
    }

    #[test]
    fn test_worker_assignment_counts_three_primary_two_standby() {
        let seed = b"assignment-seed";
        let commitment = commit_seed(seed);
        let reveal = Reveal {
            seed: seed.to_vec(),
        };
        let candidates: Vec<WorkerMetrics> = (0..8)
            .map(|i| worker(i, i as u32 % 3, 50 + i as u32, 80))
            .collect();

        let assignment =
            assign_from_commit_reveal(&commitment, &reveal, b"job-xyz", &candidates).unwrap();

        assert_eq!(assignment.primary_count(), PRIMARY_COUNT);
        assert_eq!(assignment.standby_count(), STANDBY_COUNT);
        assert_eq!(assignment.total_count(), TOTAL_ASSIGNED);
        assert_eq!(PRIMARY_COUNT, 3);
        assert_eq!(STANDBY_COUNT, 2);
    }

    #[test]
    fn test_assignment_prefers_least_loaded() {
        let bound = bind_seed_to_job(b"seed", b"job");
        let candidates = vec![
            worker(1, 5, 40, 100), // busy
            worker(2, 0, 40, 100), // idle
            worker(3, 1, 40, 100),
            worker(4, 2, 40, 100),
            worker(5, 3, 40, 100),
        ];
        let assignment = assign_workers(&bound, &candidates);
        // Idle worker should be among primaries.
        assert!(assignment.primaries.iter().any(|id| id == &vec![2]));
        assert_eq!(assignment.primary_count(), 3);
        assert_eq!(assignment.standby_count(), 2);
    }

    #[test]
    fn test_thermal_and_battery_hooks() {
        assert_eq!(thermal_penalty(THERMAL_SOFT_LIMIT_C), 1000);
        assert!(thermal_penalty(30) < thermal_penalty(70));
        assert_eq!(battery_capacity_score(100), 1000);
        assert_eq!(battery_capacity_score(9), 0);
        assert!(battery_capacity_score(25) > battery_capacity_score(15));
    }

    #[test]
    fn test_bad_reveal_rejected() {
        let commitment = commit_seed(b"real");
        let reveal = Reveal {
            seed: b"fake".to_vec(),
        };
        let err = assign_from_commit_reveal(&commitment, &reveal, b"job", &[]).unwrap_err();
        assert_eq!(err, "reveal does not open commitment");
    }

    #[test]
    fn test_assignment_deterministic() {
        let bound = bind_seed_to_job(b"s", b"j");
        let candidates = vec![
            worker(10, 1, 55, 90),
            worker(11, 1, 55, 90),
            worker(12, 1, 55, 90),
            worker(13, 1, 55, 90),
            worker(14, 1, 55, 90),
        ];
        let a = assign_workers(&bound, &candidates);
        let b = assign_workers(&bound, &candidates);
        assert_eq!(a, b);
    }
}
