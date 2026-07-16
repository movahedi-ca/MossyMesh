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
//! # Production vs test parameters
//! | Constant | Value | Use |
//! | --- | --- | --- |
//! | [`PRODUCTION_ITERATIONS`] | 50_000_000 | ≈10 min sequential delay on edge silicon |
//! | [`DEFAULT_TEST_ITERATIONS`] | 16 | unit / integration tests only |
//! | [`MAX_TEST_ITERATIONS`] | 10_000 | hard cap for [`VdfParams::for_tests`] |
//!
//! Re-benchmark production iterations on the fleet's slowest device before locking
//! consensus parameters. Tests must never invoke [`PRODUCTION_ITERATIONS`] for wall-clock
//! delay (use [`VdfParams::for_tests`] / [`DEFAULT_TEST_ITERATIONS`]).
//!
//! # Ephemeral Job DID
//! ```text
//! JobDID = SHA-256( VDF_output_bytes || job_meta )
//! ```

use sha2::{Digest, Sha256};

/// Documented production iteration count targeting ≈10 minutes of sequential delay
/// on constrained edge hardware.
///
/// **Do not use this constant in unit tests.** Prefer [`DEFAULT_TEST_ITERATIONS`] via
/// [`VdfParams::for_tests`].
pub const PRODUCTION_ITERATIONS: u64 = 50_000_000;

/// Portable production modulus stand-in (u64 field).
///
/// Requirements (validated by [`validate_modulus`]):
/// - odd prime `p > 5`
/// - `p ≡ 3 (mod 5)` so classical MinRoot exponent `d = (2p − 1) / 5` is integral
///
/// `1_000_000_033` is prime and `1_000_000_033 % 5 == 3`.
pub const PRODUCTION_MODULUS: u64 = 1_000_000_033;

/// Default small field modulus used for portable unit tests.
/// Chosen so `p ≡ 3 (mod 5)` (103 % 5 = 3) for classical MinRoot exponent validity.
pub const DEFAULT_TEST_MODULUS: u64 = 103;

/// Default sequential iterations for unit tests (orders of magnitude below production).
pub const DEFAULT_TEST_ITERATIONS: u64 = 16;

/// Hard upper bound for [`VdfParams::for_tests`] so accidental production-scale
/// iteration counts never enter the test path.
pub const MAX_TEST_ITERATIONS: u64 = 10_000;

/// Fixed-size VDF state (u64 encoded big-endian in proofs / DID hashing).
pub type VdfState = u64;

/// Structured reasons a proof is rejected. Stable for logs / mesh diagnostics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VdfVerifyError {
    /// Modulus fails primality / MinRoot congruence checks.
    InvalidModulus,
    /// `claimed_iterations != params.iterations`.
    IterationMismatch,
    /// Zero-step "proofs" are never accepted.
    ZeroIterations,
    /// Recomputed output differs from claimed final state.
    OutputMismatch,
    /// Fifth-root exponent is undefined for the modulus.
    UndefinedExponent,
}

impl VdfVerifyError {
    /// Stable machine-readable error code.
    pub fn code(self) -> &'static str {
        match self {
            VdfVerifyError::InvalidModulus => "INVALID_MODULUS",
            VdfVerifyError::IterationMismatch => "ITERATION_MISMATCH",
            VdfVerifyError::ZeroIterations => "ZERO_ITERATIONS",
            VdfVerifyError::OutputMismatch => "OUTPUT_MISMATCH",
            VdfVerifyError::UndefinedExponent => "UNDEFINED_EXPONENT",
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            VdfVerifyError::InvalidModulus => "VDF verify failed: invalid modulus.",
            VdfVerifyError::IterationMismatch => "VDF verify failed: iteration claim mismatch.",
            VdfVerifyError::ZeroIterations => "VDF verify failed: zero iterations.",
            VdfVerifyError::OutputMismatch => "VDF verify failed: output mismatch.",
            VdfVerifyError::UndefinedExponent => "VDF verify failed: fifth-root exponent undefined.",
        }
    }
}

impl core::fmt::Display for VdfVerifyError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::error::Error for VdfVerifyError {}

/// Parameters for a MinRoot-style sequential VDF instance.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VdfParams {
    /// Prime modulus `p` with `p ≡ 3 (mod 5)` so the fifth-root map
    /// can be expressed as exponentiation by `d = (2p − 1) / 5`.
    pub modulus: u64,
    /// Number of sequential iterations. Production: [`PRODUCTION_ITERATIONS`].
    /// Tests: [`DEFAULT_TEST_ITERATIONS`] (via [`Self::for_tests`]).
    pub iterations: u64,
}

impl VdfParams {
    /// Test-friendly parameters with configurable iteration count.
    ///
    /// `iterations` is clamped to `1..=MAX_TEST_ITERATIONS` so production-scale
    /// delays cannot be requested on the test path by mistake.
    pub fn for_tests(iterations: u64) -> Self {
        let iterations = iterations.clamp(1, MAX_TEST_ITERATIONS);
        Self {
            modulus: DEFAULT_TEST_MODULUS,
            iterations,
        }
    }

    /// Production parameters targeting ≈10 minutes of sequential delay.
    pub fn production() -> Self {
        Self {
            modulus: PRODUCTION_MODULUS,
            iterations: PRODUCTION_ITERATIONS,
        }
    }

    /// Validate modulus + that a fifth-root exponent exists.
    pub fn validate(&self) -> Result<(), VdfVerifyError> {
        if !validate_modulus(self.modulus) {
            return Err(VdfVerifyError::InvalidModulus);
        }
        if self.fifth_root_exponent().is_none() {
            return Err(VdfVerifyError::UndefinedExponent);
        }
        if self.iterations == 0 {
            return Err(VdfVerifyError::ZeroIterations);
        }
        Ok(())
    }

    /// Exponent for the fifth-root map when valid.
    ///
    /// Classical MinRoot closed form `d = (2p − 1) / 5` (requires `p ≡ 3 (mod 5)`).
    /// Falls back to modular inverse of 5 modulo `(p − 1)` when `p ≢ 1 (mod 5)`.
    pub fn fifth_root_exponent(&self) -> Option<u64> {
        fifth_root_exponent_for(self.modulus)
    }
}

impl Default for VdfParams {
    fn default() -> Self {
        Self::for_tests(DEFAULT_TEST_ITERATIONS)
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

/// Deterministic Miller–Rabin witnesses sufficient for all `u64` integers
/// (Jaeschke bases for n < 3_317_044_064_679_887_385_961_981, covers u64).
const MR_WITNESSES: &[u64] = &[2, 3, 5, 7, 11, 13, 23];

/// Strong modulus validation for MinRoot:
/// - `p > 5` and odd
/// - `p ≢ 1 (mod 5)` (fifth root of unity condition)
/// - prefer classical form: `p ≡ 3 (mod 5)` so `d = (2p−1)/5` is integral
/// - probable prime under Miller–Rabin with fixed deterministic witnesses
pub fn validate_modulus(p: u64) -> bool {
    if p <= 5 || p % 2 == 0 {
        return false;
    }
    // Classical MinRoot: p ≡ 3 (mod 5). Also accept other residues ≠ 1 if
    // inverse of 5 mod (p-1) exists (checked separately for exponent).
    if p % 5 == 1 {
        return false;
    }
    // Require classical integral d for production-grade acceptance.
    let num = 2u128 * p as u128 - 1;
    if num % 5 != 0 {
        return false;
    }
    is_prime_u64(p)
}

/// Primality test for `u64` via deterministic Miller–Rabin.
pub fn is_prime_u64(n: u64) -> bool {
    if n < 2 {
        return false;
    }
    // Small primes + quick composite filters.
    const SMALL: &[u64] = &[2, 3, 5, 7, 11, 13, 17, 19, 23, 29, 31, 37];
    for &p in SMALL {
        if n == p {
            return true;
        }
        if n % p == 0 {
            return false;
        }
    }
    // Write n-1 = d * 2^s with d odd.
    let mut d = n - 1;
    let mut s = 0u32;
    while d % 2 == 0 {
        d /= 2;
        s += 1;
    }
    'witness: for &a in MR_WITNESSES {
        if a % n == 0 {
            continue;
        }
        let mut x = mod_exp(a, d, n);
        if x == 1 || x == n - 1 {
            continue 'witness;
        }
        for _ in 1..s {
            x = ((x as u128 * x as u128) % n as u128) as u64;
            if x == n - 1 {
                continue 'witness;
            }
        }
        return false;
    }
    true
}

fn fifth_root_exponent_for(p: u64) -> Option<u64> {
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
/// Requires a valid MinRoot modulus (`validate_modulus` / defined fifth-root
/// exponent). Returns `None` when the exponent is undefined (callers must not
/// fall back to weaker maps in production verify paths).
pub fn try_compute_minroot_step(x: u64, i: u64, p: u64) -> Option<u64> {
    let d = fifth_root_exponent_for(p)?;
    if p <= 1 {
        return None;
    }
    let base = (x as u128 + i as u128) % p as u128;
    Some(mod_exp(base as u64, d, p))
}

/// One sequential MinRoot-style step (panics in debug if modulus is invalid).
///
/// DOC: MinRoot requires `p ≢ 1 (mod 5)` so the fifth root maps to exponent
/// `d = (2p − 1) / 5`. Production verify uses [`try_compute_minroot_step`].
pub fn compute_minroot_step(x: u64, i: u64, p: u64) -> u64 {
    match try_compute_minroot_step(x, i, p) {
        Some(y) => y,
        None => {
            // Last-resort sequential map for legacy callers with non-MinRoot moduli.
            // Verification paths must reject these moduli via [`validate_modulus`].
            debug_assert!(p > 1, "modulus must be > 1");
            if p <= 1 {
                return 0;
            }
            let base = (x as u128 + i as u128) % p as u128;
            mod_exp(base as u64, 3, p)
        }
    }
}

/// Evaluate the full sequential VDF.
///
/// Each iteration depends on the previous state — this loop cannot be parallelized.
/// Callers should pass params that pass [`VdfParams::validate`]; invalid moduli still
/// evaluate (legacy) but will fail [`verify_vdf_proof_detailed`].
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

/// Evaluate only when parameters are cryptographically acceptable.
pub fn evaluate_vdf_checked(
    input: VdfState,
    params: &VdfParams,
) -> Result<VdfProof, VdfVerifyError> {
    params.validate()?;
    Ok(evaluate_vdf(input, params))
}

/// Verify a VDF proof by re-running the sequential steps (detailed errors).
pub fn verify_vdf_proof_detailed(proof: &VdfProof) -> Result<(), VdfVerifyError> {
    if proof.params.iterations == 0 || proof.claimed_iterations == 0 {
        return Err(VdfVerifyError::ZeroIterations);
    }
    if proof.claimed_iterations != proof.params.iterations {
        return Err(VdfVerifyError::IterationMismatch);
    }
    if !validate_modulus(proof.params.modulus) {
        return Err(VdfVerifyError::InvalidModulus);
    }
    if fifth_root_exponent_for(proof.params.modulus).is_none() {
        return Err(VdfVerifyError::UndefinedExponent);
    }
    let mut current = proof.input;
    for i in 1..=proof.params.iterations {
        current = try_compute_minroot_step(current, i, proof.params.modulus)
            .ok_or(VdfVerifyError::UndefinedExponent)?;
    }
    if current != proof.output {
        return Err(VdfVerifyError::OutputMismatch);
    }
    Ok(())
}

/// Verify a VDF proof by re-running the sequential steps.
///
/// Returns `true` iff the modulus is valid, the iteration count matches parameters,
/// and the output is correct.
pub fn verify_vdf_proof(proof: &VdfProof) -> bool {
    verify_vdf_proof_detailed(proof).is_ok()
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
        "Production VDF iterations (≈10 min target): {} (modulus={})",
        PRODUCTION_ITERATIONS, PRODUCTION_MODULUS
    );
    println!(
        "Test VDF defaults: iterations={} (max {}), modulus={}",
        DEFAULT_TEST_ITERATIONS, MAX_TEST_ITERATIONS, DEFAULT_TEST_MODULUS
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
    fn test_is_prime_u64_basic() {
        assert!(is_prime_u64(2));
        assert!(is_prime_u64(3));
        assert!(is_prime_u64(103));
        assert!(is_prime_u64(PRODUCTION_MODULUS));
        assert!(!is_prime_u64(1));
        assert!(!is_prime_u64(9));
        assert!(!is_prime_u64(100));
        assert!(!is_prime_u64(1_000_000_035));
    }

    #[test]
    fn test_validate_modulus_rejects_weak() {
        assert!(!validate_modulus(0));
        assert!(!validate_modulus(1));
        assert!(!validate_modulus(2));
        assert!(!validate_modulus(4));
        assert!(!validate_modulus(11)); // 11 ≡ 1 (mod 5)
        assert!(!validate_modulus(25)); // composite
        assert!(validate_modulus(DEFAULT_TEST_MODULUS));
        assert!(validate_modulus(PRODUCTION_MODULUS));
        assert_eq!(PRODUCTION_MODULUS % 5, 3);
        assert_eq!(DEFAULT_TEST_MODULUS % 5, 3);
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
        assert!(params.validate().is_ok());
        // Do not call production().validate() with evaluate — only check params surface.
        assert!(VdfParams::production().validate().is_ok());
    }

    #[test]
    fn test_vdf_verify_accepts_honest_proof() {
        let params = VdfParams::for_tests(DEFAULT_TEST_ITERATIONS);
        let proof = evaluate_vdf(7, &params);
        assert_eq!(proof.claimed_iterations, DEFAULT_TEST_ITERATIONS);
        assert!(verify_vdf_proof(&proof));
        assert!(verify_vdf_proof_detailed(&proof).is_ok());
    }

    #[test]
    fn test_vdf_verify_rejects_tampered_output() {
        let params = VdfParams::for_tests(12);
        let mut proof = evaluate_vdf(99, &params);
        proof.output = proof.output.wrapping_add(1);
        assert!(!verify_vdf_proof(&proof));
        assert_eq!(
            verify_vdf_proof_detailed(&proof).unwrap_err(),
            VdfVerifyError::OutputMismatch
        );
        assert_eq!(
            VdfVerifyError::OutputMismatch.code(),
            "OUTPUT_MISMATCH"
        );
    }

    #[test]
    fn test_vdf_verify_rejects_iteration_mismatch() {
        let params = VdfParams::for_tests(10);
        let mut proof = evaluate_vdf(1, &params);
        proof.claimed_iterations = 9; // under-claim delay
        assert!(!verify_vdf_proof(&proof));
        assert_eq!(
            verify_vdf_proof_detailed(&proof).unwrap_err(),
            VdfVerifyError::IterationMismatch
        );
    }

    #[test]
    fn test_vdf_verify_rejects_invalid_modulus() {
        let mut proof = evaluate_vdf(5, &VdfParams::for_tests(4));
        proof.params.modulus = 11; // ≡ 1 (mod 5)
        proof.claimed_iterations = proof.params.iterations;
        assert_eq!(
            verify_vdf_proof_detailed(&proof).unwrap_err(),
            VdfVerifyError::InvalidModulus
        );

        proof.params.modulus = 9; // composite
        assert_eq!(
            verify_vdf_proof_detailed(&proof).unwrap_err(),
            VdfVerifyError::InvalidModulus
        );
    }

    #[test]
    fn test_vdf_verify_rejects_zero_iterations() {
        let proof = VdfProof {
            params: VdfParams {
                modulus: DEFAULT_TEST_MODULUS,
                iterations: 0,
            },
            input: 1,
            output: 1,
            claimed_iterations: 0,
        };
        assert_eq!(
            verify_vdf_proof_detailed(&proof).unwrap_err(),
            VdfVerifyError::ZeroIterations
        );
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
    fn test_production_iterations_separated_from_tests() {
        // Guard the documented 10-minute production parameter surface.
        assert_eq!(PRODUCTION_ITERATIONS, 50_000_000);
        assert_eq!(PRODUCTION_MODULUS, 1_000_000_033);
        let prod = VdfParams::production();
        assert_eq!(prod.iterations, PRODUCTION_ITERATIONS);
        assert_eq!(prod.modulus, PRODUCTION_MODULUS);
        assert!(prod.iterations > 1_000_000);

        // Test path is orders of magnitude smaller and capped.
        assert!(DEFAULT_TEST_ITERATIONS < MAX_TEST_ITERATIONS);
        assert!(MAX_TEST_ITERATIONS < PRODUCTION_ITERATIONS);
        let t = VdfParams::for_tests(DEFAULT_TEST_ITERATIONS);
        assert_eq!(t.iterations, DEFAULT_TEST_ITERATIONS);
        assert_ne!(t.iterations, PRODUCTION_ITERATIONS);

        // for_tests clamps runaway iteration requests.
        let huge = VdfParams::for_tests(PRODUCTION_ITERATIONS);
        assert_eq!(huge.iterations, MAX_TEST_ITERATIONS);
        assert!(huge.iterations < PRODUCTION_ITERATIONS);
    }

    #[test]
    fn test_evaluate_checked_rejects_bad_params() {
        let bad = VdfParams {
            modulus: 11,
            iterations: 4,
        };
        assert_eq!(
            evaluate_vdf_checked(1, &bad).unwrap_err(),
            VdfVerifyError::InvalidModulus
        );
        let good = VdfParams::for_tests(4);
        let proof = evaluate_vdf_checked(3, &good).unwrap();
        assert!(verify_vdf_proof(&proof));
    }
}
