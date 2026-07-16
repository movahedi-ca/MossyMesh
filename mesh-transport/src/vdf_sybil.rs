//! MinRoot-style sequential VDF for Sybil resistance and Ephemeral Job DID minting.
//!
//! # Overview
//! Identity creation for jobs requires burning sequential delay via a MinRoot-inspired
//! iterated modular map:
//!
//! ```text
//! x_{i+1} = (x_i + i)^d  mod p    where  d ≡ 5^{-1} (mod p − 1)
//! ```
//!
//! When `p ≡ 3 (mod 5)`, the classical closed form applies:
//!
//! ```text
//! d = (2p − 1) / 5
//! ```
//!
//! The map is intentionally sequential: each step depends on the previous output, so
//! parallel hardware cannot accelerate a *single* proof's delay (though an attacker may
//! run many independent proofs on many cores). Verification re-runs the same iteration
//! count and checks equality of the final state (same asymptotic cost as evaluation —
//! not a Wesolowski/Pietrzak short proof).
//!
//! # Parameter validity
//! Fifth-root exponentiation is a group automorphism of `𝔽_p^*` iff
//! `gcd(5, p − 1) = 1`, i.e. **`p ≢ 1 (mod 5)`**. Instances with `p ≡ 1 (mod 5)` are
//! rejected by [`VdfParams::is_valid`], [`evaluate_vdf`], and [`verify_vdf_proof`].
//!
//! Prefer **`p ≡ 3 (mod 5)`** so the classical integral form `d = (2p − 1)/5` applies.
//!
//! # Production parameter (≈10-minute wall-clock delay)
//! Calibrate `iterations` on the slowest supported edge class (Pi Zero 2 W class).
//! A conservative starting point documented for production deployments:
//!
//! ```text
//! PRODUCTION_MODULUS   = 1_000_000_033   (prime, ≡ 3 mod 5)
//! PRODUCTION_ITERATIONS ≈ 50_000_000
//! ```
//!
//! Re-benchmark on target silicon and adjust so honest nodes need ~10 minutes of single-
//! core sequential work. Tests use small `iterations` (e.g. 8–64) for speed.
//!
//! See `docs/math-minroot-vdf.md` for the full parameter checklist and security claims.
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

/// Production u64 stand-in modulus: prime and `≡ 3 (mod 5)`.
///
/// ```text
/// 1_000_000_033 % 5 = 3
/// d = (2p − 1) / 5 = 400_000_013
/// 5d ≡ 1 (mod p − 1)
/// ```
///
/// Full deployments may replace this with a large prime field (e.g. Pallas scalar
/// field) once big-int MinRoot is wired; the congruence conditions are identical.
pub const PRODUCTION_MODULUS: u64 = 1_000_000_033;

/// Default small field modulus used for portable unit tests.
/// Chosen so `p ≡ 3 (mod 5)` (103 % 5 = 3) for classical MinRoot exponent validity.
pub const DEFAULT_TEST_MODULUS: u64 = 103;

/// Fixed-size VDF state (u64 encoded big-endian in proofs / DID hashing).
pub type VdfState = u64;

/// Parameters for a MinRoot-style sequential VDF instance.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VdfParams {
    /// Prime modulus `p` with `p ≢ 1 (mod 5)` so the fifth-root map
    /// can be expressed as exponentiation by `d ≡ 5^{-1} (mod p − 1)`.
    /// Prefer `p ≡ 3 (mod 5)` for the classical closed form `d = (2p − 1) / 5`.
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
            // 1_000_000_033 ≡ 3 (mod 5), prime.
            modulus: PRODUCTION_MODULUS,
            iterations: PRODUCTION_ITERATIONS,
        }
    }

    /// True when modulus admits a well-defined fifth-root exponent and is large enough
    /// for modular arithmetic (`p ≥ 5`, `p ≢ 1 (mod 5)`, and `5^{-1} (mod p−1)` exists).
    pub fn is_valid(&self) -> bool {
        self.modulus >= 5 && self.fifth_root_exponent().is_some()
    }

    /// Exponent for the fifth-root map when valid.
    ///
    /// Prefer the classical MinRoot closed form `d = (2p − 1) / 5` (requires `p ≡ 3 (mod 5)`
    /// so that `(2p − 1)` is divisible by 5). Otherwise fall back to the modular inverse
    /// of 5 modulo `(p − 1)` when `gcd(5, p − 1) = 1` (equivalently `p ≢ 1 (mod 5)`).
    ///
    /// Returns `None` when the fifth root is not a unique group automorphism
    /// (`p ≡ 1 (mod 5)` or `p < 5`).
    pub fn fifth_root_exponent(&self) -> Option<u64> {
        fifth_root_exponent_for(self.modulus)
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

/// Compute the MinRoot fifth-root exponent for modulus `p`, or `None` if invalid.
///
/// Conditions:
/// - `p ≥ 5`
/// - `p ≢ 1 (mod 5)` so `gcd(5, p−1) = 1`
/// - Prefer `d = (2p−1)/5` when integral (`p ≡ 3 (mod 5)`); else `d = 5^{-1} mod (p−1)`.
pub fn fifth_root_exponent_for(p: u64) -> Option<u64> {
    if p < 5 || p % 5 == 1 {
        return None;
    }
    // Classical closed form: d = (2p − 1)/5 when p ≡ 3 (mod 5).
    // Check: 5d = 2p − 1 ≡ 1 (mod p−1) because 2p − 2 = 2(p−1).
    let num = 2u128 * p as u128 - 1;
    if num % 5 == 0 {
        return Some((num / 5) as u64);
    }
    // p ≡ 2 or 4 (mod 5): inverse still exists, closed form does not.
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

/// One sequential MinRoot-style step: `x' = (x + i)^d mod p` with
/// `d ≡ 5^{-1} (mod p − 1)`.
///
/// Returns `None` when `p` is not a valid MinRoot modulus (`p < 5` or `p ≡ 1 (mod 5)`).
pub fn try_compute_minroot_step(x: u64, i: u64, p: u64) -> Option<u64> {
    let d = fifth_root_exponent_for(p)?;
    let base = (x as u128 + i as u128) % p as u128;
    Some(mod_exp(base as u64, d, p))
}

/// One sequential MinRoot-style step: `x' = (x + i)^d mod p`.
///
/// # Panics
/// Panics if `p` is not a valid MinRoot modulus (`p < 5` or `p ≡ 1 (mod 5)`).
/// Callers that must not panic should use [`try_compute_minroot_step`] or
/// validate with [`VdfParams::is_valid`] / [`fifth_root_exponent_for`].
///
/// DOC: MinRoot requires `p ≢ 1 (mod 5)` so the fifth root maps to exponent
/// `d ≡ 5^{-1} (mod p − 1)` (classically `d = (2p − 1) / 5` when `p ≡ 3 (mod 5)`).
pub fn compute_minroot_step(x: u64, i: u64, p: u64) -> u64 {
    try_compute_minroot_step(x, i, p).unwrap_or_else(|| {
        panic!(
            "invalid MinRoot modulus p={p}: require p ≥ 5 and p ≢ 1 (mod 5) so d ≡ 5^{{-1}} (mod p−1) exists"
        )
    })
}

/// Evaluate the full sequential VDF.
///
/// Each iteration depends on the previous state — this loop cannot be parallelized
/// within a single proof.
///
/// # Panics
/// Panics if `params` are invalid (`!params.is_valid()`). Use
/// [`try_evaluate_vdf`] for a non-panicking API.
pub fn evaluate_vdf(input: VdfState, params: &VdfParams) -> VdfProof {
    try_evaluate_vdf(input, params).expect("VdfParams must be valid for evaluate_vdf")
}

/// Fallible VDF evaluation: returns `None` when parameters are invalid
/// (`p ≡ 1 (mod 5)`, `p < 5`, or missing fifth-root exponent).
pub fn try_evaluate_vdf(input: VdfState, params: &VdfParams) -> Option<VdfProof> {
    if !params.is_valid() {
        return None;
    }
    let d = params.fifth_root_exponent()?;
    let mut current = input;
    for i in 1..=params.iterations {
        let base = (current as u128 + i as u128) % params.modulus as u128;
        current = mod_exp(base as u64, d, params.modulus);
    }
    Some(VdfProof {
        params: params.clone(),
        input,
        output: current,
        claimed_iterations: params.iterations,
    })
}

/// Verify a VDF proof by re-running the sequential steps.
///
/// Returns `true` iff:
/// - parameters admit a fifth-root exponent (`p ≢ 1 (mod 5)`, `p ≥ 5`),
/// - claimed iteration count matches `params.iterations`,
/// - re-evaluation from `input` yields `output`.
pub fn verify_vdf_proof(proof: &VdfProof) -> bool {
    if proof.claimed_iterations != proof.params.iterations {
        return false;
    }
    if !proof.params.is_valid() {
        return false;
    }
    let Some(expected) = try_evaluate_vdf(proof.input, &proof.params) else {
        return false;
    };
    expected.output == proof.output
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
///
/// # Panics
/// Panics if `params` are invalid. Prefer validating with [`VdfParams::is_valid`].
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
        "Production VDF: p={}, iterations (≈10 min target): {}",
        PRODUCTION_MODULUS, PRODUCTION_ITERATIONS
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
        assert!(!verify_vdf(start, 3, step2, p)); // wrong final_x
    }

    #[test]
    fn test_fifth_root_exponent_well_defined() {
        let params = VdfParams::for_tests(1);
        assert_eq!(params.modulus % 5, 3);
        assert!(params.fifth_root_exponent().is_some());
        assert!(VdfParams::production().fifth_root_exponent().is_some());

        // Classical closed form for p ≡ 3 (mod 5).
        let p = DEFAULT_TEST_MODULUS;
        let d = fifth_root_exponent_for(p).unwrap();
        assert_eq!(d, (2 * p - 1) / 5);
        // 5d ≡ 1 (mod p−1)
        assert_eq!((5u128 * d as u128) % (p as u128 - 1), 1);
    }

    #[test]
    fn test_production_modulus_classical_minroot() {
        let p = PRODUCTION_MODULUS;
        assert_eq!(p % 5, 3, "production modulus must be ≡ 3 (mod 5)");
        let d = fifth_root_exponent_for(p).expect("production modulus must admit d");
        assert_eq!(d, (2 * p as u128 - 1) as u64 / 5);
        assert_eq!((5u128 * d as u128) % (p as u128 - 1), 1);

        let prod = VdfParams::production();
        assert!(prod.is_valid());
        assert_eq!(prod.modulus, PRODUCTION_MODULUS);
        // Smoke: one step on production modulus is well-defined.
        let _ = compute_minroot_step(2, 1, p);
    }

    #[test]
    fn test_invalid_modulus_p_equiv_1_mod_5_rejected() {
        // p = 11 ≡ 1 (mod 5): gcd(5, 10) = 5 ≠ 1 — fifth root not unique.
        let bad = VdfParams {
            modulus: 11,
            iterations: 4,
        };
        assert!(!bad.is_valid());
        assert!(bad.fifth_root_exponent().is_none());
        assert!(fifth_root_exponent_for(11).is_none());
        assert!(try_compute_minroot_step(3, 1, 11).is_none());
        assert!(try_evaluate_vdf(3, &bad).is_none());

        // Forged proof with invalid modulus must not verify.
        let forged = VdfProof {
            params: bad.clone(),
            input: 3,
            output: 0,
            claimed_iterations: 4,
        };
        assert!(!verify_vdf_proof(&forged));
        assert!(!verify_vdf(3, 4, 0, 11));

        // Another p ≡ 1 (mod 5): 31.
        assert_eq!(31 % 5, 1);
        assert!(fifth_root_exponent_for(31).is_none());
        assert!(!VdfParams {
            modulus: 31,
            iterations: 2
        }
        .is_valid());
    }

    #[test]
    fn test_modulus_too_small_rejected() {
        for p in [0u64, 1, 2, 3, 4] {
            assert!(
                fifth_root_exponent_for(p).is_none(),
                "p={p} should be rejected"
            );
            assert!(!verify_vdf(1, 1, 1, p));
        }
    }

    #[test]
    fn test_fifth_root_for_p_equiv_2_or_4_mod_5() {
        // p = 7 ≡ 2 (mod 5): closed form (2p−1)/5 not integral, but inverse exists.
        assert_eq!(7 % 5, 2);
        let d = fifth_root_exponent_for(7).expect("p=7 should admit inverse of 5");
        assert_eq!((5u128 * d as u128) % 6, 1);
        // Step must use inverse, not a silent cube fallback.
        let x = try_compute_minroot_step(3, 1, 7).unwrap();
        let base = (3u64 + 1) % 7;
        assert_eq!(x, mod_exp(base, d, 7));

        // p = 19 ≡ 4 (mod 5).
        assert_eq!(19 % 5, 4);
        let d19 = fifth_root_exponent_for(19).unwrap();
        assert_eq!((5u128 * d19 as u128) % 18, 1);
        assert!(VdfParams {
            modulus: 19,
            iterations: 3
        }
        .is_valid());
        let proof = try_evaluate_vdf(5, &VdfParams {
            modulus: 19,
            iterations: 3,
        })
        .unwrap();
        assert!(verify_vdf_proof(&proof));
    }

    #[test]
    fn test_vdf_verify_accepts_honest_proof() {
        let params = VdfParams::for_tests(16);
        let proof = evaluate_vdf(7, &params);
        assert_eq!(proof.claimed_iterations, 16);
        assert!(verify_vdf_proof(&proof));
    }

    #[test]
    fn test_vdf_verify_rejects_wrong_final_x() {
        let params = VdfParams::for_tests(12);
        let mut proof = evaluate_vdf(99, &params);
        proof.output = proof.output.wrapping_add(1);
        assert!(!verify_vdf_proof(&proof));
        // Also via legacy API.
        assert!(!verify_vdf(99, 12, proof.output, params.modulus));
    }

    #[test]
    fn test_vdf_verify_rejects_wrong_steps() {
        let params = VdfParams::for_tests(10);
        let mut proof = evaluate_vdf(1, &params);
        // Under-claim delay (claimed_iterations mismatch).
        proof.claimed_iterations = 9;
        assert!(!verify_vdf_proof(&proof));

        // Over-claim with matching claimed_iterations but wrong params.iterations:
        // re-eval uses params.iterations, so output won't match a longer path.
        let short = evaluate_vdf(1, &VdfParams::for_tests(5));
        let long_params = VdfParams::for_tests(8);
        let forged = VdfProof {
            params: long_params,
            input: 1,
            output: short.output, // output of only 5 steps
            claimed_iterations: 8,
        };
        assert!(!verify_vdf_proof(&forged));
    }

    #[test]
    fn test_vdf_prove_verify_determinism() {
        let params = VdfParams::for_tests(24);
        let input = 12345u64;
        let a = evaluate_vdf(input, &params);
        let b = evaluate_vdf(input, &params);
        assert_eq!(a, b);
        assert!(verify_vdf_proof(&a));
        assert!(verify_vdf_proof(&b));
        // Re-verify is stable.
        assert_eq!(verify_vdf_proof(&a), verify_vdf_proof(&b));
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
    fn test_job_did_stability() {
        // Same (output, meta) always yields the same DID (byte-stable SHA-256).
        let out = 0xdead_beef_u64;
        let meta = b"stable-job-meta-v1";
        let d1 = mint_ephemeral_job_did(out, meta);
        let d2 = mint_ephemeral_job_did(out, meta);
        assert_eq!(d1, d2);
        assert_eq!(d1.as_bytes(), d2.as_bytes());

        // Encoding is big-endian output bytes || meta.
        let mut hasher = Sha256::new();
        hasher.update(out.to_be_bytes());
        hasher.update(meta);
        let expected: [u8; 32] = hasher.finalize().into();
        assert_eq!(d1.0, expected);

        // Endianness matters: little-endian would differ for multi-byte values.
        let mut hasher_le = Sha256::new();
        hasher_le.update(out.to_le_bytes());
        hasher_le.update(meta);
        let le: [u8; 32] = hasher_le.finalize().into();
        assert_ne!(d1.0, le);
    }

    #[test]
    fn test_production_iterations_documented() {
        // Guard the documented 10-minute production parameter surface.
        assert_eq!(PRODUCTION_ITERATIONS, 50_000_000);
        let prod = VdfParams::production();
        assert_eq!(prod.iterations, PRODUCTION_ITERATIONS);
        assert_eq!(prod.modulus, PRODUCTION_MODULUS);
        assert!(prod.iterations > 1_000_000);
        assert!(prod.is_valid());
    }
}
