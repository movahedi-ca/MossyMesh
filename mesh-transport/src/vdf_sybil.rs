//! Wesolowski verifiable delay function over RSA-2048 for Sybil resistance
//! and Ephemeral Job DID minting.
//!
//! # Why this construction
//!
//! The previous sequential map (MinRoot-style iterated fifth roots) had no
//! fast verification: rejecting a bogus proof cost the same as evaluating it
//! (about 10 CPU-minutes at production parameters), so an attacker could mint
//! random outputs at near-zero cost and force every verifier to burn the full
//! delay on each one. That is a CPU denial-of-service amplifier (issue #199).
//!
//! This module replaces it with the Wesolowski VDF (Benjamin Wesolowski,
//! "Efficient Verifiable Delay Functions", EUROCRYPT 2019):
//!
//! ```text
//! evaluate: y  = x^(2^t) mod N            (t sequential squarings)
//! prove:    pi = x^floor(2^t / l) mod N   (one extra sequential pass)
//! verify:   pi^l * x^(2^t mod l) = y (mod N)   (two exponentiations, no loop over t)
//! ```
//!
//! Verification never performs `t` sequential steps: its cost is two modular
//! exponentiations with ~128-bit exponents plus a 128-bit prime search,
//! measured in milliseconds and independent of `t`. A bogus proof is therefore
//! cheap to reject no matter how large the claimed iteration count is.
//!
//! # Group of unknown order: which N, and the trust assumption
//!
//! The VDF evaluates in the multiplicative group `(Z/NZ)*` where `N` is the
//! RSA-2048 challenge integer: the 617-digit semiprime published by RSA
//! Laboratories for the RSA Factoring Challenge (2001, US$200,000 prize; the
//! challenge was retired in 2007 with this number still unfactored, and it
//! remains unfactored today).
//!
//! Why this `N`, and what is being trusted:
//!
//! - Nothing up my sleeve. RSA Laboratories generated the challenge numbers
//!   on an offline computer and destroyed the hard drive afterwards, so no
//!   party is known to hold the factors. There is no setup ceremony to run,
//!   no parameters to generate, and no "generate the modulus, then forget the
//!   factors" honesty to take on faith from any individual.
//! - Security level. A 2048-bit RSA modulus carries roughly 112 bits of
//!   factoring security (NIST SP 800-57). The Fiat-Shamir challenge below is
//!   128 bits, so the construction as a whole targets the lower of the two:
//!   about 112-bit security. That matches the deployment point of other
//!   fielded Wesolowski VDFs and fits a delay gate, which protects rate
//!   rather than long-term confidentiality.
//!
//! Stated plainly, the trust assumption is: **factoring this `N` stays
//! infeasible**. Anyone who learns the factors can reduce the exponent `2^t`
//! modulo `phi(N)` and evaluate the VDF in `O(log t)` time, minting identity
//! proofs without burning the delay and collapsing the Sybil gate this module
//! provides. If this `N` is ever factored, or if 128-bit factoring security
//! is required, rotate to a fresh 3072-bit RSA modulus from a trusted
//! multi-party ceremony and register it under a new `MODULUS_ID_*` value; the
//! id registry exists so that rotation does not change the proof format.
//!
//! # Protocol detail
//!
//! Public inputs: 64-bit seed `input`, delay `t = params.iterations`.
//!
//! 1. `x = hash_to_group(input, t)`: SHA-256 in counter mode expands
//!    `DOMAIN || 0x01 || input_be || t_be || counter` to 2048 bits, reduced
//!    into `[2, N-1]`. Deterministic, and binds the group element to `t`.
//! 2. Evaluation: `y = x^(2^t) mod N` by `t` sequential squarings. Each step
//!    consumes the previous one, so this cannot be parallelized.
//! 3. Challenge: `l = H_prime(x, y, t)`, a 128-bit prime derived by
//!    Fiat-Shamir from SHA-256 over `(x, y, t)` with a counter incremented
//!    until Miller-Rabin (with trial division) accepts. Because `l` is hashed
//!    from the claimed `y`, a prover cannot grind a favorable challenge.
//! 4. Proof: `pi = x^floor(2^t / l) mod N`, computed in a second sequential
//!    pass by binary long division in the exponent (one squaring, plus one
//!    conditional multiply, per step). Total honest prover work is on the
//!    order of 2.5 sequential 2048-bit multiplications per step.
//! 5. Verification: recompute `x` and `l` (cheap), set `r = 2^t mod l` by fast
//!    modular exponentiation (`O(log t)` small-integer ops), then check
//!    `pi^l * x^r = y (mod N)`. There is no loop over `t` anywhere in this path.
//!
//! Soundness rests on the adaptive root assumption in a group of unknown
//! order (Wesolowski 2019, Section 3): from `(x, y, pi)` and a random 128-bit
//! prime `l`, passing the check without doing the sequential work implies
//! computing an `l`-th root of a chosen group element, which is assumed hard
//! without the factorization of `N`.
//!
//! # Production vs test parameters
//! | Constant | Value | Use |
//! | --- | --- | --- |
//! | [`PRODUCTION_ITERATIONS`] | 50_000_000 | delay target, see calibration note |
//! | [`DEFAULT_TEST_ITERATIONS`] | 16 | unit / integration tests only |
//! | [`MAX_TEST_ITERATIONS`] | 10_000 | hard cap for [`VdfParams::for_tests`] |
//!
//! Calibration note: at 50M iterations the prover performs roughly 125M
//! sequential 2048-bit modular multiplications. Measured single-squaring cost
//! is about 9us on reference x86-64 (release build, portable `num-bigint`
//! backend), which puts the full proof in the tens-of-minutes range there;
//! edge silicon will be slower. Re-benchmark `evaluate_vdf` on the fleet's
//! slowest device before locking consensus parameters: the iteration count
//! that lands near ~10 minutes there is the right production value, and this
//! constant must move to match it.
//!
//! Tests must never invoke [`PRODUCTION_ITERATIONS`] for wall-clock delay
//! (use [`VdfParams::for_tests`] / [`DEFAULT_TEST_ITERATIONS`]).
//!
//! # Ephemeral Job DID
//! ```text
//! JobDID = SHA-256( VDF_output_bytes || job_meta )
//! ```
//! where `VDF_output_bytes` is the 256-byte big-endian encoding of `y`.

use num_bigint::BigUint;
use sha2::{Digest, Sha256};
use std::sync::OnceLock;

/// Documented production iteration count.
///
/// **Do not use this constant in unit tests.** Prefer [`DEFAULT_TEST_ITERATIONS`] via
/// [`VdfParams::for_tests`]. See the module-level calibration note: re-benchmark
/// on the fleet's slowest device before locking consensus parameters.
pub const PRODUCTION_ITERATIONS: u64 = 50_000_000;

/// Registered group id for the RSA-2048 challenge modulus (the only
/// production group in this module). Kept as an id registry so a future
/// modulus rotation does not change the proof format.
pub const MODULUS_ID_RSA2048_CHALLENGE: u32 = 1;

/// Default sequential iterations for unit tests (orders of magnitude below production).
pub const DEFAULT_TEST_ITERATIONS: u64 = 16;

/// Hard upper bound for [`VdfParams::for_tests`] so accidental production-scale
/// iteration counts never enter the test path.
pub const MAX_TEST_ITERATIONS: u64 = 10_000;

/// Byte length of group elements (2048-bit modulus).
pub const VDF_MODULUS_BYTES: usize = 256;

/// Bit length of the Fiat-Shamir challenge prime `l`.
pub const CHALLENGE_PRIME_BITS: u32 = 128;

/// Domain separation for all hashes in this module.
const VDF_DOMAIN: &[u8] = b"mossymesh.vdf.wesolowski.v1";

/// RSA-2048 challenge modulus, hex, split into 64-char chunks to avoid a
/// single giant string literal.
const VDF_MODULUS_HEX: &str = concat!(
    "c7970ceedcc3b0754490201a7aa613cd73911081c790f5f1a8726f463550bb5b",
    "7ff0db8e1ea1189ec72f93d1650011bd721aeeacc2acde32a04107f0648c2813",
    "a31f5b0b7765ff8b44b4b6ffc93384b646eb09c7cf5e8592d40ea33c80039f35",
    "b4f14a04b51f7bfd781be4d1673164ba8eb991c2c4d730bbbe35f592bdef524a",
    "f7e8daefd26c66fc02c479af89d64d373f442709439de66ceb955f3ea37d5159",
    "f6135809f85334b5cb1813addc80cd05609f10ac6a95ad65872c909525bdad32",
    "bc729592642920f24c61dc5b3c3b7923e56b16a4d9d373d8721f24a3fc0f1b31",
    "31f55615172866bccc30f95054c824e733a5eb6817f7bc16399d48c6361cc7e5",
);

/// Parsed RSA-2048 challenge modulus, initialized once.
static VDF_MODULUS: OnceLock<BigUint> = OnceLock::new();

/// The VDF group modulus `N` (RSA-2048 challenge integer).
fn vdf_modulus() -> BigUint {
    VDF_MODULUS
        .get_or_init(|| {
            BigUint::parse_bytes(VDF_MODULUS_HEX.as_bytes(), 16)
                .expect("VDF_MODULUS_HEX must be valid hex")
        })
        .clone()
}

/// Resolve a registered group id to its modulus. Unregistered ids are
/// rejected so no weak or attacker-chosen group can enter the verify path.
fn resolve_group(modulus_id: u32) -> Option<BigUint> {
    match modulus_id {
        MODULUS_ID_RSA2048_CHALLENGE => Some(vdf_modulus()),
        _ => None,
    }
}

/// Fixed-size VDF input seed (hashed into the group; the group elements
/// themselves are 256-byte big-endian encodings).
pub type VdfState = u64;

/// Structured reasons a proof is rejected. Stable for logs / mesh diagnostics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VdfVerifyError {
    /// Group id is not registered (unknown / weak group).
    InvalidModulus,
    /// `claimed_iterations != params.iterations`.
    IterationMismatch,
    /// Zero-step "proofs" are never accepted.
    ZeroIterations,
    /// The Wesolowski equation failed, or a proof/output encoding was not a
    /// canonical group element (wrong length, zero, or `>= N`). Checked with
    /// cheap tests first, so this rejects in milliseconds.
    OutputMismatch,
}

impl VdfVerifyError {
    /// Stable machine-readable error code.
    pub fn code(self) -> &'static str {
        match self {
            VdfVerifyError::InvalidModulus => "INVALID_MODULUS",
            VdfVerifyError::IterationMismatch => "ITERATION_MISMATCH",
            VdfVerifyError::ZeroIterations => "ZERO_ITERATIONS",
            VdfVerifyError::OutputMismatch => "OUTPUT_MISMATCH",
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            VdfVerifyError::InvalidModulus => "VDF verify failed: unregistered group.",
            VdfVerifyError::IterationMismatch => "VDF verify failed: iteration claim mismatch.",
            VdfVerifyError::ZeroIterations => "VDF verify failed: zero iterations.",
            VdfVerifyError::OutputMismatch => "VDF verify failed: output mismatch.",
        }
    }
}

impl core::fmt::Display for VdfVerifyError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::error::Error for VdfVerifyError {}

/// Parameters for a Wesolowski VDF instance.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VdfParams {
    /// Registered group id ([`MODULUS_ID_RSA2048_CHALLENGE`]).
    pub modulus_id: u32,
    /// Number of sequential squarings. Production: [`PRODUCTION_ITERATIONS`].
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
            modulus_id: MODULUS_ID_RSA2048_CHALLENGE,
            iterations,
        }
    }

    /// Production parameters (RSA-2048 challenge group, full delay).
    pub fn production() -> Self {
        Self {
            modulus_id: MODULUS_ID_RSA2048_CHALLENGE,
            iterations: PRODUCTION_ITERATIONS,
        }
    }

    /// Validate group registration and non-zero iteration count.
    pub fn validate(&self) -> Result<(), VdfVerifyError> {
        if resolve_group(self.modulus_id).is_none() {
            return Err(VdfVerifyError::InvalidModulus);
        }
        if self.iterations == 0 {
            return Err(VdfVerifyError::ZeroIterations);
        }
        Ok(())
    }
}

impl Default for VdfParams {
    fn default() -> Self {
        Self::for_tests(DEFAULT_TEST_ITERATIONS)
    }
}

/// Public proof that a sequential delay of `params.iterations` squarings was performed.
///
/// `output` and `proof` are fixed [`VDF_MODULUS_BYTES`]-byte big-endian group
/// elements (`y` and Wesolowski `pi`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VdfProof {
    pub params: VdfParams,
    pub input: VdfState,
    pub output: Vec<u8>,
    pub proof: Vec<u8>,
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

/// Left-pad a group element to [`VDF_MODULUS_BYTES`] bytes big-endian.
fn pad_to_256(v: &BigUint) -> Vec<u8> {
    let mut bytes = v.to_bytes_be();
    if bytes.len() < VDF_MODULUS_BYTES {
        let mut padded = vec![0u8; VDF_MODULUS_BYTES - bytes.len()];
        padded.append(&mut bytes);
        return padded;
    }
    bytes
}

/// Map `(input, iterations)` into the group: SHA-256 counter mode to 2048
/// bits, reduced into `[2, N-1]`. Deterministic; binds the element to `t`.
fn hash_to_group(input: VdfState, iterations: u64, n: &BigUint) -> BigUint {
    let mut bytes = Vec::with_capacity(VDF_MODULUS_BYTES);
    for counter in 0..8u32 {
        let mut hasher = Sha256::new();
        hasher.update(VDF_DOMAIN);
        hasher.update([0x01]);
        hasher.update(input.to_be_bytes());
        hasher.update(iterations.to_be_bytes());
        hasher.update(counter.to_be_bytes());
        bytes.extend_from_slice(&hasher.finalize());
    }
    let v = BigUint::from_bytes_be(&bytes);
    let two = BigUint::from(2u32);
    (v % (n - &two)) + two
}

/// Fiat-Shamir challenge: first 128-bit prime at or after the SHA-256-derived
/// candidate from `(x, y, t)`. `x_be` / `y_be` must be [`VDF_MODULUS_BYTES`]
/// bytes so the hash input is unambiguous.
fn fiat_shamir_prime(x_be: &[u8], y_be: &[u8], iterations: u64) -> BigUint {
    let mut counter = 0u32;
    loop {
        let mut hasher = Sha256::new();
        hasher.update(VDF_DOMAIN);
        hasher.update([0x02]);
        hasher.update(x_be);
        hasher.update(y_be);
        hasher.update(iterations.to_be_bytes());
        hasher.update(counter.to_be_bytes());
        let digest = hasher.finalize();
        let mut candidate = BigUint::from_bytes_be(&digest[..16]);
        candidate.set_bit((CHALLENGE_PRIME_BITS - 1) as u64, true);
        candidate.set_bit(0, true);
        if is_probable_prime(&candidate) {
            return candidate;
        }
        counter = counter.wrapping_add(1);
    }
}

/// Small primes below 1000 for trial division.
const SMALL_PRIMES: &[u64] = &[
    3, 5, 7, 11, 13, 17, 19, 23, 29, 31, 37, 41, 43, 47, 53, 59, 61, 67, 71, 73, 79, 83, 89, 97,
    101, 103, 107, 109, 113, 127, 131, 137, 139, 149, 151, 157, 163, 167, 173, 179, 181, 191, 193,
    197, 199, 211, 223, 227, 229, 233, 239, 241, 251, 257, 263, 269, 271, 277, 281, 283, 293, 307,
    311, 313, 317, 331, 337, 347, 349, 353, 359, 367, 373, 379, 383, 389, 397, 401, 409, 419, 421,
    431, 433, 439, 443, 449, 457, 461, 463, 467, 479, 487, 491, 499, 503, 509, 521, 523, 541, 547,
    557, 563, 569, 571, 577, 587, 593, 599, 601, 607, 613, 617, 619, 631, 641, 643, 647, 653, 659,
    661, 673, 677, 683, 691, 701, 709, 719, 727, 733, 739, 743, 751, 757, 761, 769, 773, 787, 797,
    809, 811, 821, 823, 827, 829, 839, 853, 857, 859, 863, 877, 881, 883, 887, 907, 911, 919, 929,
    937, 941, 947, 953, 967, 971, 977, 983, 991, 997,
];

/// Miller-Rabin bases (first 12 primes) for the challenge-prime search.
const MR_BASES: &[u64] = &[2, 3, 5, 7, 11, 13, 17, 19, 23, 29, 31, 37];

/// Probable-prime test for the 128-bit Fiat-Shamir candidate: trial division
/// by small primes, then 12 Miller-Rabin rounds with fixed prime bases.
/// The candidate always exceeds 1000 with its top bit set, so a zero remainder
/// means composite (never equality with a trial divisor).
fn is_probable_prime(n: &BigUint) -> bool {
    for &p in SMALL_PRIMES {
        if n % p == BigUint::from(0u32) {
            return false;
        }
    }
    // Write n - 1 = d * 2^s with d odd.
    let one = BigUint::from(1u32);
    let mut d = n - &one;
    let mut s = 0u32;
    while !d.bit(0) {
        d >>= 1;
        s += 1;
    }
    let n_minus_1 = n - &one;
    'witness: for &a in MR_BASES {
        let a = BigUint::from(a);
        if a >= *n {
            continue;
        }
        let mut x = a.modpow(&d, n);
        if x == one || x == n_minus_1 {
            continue 'witness;
        }
        for _ in 1..s {
            x = (&x * &x) % n;
            if x == n_minus_1 {
                continue 'witness;
            }
        }
        return false;
    }
    true
}

/// Evaluate the Wesolowski VDF: `y = x^(2^iterations) mod n` plus the proof
/// `pi = x^floor(2^iterations / l) mod n`, where `l` is the Fiat-Shamir
/// challenge over the finished `y` (so the proof pass necessarily runs after
/// the evaluation pass). Returns `(y_be, pi_be)`.
fn evaluate_wesolowski(input: VdfState, iterations: u64, n: &BigUint) -> (Vec<u8>, Vec<u8>) {
    let x = hash_to_group(input, iterations, n);
    // Pass 1: y = x^(2^iterations) by sequential squaring.
    let mut y = x.clone();
    for _ in 0..iterations {
        y = (&y * &y) % n;
    }
    let y_be = pad_to_256(&y);
    // Challenge binds the claimed output; the prover cannot grind it.
    let ell = fiat_shamir_prime(&pad_to_256(&x), &y_be, iterations);
    // Pass 2: pi = x^floor(2^iterations / ell) by binary long division in the
    // exponent. Invariant after step i: pi = x^floor(2^i / ell), r = 2^i mod ell.
    let mut pi = BigUint::from(1u32);
    let mut r = BigUint::from(1u32);
    for _ in 0..iterations {
        let two_r = &r + &r;
        pi = (&pi * &pi) % n;
        if two_r >= ell {
            pi = (&pi * &x) % n;
        }
        r = two_r % &ell;
    }
    (y_be, pad_to_256(&pi))
}

/// Evaluate the full sequential VDF and produce its proof.
///
/// Each squaring depends on the previous state: this loop cannot be
/// parallelized. The proof pass roughly doubles the sequential work, which is
/// the honest prover's delay cost.
pub fn evaluate_vdf(input: VdfState, params: &VdfParams) -> VdfProof {
    evaluate_vdf_checked(input, params).expect(
        "evaluate_vdf requires valid params; use evaluate_vdf_checked to handle invalid params",
    )
}

/// Evaluate only when parameters are acceptable (registered group, non-zero delay).
pub fn evaluate_vdf_checked(
    input: VdfState,
    params: &VdfParams,
) -> Result<VdfProof, VdfVerifyError> {
    params.validate()?;
    let n = resolve_group(params.modulus_id).ok_or(VdfVerifyError::InvalidModulus)?;
    let (output, proof) = evaluate_wesolowski(input, params.iterations, &n);
    Ok(VdfProof {
        params: params.clone(),
        input,
        output,
        proof,
        claimed_iterations: params.iterations,
    })
}

/// Verify a VDF proof with fast verification (detailed errors).
///
/// Cost is independent of the iteration count: after cheap structural checks,
/// the work is one 128-bit prime search and two modular exponentiations
/// (milliseconds), never `t` sequential squarings. Bogus proofs are therefore
/// cheap to reject.
pub fn verify_vdf_proof_detailed(proof: &VdfProof) -> Result<(), VdfVerifyError> {
    // Cheap structural checks first: no big-integer work happens before these.
    if proof.params.iterations == 0 || proof.claimed_iterations == 0 {
        return Err(VdfVerifyError::ZeroIterations);
    }
    if proof.claimed_iterations != proof.params.iterations {
        return Err(VdfVerifyError::IterationMismatch);
    }
    let n = resolve_group(proof.params.modulus_id).ok_or(VdfVerifyError::InvalidModulus)?;
    // Encodings must be canonical group elements before any arithmetic.
    if proof.output.len() != VDF_MODULUS_BYTES || proof.proof.len() != VDF_MODULUS_BYTES {
        return Err(VdfVerifyError::OutputMismatch);
    }
    let y = BigUint::from_bytes_be(&proof.output);
    let pi = BigUint::from_bytes_be(&proof.proof);
    let one = BigUint::from(1u32);
    if y < one || y >= n || pi < one || pi >= n {
        return Err(VdfVerifyError::OutputMismatch);
    }
    // Recompute the input element and the Fiat-Shamir challenge from the
    // *claimed* output, then check the Wesolowski equation:
    // pi^l * x^(2^t mod l) = y (mod n).
    let x = hash_to_group(proof.input, proof.params.iterations, &n);
    let ell = fiat_shamir_prime(&pad_to_256(&x), &proof.output, proof.params.iterations);
    let r = BigUint::from(2u32).modpow(&BigUint::from(proof.params.iterations), &ell);
    let lhs = (pi.modpow(&ell, &n) * x.modpow(&r, &n)) % &n;
    if lhs != y {
        return Err(VdfVerifyError::OutputMismatch);
    }
    Ok(())
}

/// Verify a VDF proof with fast verification.
///
/// Returns `true` iff the group is registered, the iteration count matches
/// parameters, and the Wesolowski equation holds. Never loops over the
/// iteration count.
pub fn verify_vdf_proof(proof: &VdfProof) -> bool {
    verify_vdf_proof_detailed(proof).is_ok()
}

/// Mint an Ephemeral Job DID:
/// `JobDID = SHA-256(VDF_output_bytes || job_meta)`.
///
/// `vdf_output` is the 256-byte big-endian VDF output. Sybil resistance comes
/// from the sequential VDF burn required to produce a valid proof for it.
pub fn mint_ephemeral_job_did(vdf_output: &[u8], job_meta: &[u8]) -> EphemeralJobDid {
    let mut hasher = Sha256::new();
    hasher.update(vdf_output);
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
    let did = mint_ephemeral_job_did(&proof.output, job_meta);
    (proof, did)
}

pub fn init_vdf_sybil() {
    println!("Initializing VDF Sybil protection (Wesolowski VDF over RSA-2048).");
    println!(
        "Production VDF iterations: {} (group: RSA-2048 challenge modulus)",
        PRODUCTION_ITERATIONS
    );
    println!(
        "Test VDF defaults: iterations={} (max {}), modulus_id={}",
        DEFAULT_TEST_ITERATIONS, MAX_TEST_ITERATIONS, MODULUS_ID_RSA2048_CHALLENGE
    );
    let params = VdfParams::for_tests(3);
    let start = 42;
    let proof = evaluate_vdf(start, &params);
    let prefix: String = proof
        .output
        .iter()
        .take(8)
        .map(|b| format!("{b:02x}"))
        .collect();
    println!(
        "VDF eval (iters={}): input={} -> y=0x{}... (verified={})",
        params.iterations,
        start,
        prefix,
        verify_vdf_proof(&proof)
    );
    let did = mint_ephemeral_job_did(&proof.output, b"demo-job-meta");
    println!("Ephemeral Job DID (hex prefix): {:02x?}", &did.0[..8]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn production_modulus_is_rsa2048_challenge() {
        let n = vdf_modulus();
        assert_eq!(n.bits(), 2048);
        // Cross-checked decimal expansion: Wikipedia "RSA numbers" and the
        // MTC3/RSA Inc challenge PDF agree on all 617 digits. This pins the
        // constant against transcription corruption.
        let dec_expected = "25195908475657893494027183240048398571429282126204032027777137836043662020707595556264018525880784406918290641249515082189298559149176184502808489120072844992687392807287776735971418347270261896375014971824691165077613379859095700097330459748808428401797429100642458691817195118746121515172654632282216869987549182422433637259085141865462043576798423387184774447920739934236584823824281198163815010674810451660377306056201619676256133844143603833904414952634432190114657544454178424020924616515723350778707749817125772467962926386356373289912154831438167899885040445364023527381951378636564391212010397122822120720357";
        assert_eq!(n.to_str_radix(10), dec_expected);
    }

    #[test]
    fn hash_to_group_is_deterministic_and_in_range() {
        let n = vdf_modulus();
        let two = BigUint::from(2u32);
        let a = hash_to_group(42, 16, &n);
        let b = hash_to_group(42, 16, &n);
        assert_eq!(a, b);
        assert!(a >= two && a < n);
        // Binds the delay and the seed.
        assert_ne!(a, hash_to_group(42, 17, &n));
        assert_ne!(a, hash_to_group(43, 16, &n));
    }

    #[test]
    fn fiat_shamir_prime_is_128bit_prime_and_deterministic() {
        let n = vdf_modulus();
        let params = VdfParams::for_tests(16);
        let proof = evaluate_vdf(7, &params);
        let x = hash_to_group(7, params.iterations, &n);
        let x_be = pad_to_256(&x);
        let l1 = fiat_shamir_prime(&x_be, &proof.output, params.iterations);
        let l2 = fiat_shamir_prime(&x_be, &proof.output, params.iterations);
        assert_eq!(l1, l2);
        assert_eq!(l1.bits(), CHALLENGE_PRIME_BITS as u64);
        assert!(is_probable_prime(&l1));
        // Binds the claimed output: a different y gives (overwhelmingly) a different l.
        let mut other_y = proof.output.clone();
        other_y[0] ^= 0x01;
        assert_ne!(l1, fiat_shamir_prime(&x_be, &other_y, params.iterations));
    }

    #[test]
    fn vdf_verify_accepts_honest_proof() {
        let params = VdfParams::for_tests(DEFAULT_TEST_ITERATIONS);
        let proof = evaluate_vdf(7, &params);
        assert_eq!(proof.claimed_iterations, DEFAULT_TEST_ITERATIONS);
        assert_eq!(proof.output.len(), VDF_MODULUS_BYTES);
        assert_eq!(proof.proof.len(), VDF_MODULUS_BYTES);
        assert!(verify_vdf_proof(&proof));
        assert!(verify_vdf_proof_detailed(&proof).is_ok());
    }

    #[test]
    fn evaluate_is_deterministic() {
        let params = VdfParams::for_tests(24);
        let a = evaluate_vdf(1234, &params);
        let b = evaluate_vdf(1234, &params);
        assert_eq!(a, b);
    }

    #[test]
    fn vdf_verify_rejects_tampered_output() {
        let params = VdfParams::for_tests(12);
        let mut proof = evaluate_vdf(99, &params);
        let last = proof.output.len() - 1;
        proof.output[last] ^= 0x01;
        assert!(!verify_vdf_proof(&proof));
        assert_eq!(
            verify_vdf_proof_detailed(&proof).unwrap_err(),
            VdfVerifyError::OutputMismatch
        );
        assert_eq!(VdfVerifyError::OutputMismatch.code(), "OUTPUT_MISMATCH");
    }

    #[test]
    fn vdf_verify_rejects_mutated_proof() {
        let params = VdfParams::for_tests(64);
        let mut proof = evaluate_vdf(99, &params);
        let last = proof.proof.len() - 1;
        proof.proof[last] ^= 0x01;
        assert!(!verify_vdf_proof(&proof));
        assert_eq!(
            verify_vdf_proof_detailed(&proof).unwrap_err(),
            VdfVerifyError::OutputMismatch
        );
    }

    #[test]
    fn vdf_verify_rejects_malformed_encoding() {
        // Wrong length.
        let params = VdfParams::for_tests(16);
        let mut proof = evaluate_vdf(5, &params);
        proof.proof = vec![0u8; 32];
        assert_eq!(
            verify_vdf_proof_detailed(&proof).unwrap_err(),
            VdfVerifyError::OutputMismatch
        );
        // Output >= N (top byte 0xff exceeds N's top byte 0xc7).
        let mut proof = evaluate_vdf(5, &params);
        proof.output = vec![0xffu8; VDF_MODULUS_BYTES];
        assert_eq!(
            verify_vdf_proof_detailed(&proof).unwrap_err(),
            VdfVerifyError::OutputMismatch
        );
        // Zero is not a group element.
        let mut proof = evaluate_vdf(5, &params);
        proof.proof = vec![0u8; VDF_MODULUS_BYTES];
        assert_eq!(
            verify_vdf_proof_detailed(&proof).unwrap_err(),
            VdfVerifyError::OutputMismatch
        );
    }

    #[test]
    fn vdf_verify_rejects_iteration_mismatch() {
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
    fn vdf_verify_rejects_zero_iterations() {
        let proof = VdfProof {
            params: VdfParams {
                modulus_id: MODULUS_ID_RSA2048_CHALLENGE,
                iterations: 0,
            },
            input: 1,
            output: vec![1u8; VDF_MODULUS_BYTES],
            proof: vec![1u8; VDF_MODULUS_BYTES],
            claimed_iterations: 0,
        };
        assert_eq!(
            verify_vdf_proof_detailed(&proof).unwrap_err(),
            VdfVerifyError::ZeroIterations
        );
    }

    #[test]
    fn vdf_verify_rejects_unknown_modulus_id() {
        let params = VdfParams {
            modulus_id: 999,
            iterations: 16,
        };
        assert_eq!(
            params.validate().unwrap_err(),
            VdfVerifyError::InvalidModulus
        );
        let proof = VdfProof {
            params,
            input: 1,
            output: vec![1u8; VDF_MODULUS_BYTES],
            proof: vec![1u8; VDF_MODULUS_BYTES],
            claimed_iterations: 16,
        };
        assert_eq!(
            verify_vdf_proof_detailed(&proof).unwrap_err(),
            VdfVerifyError::InvalidModulus
        );
        assert_eq!(VdfVerifyError::InvalidModulus.code(), "INVALID_MODULUS");
    }

    #[test]
    fn garbage_proof_rejected_in_fraction_of_eval_time() {
        // Issue #199: bogus proofs must be cheap to reject. Rejection cost
        // must not scale with the claimed iteration count.
        let params = VdfParams::for_tests(8_000);
        let start = Instant::now();
        let _honest = evaluate_vdf(7, &params);
        let eval_time = start.elapsed();

        // Deterministic garbage: decodes below N (top byte 0x5a < 0xc7) but
        // does not satisfy the Wesolowski equation.
        let garbage = VdfProof {
            params: params.clone(),
            input: 7,
            output: vec![0x5au8; VDF_MODULUS_BYTES],
            proof: vec![0xa5u8; VDF_MODULUS_BYTES],
            claimed_iterations: params.iterations,
        };
        let start = Instant::now();
        let err = verify_vdf_proof_detailed(&garbage).unwrap_err();
        let reject_time = start.elapsed();
        assert_eq!(err, VdfVerifyError::OutputMismatch);
        assert!(
            reject_time * 5 < eval_time,
            "garbage rejection ({reject_time:?}) must be a fraction of eval ({eval_time:?})"
        );
    }

    #[test]
    fn verify_is_a_small_fraction_of_evaluate() {
        // Regression guard for issue #199: verification must never re-run the
        // sequential steps. If verify loops over t again, verify time will
        // approach eval time and this assertion fails.
        let params = VdfParams::for_tests(MAX_TEST_ITERATIONS);
        let start = Instant::now();
        let proof = evaluate_vdf(7, &params);
        let eval_time = start.elapsed();
        assert!(
            eval_time >= Duration::from_millis(50),
            "eval at MAX_TEST_ITERATIONS must take clearly measurable wall-clock time, got {eval_time:?}"
        );
        let start = Instant::now();
        verify_vdf_proof_detailed(&proof).expect("honest proof verifies");
        let verify_time = start.elapsed();
        assert!(
            verify_time * 8 < eval_time,
            "verify ({verify_time:?}) must take a small fraction of eval ({eval_time:?})"
        );
    }

    #[test]
    fn error_codes_stable() {
        assert_eq!(VdfVerifyError::InvalidModulus.code(), "INVALID_MODULUS");
        assert_eq!(
            VdfVerifyError::IterationMismatch.code(),
            "ITERATION_MISMATCH"
        );
        assert_eq!(VdfVerifyError::ZeroIterations.code(), "ZERO_ITERATIONS");
        assert_eq!(VdfVerifyError::OutputMismatch.code(), "OUTPUT_MISMATCH");
    }

    #[test]
    fn ephemeral_job_did_binds_output_and_meta() {
        let params = VdfParams::for_tests(8);
        let (proof, did) = mint_job_did_with_vdf(5, &params, b"job-A");
        assert!(verify_vdf_proof(&proof));

        let did_same = mint_ephemeral_job_did(&proof.output, b"job-A");
        assert_eq!(did, did_same);

        let did_other_meta = mint_ephemeral_job_did(&proof.output, b"job-B");
        assert_ne!(did, did_other_meta);

        let other_proof = evaluate_vdf(6, &params);
        let did_other_vdf = mint_ephemeral_job_did(&other_proof.output, b"job-A");
        assert_ne!(did, did_other_vdf);
    }

    #[test]
    fn production_params_shape() {
        // Guard the documented production parameter surface.
        assert_eq!(PRODUCTION_ITERATIONS, 50_000_000);
        let prod = VdfParams::production();
        assert_eq!(prod.iterations, PRODUCTION_ITERATIONS);
        assert_eq!(prod.modulus_id, MODULUS_ID_RSA2048_CHALLENGE);
        assert!(prod.validate().is_ok());
        assert!(prod.iterations > 1_000_000);

        // Test path is orders of magnitude smaller and capped.
        const {
            assert!(DEFAULT_TEST_ITERATIONS < MAX_TEST_ITERATIONS);
            assert!(MAX_TEST_ITERATIONS < PRODUCTION_ITERATIONS);
        }
        let t = VdfParams::for_tests(DEFAULT_TEST_ITERATIONS);
        assert_eq!(t.iterations, DEFAULT_TEST_ITERATIONS);
        assert_ne!(t.iterations, PRODUCTION_ITERATIONS);

        // for_tests clamps runaway iteration requests.
        let huge = VdfParams::for_tests(PRODUCTION_ITERATIONS);
        assert_eq!(huge.iterations, MAX_TEST_ITERATIONS);
        assert!(huge.iterations < PRODUCTION_ITERATIONS);
    }

    #[test]
    fn evaluate_checked_rejects_bad_params() {
        let bad = VdfParams {
            modulus_id: 77,
            iterations: 4,
        };
        assert_eq!(
            evaluate_vdf_checked(1, &bad).unwrap_err(),
            VdfVerifyError::InvalidModulus
        );
        let zero = VdfParams {
            modulus_id: MODULUS_ID_RSA2048_CHALLENGE,
            iterations: 0,
        };
        assert_eq!(
            evaluate_vdf_checked(1, &zero).unwrap_err(),
            VdfVerifyError::ZeroIterations
        );
        let good = VdfParams::for_tests(4);
        let proof = evaluate_vdf_checked(3, &good).unwrap();
        assert!(verify_vdf_proof(&proof));
    }

    #[test]
    fn sandbox_verifier_agrees_with_transport_proofs() {
        // The sandbox admit gate duplicates this module's Wesolowski
        // arithmetic (it cannot depend on mesh-transport without a dependency
        // cycle). This test pins the duplication: proofs issued by the
        // transport evaluator must admit through the sandbox verifier, and
        // sandbox-issued receipts must verify here.
        use sandbox::VdfVerifier as _;
        assert_eq!(
            sandbox::MODULUS_ID_WESOLOWSKI_RSA2048,
            MODULUS_ID_RSA2048_CHALLENGE,
            "group id registry must agree across crates"
        );
        let params = VdfParams::for_tests(32);
        let proof = evaluate_vdf(2026, &params);
        let verifier = sandbox::WesolowskiVdfVerifier::for_tests(8);
        let meta = b"cross-crate-consistency";
        let receipt = sandbox::VdfReceipt {
            start_x: proof.input,
            steps: proof.claimed_iterations,
            final_x: proof.output.clone(),
            proof: proof.proof.clone(),
            modulus_id: sandbox::MODULUS_ID_WESOLOWSKI_RSA2048,
            job_meta: meta.to_vec(),
            job_did: sandbox::WesolowskiVdfVerifier::mint_job_did(&proof.output, meta),
        };
        let did =
            sandbox::admit_job(&receipt, &verifier).expect("sandbox must admit transport proof");
        assert_eq!(did, receipt.job_did);

        // Reverse direction: sandbox-issued receipt verifies in transport.
        let receipt = verifier.issue_test(77, 24, meta).expect("issue");
        let proof = VdfProof {
            params: VdfParams {
                modulus_id: MODULUS_ID_RSA2048_CHALLENGE,
                iterations: receipt.steps,
            },
            input: receipt.start_x,
            output: receipt.final_x.clone(),
            proof: receipt.proof.clone(),
            claimed_iterations: receipt.steps,
        };
        assert!(verify_vdf_proof(&proof));
    }
}
