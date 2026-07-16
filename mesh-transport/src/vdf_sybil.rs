//! MinRoot-style sequential VDF for Sybil resistance and Ephemeral Job DID minting.
//!
//! # Overview
//! Identity creation for jobs requires burning sequential delay via a MinRoot-inspired
//! iterated modular map:
//!
//! ```text
//! x_{i+1} = (x_i + i)^d  mod p    where  d = (2p − 1) / 5
//! ```
//!
//! The map is intentionally sequential: each step depends on the previous output, so
//! parallel hardware (ASICs/GPU farms) cannot accelerate the delay. Verification re-runs
//! the same iteration count and checks equality of the final state.
//!
//! # Production parameter (≈10-minute wall-clock delay)
//! Calibrate `iterations` on the slowest supported edge class (Pi Zero 2 W class).
//! A conservative starting point documented for production deployments:
//!
//! ```text
//! PRODUCTION_ITERATIONS ≈ 50_000_000
//! ```
//!
//! Re-benchmark on target silicon and adjust so honest nodes need ~10 minutes of single-
//! core sequential work. Tests use small `iterations` (e.g. 8–64) for speed.
//!
//! # Ephemeral Job DID
//! ```text
//! JobDID = SHA-256( VDF_output_bytes || job_meta )
//! ```

use sha2::{Digest, Sha256};

/// Documented production iteration count targeting ≈10 minutes of sequential delay
/// on constrained edge hardware. Tests must pass a smaller configurable value.
///
/// Re-measure on the fleet's slowest device before locking consensus parameters.
pub const PRODUCTION_ITERATIONS: u64 = 50_000_000;

/// Default small field modulus used for portable unit tests.
/// Chosen so `p ≢ 1 (mod 5)` (103 % 5 = 3) for classical MinRoot exponent validity.
pub const DEFAULT_TEST_MODULUS: u64 = 103;

/// Fixed-size VDF state (u64 encoded big-endian in proofs / DID hashing).
pub type VdfState = u64;

/// Parameters for a MinRoot-style sequential VDF instance.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VdfParams {
    /// Prime (or prime-like) modulus `p` with `p ≢ 1 (mod 5)` so the fifth-root map
    /// can be expressed as exponentiation by `d = (2p − 1) / 5`.
    pub modulus: u64,
    /// Number of sequential iterations. Production: [`PRODUCTION_ITERATIONS`].
    pub iterations: u64,
}

impl VdfParams {
    /// Test-friendly parameters with configurable iteration count.
    pub fn for_tests(iterations: u64) -> Self {
        Self {
            modulus: DEFAULT_TEST_MODULUS,
            iterations,
        }
    }

    /// Production parameters targeting ≈10 minutes of sequential delay.
    pub fn production() -> Self {
        Self {
            // Portable u64 stand-in; production curves (e.g. Pallas) would use a big-int field.
            // Require `p ≡ 3 (mod 5)` so classical MinRoot exponent `d = (2p − 1) / 5` is integral.
            // 1_000_000_033 ≡ 3 (mod 5).
            modulus: 1_000_000_033,
            iterations: PRODUCTION_ITERATIONS,
        }
    }

    /// Exponent for the fifth-root map when valid.
    ///
    /// Prefer the classical MinRoot closed form `d = (2p − 1) / 5` (requires `p ≡ 3 (mod 5)`).
    /// Otherwise fall back to modular inverse of 5 modulo `(p − 1)` when `p ≢ 1 (mod 5)`.
    pub fn fifth_root_exponent(&self) -> Option<u64> {
        let p = self.modulus;
        if p < 5 || p % 5 == 1 {
            return None;
        }
        let num = 2u128 * p as u128 - 1;
        if num % 5 == 0 {
            return Some((num / 5) as u64);
        }
        // d ≡ 5^{-1} (mod p-1)
        mod_inverse(5, p - 1)
    }
}

impl Default for VdfParams {
    fn default() -> Self {
        Self::for_tests(16)
    }
}

/// Public proof that a sequential delay of `params.iterations` steps was performed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VdfProof {
    pub params: VdfParams,
    pub input: VdfState,
    pub output: VdfState,
    /// Explicit iteration count claimed by the prover (must match `params.iterations`).
    pub claimed_iterations: u64,
}

/// Ephemeral Job DID: 32-byte digest binding VDF output to job metadata.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct EphemeralJobDid(pub [u8; 32]);

impl EphemeralJobDid {
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// Modular inverse of `a` modulo `m` via extended Euclidean algorithm.
fn mod_inverse(a: u64, m: u64) -> Option<u64> {
    if m == 0 {
        return None;
    }
    let (mut t, mut new_t) = (0i128, 1i128);
    let (mut r, mut new_r) = (m as i128, (a % m) as i128);
    while new_r != 0 {
        let q = r / new_r;
        (t, new_t) = (new_t, t - q * new_t);
        (r, new_r) = (new_r, r - q * new_r);
    }
    if r > 1 {
        return None;
    }
    if t < 0 {
        t += m as i128;
    }
    Some(t as u64)
}

/// Modular exponentiation: `(base^exp) % modulus` using u128 intermediates.
pub fn mod_exp(mut base: u64, mut exp: u64, modulus: u64) -> u64 {
    if modulus == 0 {
        return 0;
    }
    if modulus == 1 {
        return 0;
    }
    let mut result: u64 = 1;
    base %= modulus;
    while exp > 0 {
        if exp & 1 == 1 {
            result = ((result as u128 * base as u128) % modulus as u128) as u64;
        }
        exp >>= 1;
        base = ((base as u128 * base as u128) % modulus as u128) as u64;
    }
    result
}

/// One sequential MinRoot-style step: `x' = (x + i)^d mod p`.
///
/// DOC: MinRoot requires `p ≢ 1 (mod 5)` so the fifth root maps to exponent
/// `d = (2p − 1) / 5`.
pub fn compute_minroot_step(x: u64, i: u64, p: u64) -> u64 {
    debug_assert!(p > 1, "modulus must be > 1");
    // Prefer classical MinRoot exponent when valid; fall back to cube map for
    // poorly chosen moduli so the sequential delay property still holds.
    let d = {
        let num = 2u128 * p as u128 - 1;
        if p % 5 != 1 && num % 5 == 0 {
            (num / 5) as u64
        } else {
            // Sequential hard step still enforced via modular powering.
            3
        }
    };
    let base = (x as u128 + i as u128) % p as u128;
    mod_exp(base as u64, d, p)
}

/// Evaluate the full sequential VDF.
///
/// Each iteration depends on the previous state — this loop cannot be parallelized.
pub fn evaluate_vdf(input: VdfState, params: &VdfParams) -> VdfProof {
    let mut current = input;
    for i in 1..=params.iterations {
        current = compute_minroot_step(current, i, params.modulus);
    }
    VdfProof {
        params: params.clone(),
        input,
        output: current,
        claimed_iterations: params.iterations,
    }
}

/// Verify a VDF proof by re-running the sequential steps.
///
/// Returns `true` iff the iteration count matches parameters and the output is correct.
pub fn verify_vdf_proof(proof: &VdfProof) -> bool {
    if proof.claimed_iterations != proof.params.iterations {
        return false;
    }
    if proof.params.modulus <= 1 {
        return false;
    }
    let mut current = proof.input;
    for i in 1..=proof.params.iterations {
        current = compute_minroot_step(current, i, proof.params.modulus);
    }
    current == proof.output
}

/// Legacy helper retained for callers that pass bare scalars.
pub fn verify_vdf(start_x: u64, steps: u64, final_x: u64, p: u64) -> bool {
    let proof = VdfProof {
        params: VdfParams {
            modulus: p,
            iterations: steps,
        },
        input: start_x,
        output: final_x,
        claimed_iterations: steps,
    };
    verify_vdf_proof(&proof)
}

/// Encode VDF output as fixed 8-byte big-endian for hashing.
pub fn vdf_output_bytes(output: VdfState) -> [u8; 8] {
    output.to_be_bytes()
}

/// Mint an Ephemeral Job DID:
/// `JobDID = SHA-256(VDF_output_bytes || job_meta)`.
///
/// Sybil resistance comes from the sequential VDF burn required to produce `output`.
pub fn mint_ephemeral_job_did(vdf_output: VdfState, job_meta: &[u8]) -> EphemeralJobDid {
    let mut hasher = Sha256::new();
    hasher.update(vdf_output_bytes(vdf_output));
    hasher.update(job_meta);
    let digest = hasher.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    EphemeralJobDid(out)
}

/// Convenience: evaluate VDF then mint Job DID in one shot.
pub fn mint_job_did_with_vdf(
    input: VdfState,
    params: &VdfParams,
    job_meta: &[u8],
) -> (VdfProof, EphemeralJobDid) {
    let proof = evaluate_vdf(input, params);
    let did = mint_ephemeral_job_did(proof.output, job_meta);
    (proof, did)
}

pub fn init_vdf_sybil() {
    println!("Initializing VDF Sybil protection (MinRoot sequential delay).");
    println!(
        "Production VDF iterations (≈10 min target): {}",
        PRODUCTION_ITERATIONS
    );
    let params = VdfParams::for_tests(3);
    let start = 42;
    let proof = evaluate_vdf(start, &params);
    println!(
        "VDF eval (p={}, iters={}): {} -> {} (verified={})",
        params.modulus,
        params.iterations,
        start,
        proof.output,
        verify_vdf_proof(&proof)
    );
    let did = mint_ephemeral_job_did(proof.output, b"demo-job-meta");
    println!("Ephemeral Job DID (hex prefix): {:02x?}", &did.0[..8]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_modular_exponentiation() {
        // 5^3 = 125; 125 % 13 = 8
        assert_eq!(mod_exp(5, 3, 13), 8);
        assert_eq!(mod_exp(2, 10, 1000), 24);
        assert_eq!(mod_exp(7, 0, 13), 1);
    }

    #[test]
    fn test_minroot_sequential_delay() {
        let p = DEFAULT_TEST_MODULUS;
        let start = 42;

        let step1 = compute_minroot_step(start, 1, p);
        let step2 = compute_minroot_step(step1, 2, p);
        let step3 = compute_minroot_step(step2, 3, p);

        assert!(verify_vdf(start, 3, step3, p));
        assert!(!verify_vdf(start, 3, step2, p));
    }

    #[test]
    fn test_fifth_root_exponent_well_defined() {
        let params = VdfParams::for_tests(1);
        assert_ne!(params.modulus % 5, 1);
        assert!(params.fifth_root_exponent().is_some());
        assert!(VdfParams::production().fifth_root_exponent().is_some());
    }

    #[test]
    fn test_vdf_verify_accepts_honest_proof() {
        let params = VdfParams::for_tests(16);
        let proof = evaluate_vdf(7, &params);
        assert_eq!(proof.claimed_iterations, 16);
        assert!(verify_vdf_proof(&proof));
    }

    #[test]
    fn test_vdf_verify_rejects_tampered_output() {
        let params = VdfParams::for_tests(12);
        let mut proof = evaluate_vdf(99, &params);
        proof.output = proof.output.wrapping_add(1);
        assert!(!verify_vdf_proof(&proof));
    }

    #[test]
    fn test_vdf_verify_rejects_iteration_mismatch() {
        let params = VdfParams::for_tests(10);
        let mut proof = evaluate_vdf(1, &params);
        proof.claimed_iterations = 9; // under-claim delay
        assert!(!verify_vdf_proof(&proof));
    }

    #[test]
    fn test_vdf_is_sequential_prefix_chain() {
        // Intermediate states of an N-step evaluation must match nested 1-step re-evals.
        let params = VdfParams::for_tests(5);
        let input = 17u64;
        let full = evaluate_vdf(input, &params);

        let mut current = input;
        for i in 1..=params.iterations {
            current = compute_minroot_step(current, i, params.modulus);
        }
        assert_eq!(current, full.output);
        assert!(verify_vdf_proof(&full));
    }

    #[test]
    fn test_ephemeral_job_did_binds_vdf_and_meta() {
        let params = VdfParams::for_tests(8);
        let (proof, did) = mint_job_did_with_vdf(5, &params, b"job-A");
        assert!(verify_vdf_proof(&proof));

        let did_same = mint_ephemeral_job_did(proof.output, b"job-A");
        assert_eq!(did, did_same);

        let did_other_meta = mint_ephemeral_job_did(proof.output, b"job-B");
        assert_ne!(did, did_other_meta);

        let other_proof = evaluate_vdf(6, &params);
        let did_other_vdf = mint_ephemeral_job_did(other_proof.output, b"job-A");
        assert_ne!(did, did_other_vdf);
    }

    #[test]
    fn test_production_iterations_documented() {
        // Guard the documented 10-minute production parameter surface.
        assert_eq!(PRODUCTION_ITERATIONS, 50_000_000);
        let prod = VdfParams::production();
        assert_eq!(prod.iterations, PRODUCTION_ITERATIONS);
        assert!(prod.iterations > 1_000_000);
    }
}
