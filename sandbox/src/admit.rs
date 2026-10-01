//! Ephemeral Job DID admission: VDF-backed anti-spam gate for the sandbox admit path.
//!
//! # Background
//!
//! A job enters the sandbox with a [`VdfReceipt`]: a self-certifying receipt
//! binding a 64-bit seed to a VDF output after `steps` sequential squarings.
//! [`admit_job`] verifies the receipt against the configured [`VdfVerifier`}
//! and returns the deterministic Ephemeral Job DID. Admission is "verify
//! cheaply, burn work once": minting costs the prover `steps` sequential
//! squarings, verification costs milliseconds.
//!
//! # VDF construction (Wesolowski over RSA-2048)
//!
//! The receipt math mirrors `mesh-transport::vdf_sybil` exactly: the sandbox
//! cannot depend on mesh-transport (that would be a circular dependency), so
//! the Wesolowski arithmetic is duplicated here and the duplication is pinned
//! by a cross-crate consistency test in `mesh-transport` (transport-issued
//! proofs admit through this verifier and sandbox-issued receipts verify in
//! transport). Byte-exact spec shared by both crates:
//!
//! - Domain: `b"mossymesh.vdf.wesolowski.v1"`.
//! - `hash_to_group(input, t)`: 8x SHA-256(`domain || 0x01 || input_be8 ||
//!   t_be8 || counter_be4`), 2048 bits, reduced to `[2, N-1]`.
//! - Fiat-Shamir: SHA-256(`domain || 0x02 || x_be256 || y_be256 || t_be8 ||
//!   counter_be4`); first 16 bytes with bits 127 and 0 set, incremented until
//!   a 128-bit prime (trial division below 1000, then 12 Miller-Rabin rounds
//!   with fixed prime bases).
//! - Verify: `pi^l * x^(2^t mod l) = y (mod N)`; no loop over `t`.
//! - DID: `SHA-256(y_be256 || job_meta)`.
//!
//! The group modulus is the RSA-2048 challenge integer (617-digit RSA Labs
//! semiprime). The trust assumption: factoring it stays infeasible. Anyone
//! who learns the factors evaluates the VDF in `O(log t)`, collapsing the
//! Sybil gate. See `mesh-transport::vdf_sybil` for the full rationale.
//!
//! # Test vs production parameters
//!
//! | Constant | Value | Use |
//! | --- | --- | --- |
//! | [`PRODUCTION_ITERATIONS`] | 50_000_000 | delay target |
//! | [`MOBILE_ITERATIONS`] | 25_000_000 | battery-constrained provers |
//! | [`DEFAULT_TEST_ITERATIONS`] | 16 | unit / integration tests only |
//! | [`MAX_TEST_ITERATIONS`] | 10_000 | hard cap for `for_tests` verifier |
//!
//! Policy knobs on [`WesolowskiVdfVerifier`]:
//! - `min_steps`: reject receipts below this many steps (Sybil floor).
//! - `required_modulus_id`: when `Some`, only receipts minted under that
//!   registered group are admitted.

use num_bigint::BigUint;
use sha2::{Digest, Sha256};
use std::sync::OnceLock;

/// Documented production iteration count (see module-level parameter table).
/// Tests must use [`DEFAULT_TEST_ITERATIONS`] instead.
pub const PRODUCTION_ITERATIONS: u64 = 50_000_000;

/// Reduced iteration count for battery-constrained provers.
pub const MOBILE_ITERATIONS: u64 = 25_000_000;

/// Test-friendly default iteration count (orders of magnitude below production).
pub const DEFAULT_TEST_ITERATIONS: u64 = 16;

/// Hard upper bound for test iteration counts so runaway requests never enter
/// the test path.
pub const MAX_TEST_ITERATIONS: u64 = 10_000;

/// Byte length of group elements (2048-bit modulus).
pub const VDF_MODULUS_BYTES: usize = 256;

/// Registered group id for the RSA-2048 challenge modulus (the only
/// production group). Must equal `mesh-transport`'s
/// `MODULUS_ID_RSA2048_CHALLENGE`; the cross-crate consistency test pins this.
pub const MODULUS_ID_WESOLOWSKI_RSA2048: u32 = 1;

/// Domain separation shared with `mesh-transport::vdf_sybil`.
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

static VDF_MODULUS: OnceLock<BigUint> = OnceLock::new();

fn vdf_modulus() -> BigUint {
    VDF_MODULUS
        .get_or_init(|| {
            BigUint::parse_bytes(VDF_MODULUS_HEX.as_bytes(), 16)
                .expect("VDF_MODULUS_HEX must be valid hex")
        })
        .clone()
}

/// Ephemeral Job DID: `SHA-256(VDF_output_bytes || job_meta)`, 32 bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct JobDid(pub [u8; 32]);

impl JobDid {
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
    pub fn to_hex(&self) -> String {
        self.0.iter().map(|b| format!("{b:02x}")).collect()
    }
}

/// Self-certifying VDF receipt presented to the admit gate.
///
/// `final_x` and `proof` are fixed [`VDF_MODULUS_BYTES`]-byte big-endian group
/// elements (Wesolowski `y` and `pi`). `job_did` is the deterministic DID the
/// receipt claims; admission recomputes and compares it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VdfReceipt {
    /// 64-bit VDF input seed.
    pub start_x: u64,
    /// Claimed sequential squaring count.
    pub steps: u64,
    /// Claimed VDF output `y` (256-byte big-endian).
    pub final_x: Vec<u8>,
    /// Wesolowski proof `pi` (256-byte big-endian).
    pub proof: Vec<u8>,
    /// Registered group id (must be [`MODULUS_ID_WESOLOWSKI_RSA2048`] when a
    /// verifier pins it).
    pub modulus_id: u32,
    /// Opaque job metadata bound into the DID.
    pub job_meta: Vec<u8>,
    /// Claimed Ephemeral Job DID.
    pub job_did: JobDid,
}

/// Reasons the admit gate rejects a receipt. Stable for logs / diagnostics.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum AdmitError {
    /// No VDF receipt was supplied (admit requires a proof).
    MissingVdf,
    /// Proof / equation failed, or a proof/output encoding was not a canonical
    /// group element. Rejects in milliseconds; never scales with `steps`.
    InvalidVdf,
    /// Claimed Job DID does not match mint from `(final_x, job_meta)`.
    DidMismatch,
    /// Claimed iteration count below the verifier's `min_steps`.
    InsufficientSteps,
    /// Group id not registered (unknown / weak group).
    InvalidModulus,
}

impl AdmitError {
    /// Stable machine-readable error code.
    pub fn code(self) -> &'static str {
        match self {
            AdmitError::MissingVdf => "MISSING_VDF",
            AdmitError::InvalidVdf => "INVALID_VDF",
            AdmitError::DidMismatch => "DID_MISMATCH",
            AdmitError::InsufficientSteps => "INSUFFICIENT_STEPS",
            AdmitError::InvalidModulus => "INVALID_MODULUS",
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            AdmitError::MissingVdf => "admit rejected: missing VDF proof / receipt",
            AdmitError::InvalidVdf => "admit rejected: invalid VDF proof or output",
            AdmitError::DidMismatch => "admit rejected: Job DID does not match VDF receipt",
            AdmitError::InsufficientSteps => "admit rejected: insufficient VDF steps",
            AdmitError::InvalidModulus => "admit rejected: unregistered VDF group",
        }
    }
}

impl core::fmt::Display for AdmitError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::error::Error for AdmitError {}

fn pad_to_256(v: &BigUint) -> Vec<u8> {
    let mut bytes = v.to_bytes_be();
    if bytes.len() < VDF_MODULUS_BYTES {
        let mut padded = vec![0u8; VDF_MODULUS_BYTES - bytes.len()];
        padded.append(&mut bytes);
        return padded;
    }
    bytes
}

fn hash_to_group(input: u64, iterations: u64, n: &BigUint) -> BigUint {
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
        candidate.set_bit(127, true);
        candidate.set_bit(0, true);
        if is_probable_prime(&candidate) {
            return candidate;
        }
        counter = counter.wrapping_add(1);
    }
}

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

const MR_BASES: &[u64] = &[2, 3, 5, 7, 11, 13, 17, 19, 23, 29, 31, 37];

fn is_probable_prime(n: &BigUint) -> bool {
    for &p in SMALL_PRIMES {
        if n % p == BigUint::from(0u32) {
            return false;
        }
    }
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

/// Evaluate the Wesolowski VDF: `(y, pi)` for seed `input` and delay `steps`.
/// Returns `None` for zero steps.
fn evaluate_wesolowski(input: u64, steps: u64, n: &BigUint) -> Option<(Vec<u8>, Vec<u8>)> {
    if steps == 0 {
        return None;
    }
    let x = hash_to_group(input, steps, n);
    let mut y = x.clone();
    for _ in 0..steps {
        y = (&y * &y) % n;
    }
    let y_be = pad_to_256(&y);
    let ell = fiat_shamir_prime(&pad_to_256(&x), &y_be, steps);
    let mut pi = BigUint::from(1u32);
    let mut r = BigUint::from(1u32);
    for _ in 0..steps {
        let two_r = &r + &r;
        pi = (&pi * &pi) % n;
        if two_r >= ell {
            pi = (&pi * &x) % n;
        }
        r = two_r % &ell;
    }
    Some((y_be, pad_to_256(&pi)))
}

/// Policy interface for the admit gate.
pub trait VdfVerifier {
    /// Minimum accepted sequential step count.
    fn min_steps(&self) -> u64;
    /// Required registered group id, or `None` to accept any registered group.
    fn required_modulus_id(&self) -> Option<u32>;
    /// Verify a receipt's VDF proof (fast verification; cost independent of `steps`).
    fn verify_receipt(&self, receipt: &VdfReceipt) -> Result<(), AdmitError>;
    /// Deterministic Ephemeral Job DID: `SHA-256(final_x || job_meta)`.
    fn mint_job_did(final_x: &[u8], job_meta: &[u8]) -> JobDid {
        let mut hasher = Sha256::new();
        hasher.update(final_x);
        hasher.update(job_meta);
        let digest = hasher.finalize();
        let mut out = [0u8; 32];
        out.copy_from_slice(&digest);
        JobDid(out)
    }
}

/// Policy for the production Wesolowski VDF admission.
#[derive(Clone, Debug)]
pub struct WesolowskiVdfVerifier {
    /// Minimum accepted step count.
    pub min_steps: u64,
    /// Required group id when `Some` ([`MODULUS_ID_WESOLOWSKI_RSA2048`]).
    pub required_modulus_id: Option<u32>,
}

impl WesolowskiVdfVerifier {
    pub fn new(min_steps: u64, required_modulus_id: Option<u32>) -> Self {
        Self {
            min_steps,
            required_modulus_id,
        }
    }

    /// Production policy: full delay, RSA-2048 group pinned.
    pub fn production() -> Self {
        Self::new(PRODUCTION_ITERATIONS, Some(MODULUS_ID_WESOLOWSKI_RSA2048))
    }

    /// Battery-constrained policy.
    pub fn mobile() -> Self {
        Self::new(MOBILE_ITERATIONS, Some(MODULUS_ID_WESOLOWSKI_RSA2048))
    }

    /// Test policy: small delays, RSA-2048 group pinned, minimum steps
    /// configurable (clamped to `1..=MAX_TEST_ITERATIONS`).
    pub fn for_tests(min_steps: u64) -> Self {
        Self::new(
            min_steps.clamp(1, MAX_TEST_ITERATIONS),
            Some(MODULUS_ID_WESOLOWSKI_RSA2048),
        )
    }

    /// Mint a test receipt with a real Wesolowski evaluation (small delays only).
    pub fn issue_test(
        &self,
        start_x: u64,
        steps: u64,
        job_meta: &[u8],
    ) -> Result<VdfReceipt, AdmitError> {
        let (final_x, proof) =
            evaluate_wesolowski(start_x, steps, &vdf_modulus()).ok_or(AdmitError::InvalidVdf)?;
        Ok(VdfReceipt {
            start_x,
            steps,
            job_did: Self::mint_job_did(&final_x, job_meta),
            final_x,
            proof,
            modulus_id: MODULUS_ID_WESOLOWSKI_RSA2048,
            job_meta: job_meta.to_vec(),
        })
    }
}

impl VdfVerifier for WesolowskiVdfVerifier {
    fn min_steps(&self) -> u64 {
        self.min_steps
    }

    fn required_modulus_id(&self) -> Option<u32> {
        self.required_modulus_id
    }

    fn verify_receipt(&self, receipt: &VdfReceipt) -> Result<(), AdmitError> {
        // Policy checks first: all cheap integer comparisons.
        if receipt.steps < self.min_steps {
            return Err(AdmitError::InsufficientSteps);
        }
        if let Some(required) = self.required_modulus_id {
            if receipt.modulus_id != required {
                return Err(AdmitError::InvalidModulus);
            }
        } else if receipt.modulus_id != MODULUS_ID_WESOLOWSKI_RSA2048 {
            // No pin: still only registered groups are admitted.
            return Err(AdmitError::InvalidModulus);
        }
        if receipt.steps == 0 {
            return Err(AdmitError::InvalidVdf);
        }
        let n = vdf_modulus();
        // Encodings must be canonical group elements before any arithmetic.
        if receipt.final_x.len() != VDF_MODULUS_BYTES || receipt.proof.len() != VDF_MODULUS_BYTES {
            return Err(AdmitError::InvalidVdf);
        }
        let y = BigUint::from_bytes_be(&receipt.final_x);
        let pi = BigUint::from_bytes_be(&receipt.proof);
        let one = BigUint::from(1u32);
        if y < one || y >= n || pi < one || pi >= n {
            return Err(AdmitError::InvalidVdf);
        }
        // Fast verification: pi^l * x^(2^steps mod l) = y (mod N).
        let x = hash_to_group(receipt.start_x, receipt.steps, &n);
        let ell = fiat_shamir_prime(&pad_to_256(&x), &receipt.final_x, receipt.steps);
        let r = BigUint::from(2u32).modpow(&BigUint::from(receipt.steps), &ell);
        let lhs = (pi.modpow(&ell, &n) * x.modpow(&r, &n)) % &n;
        if lhs != y {
            return Err(AdmitError::InvalidVdf);
        }
        Ok(())
    }
}

/// Default production admission verifier: full VDF delay, RSA-2048 group pinned.
impl Default for WesolowskiVdfVerifier {
    fn default() -> Self {
        Self::production()
    }
}

/// Admit a job receipt under the given verifier, returning its Ephemeral Job
/// DID. Rejects when the proof is invalid, the steps are below policy, the
/// group is not registered, or the claimed DID does not match the recomputed one.
pub fn admit_job<V: VdfVerifier>(receipt: &VdfReceipt, verifier: &V) -> Result<JobDid, AdmitError> {
    verifier.verify_receipt(receipt)?;
    let did = V::mint_job_did(&receipt.final_x, &receipt.job_meta);
    if did != receipt.job_did {
        return Err(AdmitError::DidMismatch);
    }
    Ok(did)
}

/// Admit only when a receipt is present; `None` → [`AdmitError::MissingVdf`].
///
/// Use this at RPC / worker boundaries that accept optional attachments so
/// missing proofs cannot skip the gate.
pub fn admit_job_required<V: VdfVerifier>(
    receipt: Option<&VdfReceipt>,
    verifier: &V,
) -> Result<JobDid, AdmitError> {
    let receipt = receipt.ok_or(AdmitError::MissingVdf)?;
    admit_job(receipt, verifier)
}

// ---------------------------------------------------------------------------
// Hash-stub verifier (tests only)
// ---------------------------------------------------------------------------

/// Domain separation for the hash stub.
pub const HASH_VDF_DOMAIN: &[u8] = b"mossymesh.vdf.hashstub.v1";
/// Registered group id for the stub verifier (not a real VDF group).
pub const MODULUS_ID_HASH_STUB: u32 = 0xFFFF_FF01;
/// Test-friendly default minimum steps for the stub.
pub const HASH_VDF_STUB_DEFAULT_MIN_STEPS: u64 = 1;

/// Non-VDF stub verifier for admit-path tests that must not burn sequential
/// work. Evaluates `SHA-256(DOMAIN || start_x || steps || modulus_id)` as a
/// stand-in group element. Never use for Sybil resistance.
#[derive(Clone, Debug)]
pub struct DomainSeparatedHashVdfStub {
    /// Minimum accepted step count.
    pub min_steps: u64,
}

impl DomainSeparatedHashVdfStub {
    pub fn new(min_steps: u64) -> Self {
        Self { min_steps }
    }

    /// Stub stand-in for the group element (32 bytes, big-endian).
    pub fn evaluate(start_x: u64, steps: u64, modulus_id: u32) -> u64 {
        let mut hasher = Sha256::new();
        hasher.update(HASH_VDF_DOMAIN);
        hasher.update(start_x.to_be_bytes());
        hasher.update(steps.to_be_bytes());
        hasher.update(modulus_id.to_be_bytes());
        let digest = hasher.finalize();
        let mut bytes = [0u8; 8];
        bytes.copy_from_slice(&digest[..8]);
        u64::from_be_bytes(bytes)
    }

    /// Build an honest receipt for tests / local admit wiring.
    pub fn issue(&self, start_x: u64, steps: u64, modulus_id: u32, job_meta: &[u8]) -> VdfReceipt {
        let final_x = Self::evaluate(start_x, steps, modulus_id);
        let final_bytes = final_x.to_be_bytes().to_vec();
        VdfReceipt {
            start_x,
            steps,
            final_x: final_bytes.clone(),
            proof: Vec::new(),
            modulus_id,
            job_meta: job_meta.to_vec(),
            job_did: Self::mint_job_did(&final_bytes, job_meta),
        }
    }
}

impl Default for DomainSeparatedHashVdfStub {
    fn default() -> Self {
        Self::new(HASH_VDF_STUB_DEFAULT_MIN_STEPS)
    }
}

impl VdfVerifier for DomainSeparatedHashVdfStub {
    fn min_steps(&self) -> u64 {
        self.min_steps
    }

    fn required_modulus_id(&self) -> Option<u32> {
        // Test-only stub: the modulus id is folded into the hash, not pinned.
        None
    }

    fn verify_receipt(&self, receipt: &VdfReceipt) -> Result<(), AdmitError> {
        if receipt.steps < self.min_steps {
            return Err(AdmitError::InsufficientSteps);
        }
        let final_x = u64::from_be_bytes(
            receipt
                .final_x
                .as_slice()
                .try_into()
                .map_err(|_| AdmitError::InvalidVdf)?,
        );
        let expected = Self::evaluate(receipt.start_x, receipt.steps, receipt.modulus_id);
        if expected != final_x {
            return Err(AdmitError::InvalidVdf);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    fn test_verifier() -> WesolowskiVdfVerifier {
        WesolowskiVdfVerifier::for_tests(4)
    }

    #[test]
    fn wesolowski_honest_receipt_admits() {
        let v = test_verifier();
        let meta = b"job-42";
        let receipt = v.issue_test(11, 8, meta).expect("issue test receipt");
        assert_eq!(receipt.modulus_id, MODULUS_ID_WESOLOWSKI_RSA2048);
        assert_eq!(receipt.final_x.len(), VDF_MODULUS_BYTES);
        assert_eq!(receipt.proof.len(), VDF_MODULUS_BYTES);
        let did = admit_job(&receipt, &v).expect("admit honest receipt");
        assert_eq!(did, receipt.job_did);
        assert_eq!(
            did,
            WesolowskiVdfVerifier::mint_job_did(&receipt.final_x, meta)
        );
    }

    #[test]
    fn wesolowski_tampered_output_rejected() {
        let v = test_verifier();
        let mut receipt = v.issue_test(11, 8, b"job-42").expect("issue");
        let last = receipt.final_x.len() - 1;
        receipt.final_x[last] ^= 0x01;
        assert_eq!(admit_job(&receipt, &v).unwrap_err(), AdmitError::InvalidVdf);
    }

    #[test]
    fn wesolowski_mutated_proof_rejected() {
        let v = test_verifier();
        let mut receipt = v.issue_test(11, 8, b"job-42").expect("issue");
        let last = receipt.proof.len() - 1;
        receipt.proof[last] ^= 0x01;
        assert_eq!(admit_job(&receipt, &v).unwrap_err(), AdmitError::InvalidVdf);
    }

    #[test]
    fn wesolowski_malformed_encoding_rejected() {
        let v = test_verifier();
        let mut receipt = v.issue_test(11, 8, b"job-42").expect("issue");
        receipt.proof = vec![0u8; 32];
        assert_eq!(admit_job(&receipt, &v).unwrap_err(), AdmitError::InvalidVdf);
        let mut receipt = v.issue_test(11, 8, b"job-42").expect("issue");
        receipt.final_x = vec![0xffu8; VDF_MODULUS_BYTES];
        assert_eq!(admit_job(&receipt, &v).unwrap_err(), AdmitError::InvalidVdf);
    }

    #[test]
    fn wesolowski_garbage_rejected_in_fraction_of_eval_time() {
        // Issue #199: garbage receipts must be cheap to reject; rejection
        // cost must not scale with the claimed step count.
        let v = WesolowskiVdfVerifier::for_tests(8);
        let meta = b"job-garbage";
        let start = Instant::now();
        let _honest = v.issue_test(7, 4_000, meta).expect("issue");
        let eval_time = start.elapsed();

        // Deterministic garbage: decodes below N but fails the equation.
        let final_x = vec![0x5au8; VDF_MODULUS_BYTES];
        let receipt = VdfReceipt {
            start_x: 7,
            steps: 4_000,
            final_x: final_x.clone(),
            proof: vec![0xa5u8; VDF_MODULUS_BYTES],
            modulus_id: MODULUS_ID_WESOLOWSKI_RSA2048,
            job_meta: meta.to_vec(),
            job_did: WesolowskiVdfVerifier::mint_job_did(&final_x, meta),
        };
        let start = Instant::now();
        let err = admit_job(&receipt, &v).unwrap_err();
        let reject_time = start.elapsed();
        assert_eq!(err, AdmitError::InvalidVdf);
        assert!(
            reject_time * 4 < eval_time,
            "garbage rejection ({reject_time:?}) must be a fraction of eval ({eval_time:?})"
        );
    }

    #[test]
    fn wesolowski_invalid_group_rejected() {
        let v = test_verifier();
        let mut receipt = v.issue_test(11, 8, b"job-42").expect("issue");
        receipt.modulus_id = 0xDEAD_BEEF;
        assert_eq!(
            admit_job(&receipt, &v).unwrap_err(),
            AdmitError::InvalidModulus
        );
    }

    #[test]
    fn wesolowski_insufficient_steps_rejected() {
        let v = WesolowskiVdfVerifier::for_tests(64);
        let receipt = WesolowskiVdfVerifier::for_tests(4)
            .issue_test(11, 16, b"job-42")
            .expect("issue");
        assert_eq!(
            admit_job(&receipt, &v).unwrap_err(),
            AdmitError::InsufficientSteps
        );
    }

    #[test]
    fn wesolowski_zero_steps_rejected() {
        // for_tests clamps min_steps to >= 1, so a zero-step receipt is
        // rejected at the policy floor before any arithmetic.
        let v = WesolowskiVdfVerifier::for_tests(0);
        assert_eq!(v.min_steps(), 1);
        let receipt = VdfReceipt {
            start_x: 1,
            steps: 0,
            final_x: vec![1u8; VDF_MODULUS_BYTES],
            proof: vec![1u8; VDF_MODULUS_BYTES],
            modulus_id: MODULUS_ID_WESOLOWSKI_RSA2048,
            job_meta: b"job".to_vec(),
            job_did: JobDid([0u8; 32]),
        };
        assert_eq!(
            admit_job(&receipt, &v).unwrap_err(),
            AdmitError::InsufficientSteps
        );
    }

    #[test]
    fn wesolowski_did_mismatch_rejected() {
        let v = test_verifier();
        let mut receipt = v.issue_test(11, 8, b"job-42").expect("issue");
        receipt.job_did = JobDid([9u8; 32]);
        assert_eq!(
            admit_job(&receipt, &v).unwrap_err(),
            AdmitError::DidMismatch
        );
    }

    #[test]
    fn default_is_production_grade() {
        let v = WesolowskiVdfVerifier::default();
        assert_eq!(v.min_steps(), PRODUCTION_ITERATIONS);
        assert_eq!(v.required_modulus_id(), Some(MODULUS_ID_WESOLOWSKI_RSA2048));
        assert_eq!(PRODUCTION_ITERATIONS, 50_000_000);
        assert_eq!(MOBILE_ITERATIONS, 25_000_000);
        const {
            assert!(DEFAULT_TEST_ITERATIONS < MAX_TEST_ITERATIONS);
            assert!(MAX_TEST_ITERATIONS < PRODUCTION_ITERATIONS);
            assert!(MOBILE_ITERATIONS < PRODUCTION_ITERATIONS);
        }
    }

    #[test]
    fn production_modulus_is_rsa2048_challenge() {
        let n = vdf_modulus();
        assert_eq!(n.bits(), 2048);
        let dec_expected = "25195908475657893494027183240048398571429282126204032027777137836043662020707595556264018525880784406918290641249515082189298559149176184502808489120072844992687392807287776735971418347270261896375014971824691165077613379859095700097330459748808428401797429100642458691817195118746121515172654632282216869987549182422433637259085141865462043576798423387184774447920739934236584823824281198163815010674810451660377306056201619676256133844143603833904414952634432190114657544454178424020924616515723350778707749817125772467962926386356373289912154831438167899885040445364023527381951378636564391212010397122822120720357";
        assert_eq!(n.to_str_radix(10), dec_expected);
    }

    #[test]
    fn stable_error_codes_exhaustive() {
        assert_eq!(AdmitError::MissingVdf.code(), "MISSING_VDF");
        assert_eq!(AdmitError::InvalidVdf.code(), "INVALID_VDF");
        assert_eq!(AdmitError::DidMismatch.code(), "DID_MISMATCH");
        assert_eq!(AdmitError::InsufficientSteps.code(), "INSUFFICIENT_STEPS");
        assert_eq!(AdmitError::InvalidModulus.code(), "INVALID_MODULUS");
        assert_eq!(
            AdmitError::MissingVdf.as_str(),
            "admit rejected: missing VDF proof / receipt"
        );
        assert_eq!(
            AdmitError::InvalidVdf.as_str(),
            "admit rejected: invalid VDF proof or output"
        );
        assert_eq!(
            AdmitError::DidMismatch.as_str(),
            "admit rejected: Job DID does not match VDF receipt"
        );
        assert_eq!(
            AdmitError::InsufficientSteps.as_str(),
            "admit rejected: insufficient VDF steps"
        );
        assert_eq!(
            AdmitError::InvalidModulus.as_str(),
            "admit rejected: unregistered VDF group"
        );
        assert!(format!("{}", AdmitError::InvalidVdf).contains("invalid VDF"));
    }

    #[test]
    fn hash_stub_evaluate_is_deterministic() {
        let a = DomainSeparatedHashVdfStub::evaluate(7, 42, MODULUS_ID_HASH_STUB);
        let b = DomainSeparatedHashVdfStub::evaluate(7, 42, MODULUS_ID_HASH_STUB);
        assert_eq!(a, b);
        assert_ne!(
            a,
            DomainSeparatedHashVdfStub::evaluate(8, 42, MODULUS_ID_HASH_STUB)
        );
        assert_ne!(
            a,
            DomainSeparatedHashVdfStub::evaluate(7, 43, MODULUS_ID_HASH_STUB)
        );
    }

    #[test]
    fn hash_stub_defaults_are_safe_for_tests() {
        let stub = DomainSeparatedHashVdfStub::default();
        assert_eq!(stub.min_steps(), HASH_VDF_STUB_DEFAULT_MIN_STEPS);
        assert_eq!(stub.required_modulus_id(), None);
        assert_eq!(HASH_VDF_STUB_DEFAULT_MIN_STEPS, 1);
    }

    #[test]
    fn hash_stub_issue_accepts_matching_receipt_and_rejects_tamper() {
        let stub = DomainSeparatedHashVdfStub::new(5);
        let meta = b"stub-job";
        let receipt = stub.issue(123, 9, MODULUS_ID_HASH_STUB, meta);
        assert_eq!(receipt.final_x.len(), 8);
        let did = admit_job(&receipt, &stub).expect("stub receipt admits");
        assert_eq!(did, receipt.job_did);

        // Tampered final_x.
        let mut bad = receipt.clone();
        bad.final_x[7] ^= 0x01;
        assert_eq!(admit_job(&bad, &stub).unwrap_err(), AdmitError::InvalidVdf);

        // DID mismatch.
        let mut bad_did = receipt.clone();
        bad_did.job_did = JobDid([7u8; 32]);
        assert_eq!(
            admit_job(&bad_did, &stub).unwrap_err(),
            AdmitError::DidMismatch
        );
    }

    #[test]
    fn hash_stub_enforces_min_steps() {
        let stub = DomainSeparatedHashVdfStub::new(100);
        let receipt = stub.issue(1, 5, MODULUS_ID_HASH_STUB, b"m");
        assert_eq!(
            admit_job(&receipt, &stub).unwrap_err(),
            AdmitError::InsufficientSteps
        );
        let lenient = DomainSeparatedHashVdfStub::new(1);
        admit_job(&receipt, &lenient).expect("meets min steps");
    }

    #[test]
    fn admit_job_required_rejects_missing_receipt() {
        let v = test_verifier();
        let err = admit_job_required(None, &v).unwrap_err();
        assert_eq!(err, AdmitError::MissingVdf);
        assert_eq!(err.code(), "MISSING_VDF");

        let receipt = v.issue_test(3, 6, b"req").expect("issue");
        let did = admit_job_required(Some(&receipt), &v).expect("admit");
        assert_eq!(did, receipt.job_did);
    }
}
