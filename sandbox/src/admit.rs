//! Job admit gate: Ephemeral Job DID + VDF receipt verification.
//!
//! # Contract (docs/interface-contracts.md)
//! Creating / admitting a job requires a verifiable VDF burn bound to a
//! 32-byte Ephemeral Job DID. Transport owns production MinRoot
//! (`mesh-transport::vdf_sybil`); this crate owns the **sandbox admit hook**
//! so guest work never starts without a receipt check.
//!
//! # Stub vs production MinRoot
//! [`DomainSeparatedHashVdfStub`] is a **clearly labeled** sequential,
//! domain-separated hash proof-of-work used for unit tests and local admit
//! wiring. It is **not** MinRoot and must not be treated as Sybil-hard in
//! production.
//!
//! [`MinRootVdfVerifier`] duplicates the transport MinRoot sequential map and
//! SHA-256 DID mint **without** depending on `mesh-transport` (avoids circular
//! deps). Production nodes should use it (or a transport pre-check) via
//! [`VdfVerifier`].
//!
//! # Stable error codes
//! [`AdmitError::code`] returns a stable `SCREAMING_SNAKE` token suitable for
//! mesh logs and cross-language diagnostics (`MISSING_VDF`, `INVALID_VDF`, …).

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

// --- modulus / iteration constants (mirrored from mesh-transport::vdf_sybil) ---

/// Production iteration target (≈10 min). **Not** used by unit tests.
pub const PRODUCTION_ITERATIONS: u64 = 50_000_000;

/// Production MinRoot modulus (prime, ≡ 3 mod 5). Mirror of transport.
pub const PRODUCTION_MODULUS: u64 = 1_000_000_033;

/// Default test iterations (orders of magnitude below production).
pub const DEFAULT_TEST_ITERATIONS: u64 = 16;

/// Default test modulus (prime 103 ≡ 3 mod 5).
pub const DEFAULT_TEST_MODULUS: u64 = 103;

/// Hard cap for test iteration requests.
pub const MAX_TEST_ITERATIONS: u64 = 10_000;

/// `modulus_id` tag for the sandbox hash stub (not MinRoot).
pub const MODULUS_ID_HASH_STUB: u32 = 0;
/// `modulus_id` → [`DEFAULT_TEST_MODULUS`] MinRoot field.
pub const MODULUS_ID_TEST_MINROOT: u32 = 1;
/// `modulus_id` → [`PRODUCTION_MODULUS`] MinRoot field.
pub const MODULUS_ID_PRODUCTION_MINROOT: u32 = 2;

/// 32-byte Ephemeral Job DID (VDF-gated job identity).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct JobDid(pub [u8; 32]);

impl JobDid {
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
}

impl core::fmt::Display for JobDid {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        for b in &self.0 {
            write!(f, "{b:02x}")?;
        }
        Ok(())
    }
}

/// Portable VDF receipt attached to a job admit request.
///
/// Field layout mirrors the logical `VdfProof` in interface-contracts.md
/// (`start_x`, `steps`, `final_x`, `modulus_id`) plus the bound Job DID and
/// job metadata used when minting the DID.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VdfReceipt {
    /// MinRoot / stub start state.
    pub start_x: u64,
    /// Sequential steps claimed (production ≈ 10 min wall-clock).
    pub steps: u64,
    /// Claimed final VDF state.
    pub final_x: u64,
    /// Parameter / curve id (stub / MinRoot registry tag).
    pub modulus_id: u32,
    /// Job metadata bound into DID mint.
    pub job_meta: Vec<u8>,
    /// Claimed Ephemeral Job DID for this receipt.
    pub job_did: JobDid,
}

/// Errors from the sandbox job admit gate.
///
/// Variant set and [`Self::code`] tokens are part of the mesh error contract —
/// do not rename codes without a coordinated protocol bump.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AdmitError {
    /// No VDF receipt was supplied (admit requires a proof).
    MissingVdf,
    /// Receipt failed sequential VDF / stub verification.
    InvalidVdf,
    /// Claimed Job DID does not match mint from `(final_x, job_meta)`.
    DidMismatch,
    /// Step count below the verifier's minimum delay.
    InsufficientSteps,
    /// Modulus / parameter set rejected by the verifier.
    InvalidModulus,
    /// Receipt rejected by a custom verifier policy.
    Rejected(String),
}

impl AdmitError {
    /// Stable machine-readable error code (mesh logs / RPC).
    pub fn code(&self) -> &'static str {
        match self {
            AdmitError::MissingVdf => "MISSING_VDF",
            AdmitError::InvalidVdf => "INVALID_VDF",
            AdmitError::DidMismatch => "DID_MISMATCH",
            AdmitError::InsufficientSteps => "INSUFFICIENT_STEPS",
            AdmitError::InvalidModulus => "INVALID_MODULUS",
            AdmitError::Rejected(_) => "REJECTED",
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            AdmitError::MissingVdf => "Admit denied: missing VDF proof / receipt.",
            AdmitError::InvalidVdf => "Admit denied: VDF receipt verification failed.",
            AdmitError::DidMismatch => "Admit denied: Job DID does not match VDF receipt.",
            AdmitError::InsufficientSteps => "Admit denied: VDF steps below minimum delay.",
            AdmitError::InvalidModulus => "Admit denied: VDF modulus / parameter set invalid.",
            AdmitError::Rejected(s) => s.as_str(),
        }
    }
}

impl core::fmt::Display for AdmitError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::error::Error for AdmitError {}

/// Pluggable VDF receipt checker used by the admit gate.
///
/// Production: [`MinRootVdfVerifier`] (or transport pre-check). Tests: use
/// [`DomainSeparatedHashVdfStub`].
pub trait VdfVerifier {
    /// Return `Ok(())` when `receipt` proves the claimed sequential delay and
    /// binds `receipt.job_did` to `(final_x, job_meta)`.
    fn verify_receipt(&self, receipt: &VdfReceipt) -> Result<(), AdmitError>;
}

/// Domain tag for the sandbox-local sequential hash PoW stub.
///
/// **NOT MinRoot.** Distinct domain prevents accidental cross-protocol reuse.
pub const HASH_VDF_STUB_DOMAIN: &[u8] = b"mossymesh.sandbox.vdf.stub.v1";

/// Default minimum sequential steps accepted by the hash stub in tests.
pub const HASH_VDF_STUB_DEFAULT_MIN_STEPS: u64 = 8;

/// **STUB ONLY** — sequential domain-separated hash proof-of-work.
///
/// Each step mixes the previous state with the step index and modulus id via
/// a fixed SplitMix64-style round keyed by [`HASH_VDF_STUB_DOMAIN`]. This is
/// intentionally simple, deterministic, and **not** a production VDF.
///
/// Full MinRoot (~10 min sequential delay) lives in `mesh-transport::vdf_sybil`
/// and is mirrored by [`MinRootVdfVerifier`].
#[derive(Clone, Debug)]
pub struct DomainSeparatedHashVdfStub {
    /// Reject receipts with fewer sequential steps than this floor.
    pub min_steps: u64,
}

impl Default for DomainSeparatedHashVdfStub {
    fn default() -> Self {
        Self {
            min_steps: HASH_VDF_STUB_DEFAULT_MIN_STEPS,
        }
    }
}

impl DomainSeparatedHashVdfStub {
    pub fn new(min_steps: u64) -> Self {
        Self { min_steps }
    }

    /// Evaluate the stub sequential map for `steps` iterations.
    pub fn evaluate(start_x: u64, steps: u64, modulus_id: u32) -> u64 {
        let mut state = start_x;
        // Fold domain bytes once into the initial state so verification is
        // domain-separated from other hash uses on the mesh.
        state = mix64(state ^ domain_seed(), 0, modulus_id);
        for i in 1..=steps {
            state = mix64(state, i, modulus_id);
        }
        state
    }

    /// Mint a Job DID the same way the stub verifier expects:
    /// `DID = H_domain(final_x_be || job_meta)`.
    pub fn mint_job_did(final_x: u64, job_meta: &[u8]) -> JobDid {
        mint_job_did_from_output(final_x, job_meta)
    }

    /// Build an honest receipt for tests / local admit wiring.
    pub fn issue(&self, start_x: u64, steps: u64, modulus_id: u32, job_meta: &[u8]) -> VdfReceipt {
        let final_x = Self::evaluate(start_x, steps, modulus_id);
        let job_did = Self::mint_job_did(final_x, job_meta);
        VdfReceipt {
            start_x,
            steps,
            final_x,
            modulus_id,
            job_meta: job_meta.to_vec(),
            job_did,
        }
    }
}

impl VdfVerifier for DomainSeparatedHashVdfStub {
    fn verify_receipt(&self, receipt: &VdfReceipt) -> Result<(), AdmitError> {
        if receipt.steps < self.min_steps {
            return Err(AdmitError::InsufficientSteps);
        }
        let expected = Self::evaluate(receipt.start_x, receipt.steps, receipt.modulus_id);
        if expected != receipt.final_x {
            return Err(AdmitError::InvalidVdf);
        }
        let did = Self::mint_job_did(receipt.final_x, &receipt.job_meta);
        if did != receipt.job_did {
            return Err(AdmitError::DidMismatch);
        }
        Ok(())
    }
}

// --- MinRoot verifier (duplicated check; no mesh-transport dependency) --------

/// Production-oriented MinRoot sequential verifier for the sandbox admit path.
///
/// Mirrors `mesh-transport::vdf_sybil` evaluate/verify + SHA-256 Job DID mint
/// without importing that crate (workspace layering / no circular deps).
#[derive(Clone, Debug)]
pub struct MinRootVdfVerifier {
    /// Reject receipts with fewer sequential steps than this floor.
    ///
    /// Tests: [`DEFAULT_TEST_ITERATIONS`] or lower. Production policy should
    /// require [`PRODUCTION_ITERATIONS`] (or a calibrated minimum).
    pub min_steps: u64,
    /// When set, only this modulus is accepted (after resolving `modulus_id`).
    pub required_modulus: Option<u64>,
}

impl Default for MinRootVdfVerifier {
    fn default() -> Self {
        Self {
            // Test-safe default: never gate unit tests on PRODUCTION_ITERATIONS.
            min_steps: DEFAULT_TEST_ITERATIONS,
            required_modulus: None,
        }
    }
}

impl MinRootVdfVerifier {
    pub fn new(min_steps: u64) -> Self {
        Self {
            min_steps,
            required_modulus: None,
        }
    }

    /// Production policy: require production iteration floor + production modulus.
    pub fn production() -> Self {
        Self {
            min_steps: PRODUCTION_ITERATIONS,
            required_modulus: Some(PRODUCTION_MODULUS),
        }
    }

    /// Test policy: small iteration floor + test modulus.
    pub fn for_tests(min_steps: u64) -> Self {
        let min_steps = min_steps.clamp(1, MAX_TEST_ITERATIONS);
        Self {
            min_steps,
            required_modulus: Some(DEFAULT_TEST_MODULUS),
        }
    }

    /// Resolve receipt `modulus_id` to a concrete field modulus.
    pub fn resolve_modulus(modulus_id: u32) -> Option<u64> {
        match modulus_id {
            MODULUS_ID_TEST_MINROOT => Some(DEFAULT_TEST_MODULUS),
            MODULUS_ID_PRODUCTION_MINROOT => Some(PRODUCTION_MODULUS),
            // Allow embedding small primes directly when id >= 5 and valid.
            id if id >= 5 => {
                let p = u64::from(id);
                if validate_minroot_modulus(p) {
                    Some(p)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// Evaluate MinRoot sequential map (same step as transport).
    pub fn evaluate(start_x: u64, steps: u64, modulus: u64) -> Option<u64> {
        if steps == 0 || !validate_minroot_modulus(modulus) {
            return None;
        }
        let mut current = start_x;
        for i in 1..=steps {
            current = compute_minroot_step(current, i, modulus)?;
        }
        Some(current)
    }

    /// SHA-256 Job DID mint (matches transport `mint_ephemeral_job_did`).
    pub fn mint_job_did(final_x: u64, job_meta: &[u8]) -> JobDid {
        let mut hasher = Sha256::new();
        hasher.update(final_x.to_be_bytes());
        hasher.update(job_meta);
        let digest = hasher.finalize();
        let mut out = [0u8; 32];
        out.copy_from_slice(&digest);
        JobDid(out)
    }

    /// Build an honest MinRoot receipt for tests (test modulus id).
    pub fn issue_test(
        &self,
        start_x: u64,
        steps: u64,
        job_meta: &[u8],
    ) -> Result<VdfReceipt, AdmitError> {
        let steps = steps.clamp(1, MAX_TEST_ITERATIONS);
        let modulus = DEFAULT_TEST_MODULUS;
        let final_x = Self::evaluate(start_x, steps, modulus).ok_or(AdmitError::InvalidModulus)?;
        let job_did = Self::mint_job_did(final_x, job_meta);
        Ok(VdfReceipt {
            start_x,
            steps,
            final_x,
            modulus_id: MODULUS_ID_TEST_MINROOT,
            job_meta: job_meta.to_vec(),
            job_did,
        })
    }
}

impl VdfVerifier for MinRootVdfVerifier {
    fn verify_receipt(&self, receipt: &VdfReceipt) -> Result<(), AdmitError> {
        if receipt.steps == 0 {
            return Err(AdmitError::InvalidVdf);
        }
        if receipt.steps < self.min_steps {
            return Err(AdmitError::InsufficientSteps);
        }
        let modulus = Self::resolve_modulus(receipt.modulus_id).ok_or(AdmitError::InvalidModulus)?;
        if !validate_minroot_modulus(modulus) {
            return Err(AdmitError::InvalidModulus);
        }
        if let Some(req) = self.required_modulus {
            if modulus != req {
                return Err(AdmitError::InvalidModulus);
            }
        }
        let expected =
            Self::evaluate(receipt.start_x, receipt.steps, modulus).ok_or(AdmitError::InvalidVdf)?;
        if expected != receipt.final_x {
            return Err(AdmitError::InvalidVdf);
        }
        let did = Self::mint_job_did(receipt.final_x, &receipt.job_meta);
        if did != receipt.job_did {
            return Err(AdmitError::DidMismatch);
        }
        Ok(())
    }
}

/// Strong MinRoot modulus checks (mirror of transport `validate_modulus`).
pub fn validate_minroot_modulus(p: u64) -> bool {
    if p <= 5 || p % 2 == 0 {
        return false;
    }
    if p % 5 == 1 {
        return false;
    }
    let num = 2u128 * p as u128 - 1;
    if num % 5 != 0 {
        return false;
    }
    is_prime_u64(p)
}

fn is_prime_u64(n: u64) -> bool {
    if n < 2 {
        return false;
    }
    const SMALL: &[u64] = &[2, 3, 5, 7, 11, 13, 17, 19, 23, 29, 31, 37];
    for &p in SMALL {
        if n == p {
            return true;
        }
        if n % p == 0 {
            return false;
        }
    }
    let mut d = n - 1;
    let mut s = 0u32;
    while d % 2 == 0 {
        d /= 2;
        s += 1;
    }
    const WITNESSES: &[u64] = &[2, 3, 5, 7, 11, 13, 23];
    'witness: for &a in WITNESSES {
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

fn fifth_root_exponent(p: u64) -> Option<u64> {
    if p < 5 || p % 5 == 1 {
        return None;
    }
    let num = 2u128 * p as u128 - 1;
    if num % 5 == 0 {
        return Some((num / 5) as u64);
    }
    None
}

fn mod_exp(mut base: u64, mut exp: u64, modulus: u64) -> u64 {
    if modulus <= 1 {
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

fn compute_minroot_step(x: u64, i: u64, p: u64) -> Option<u64> {
    let d = fifth_root_exponent(p)?;
    if p <= 1 {
        return None;
    }
    let base = (x as u128 + i as u128) % p as u128;
    Some(mod_exp(base as u64, d, p))
}

/// Admit a job when the attached VDF receipt verifies under `verifier`.
///
/// On success returns the receipt's [`JobDid`] (identity under which the guest
/// may load / run). Callers should bind that DID to the subsequent [`crate::Job`].
pub fn admit_job(receipt: &VdfReceipt, verifier: &impl VdfVerifier) -> Result<JobDid, AdmitError> {
    verifier.verify_receipt(receipt)?;
    Ok(receipt.job_did)
}

/// Admit only when a receipt is present; `None` → [`AdmitError::MissingVdf`].
///
/// Use this at RPC / worker boundaries that accept optional attachments so
/// missing proofs cannot skip the gate.
pub fn admit_job_required(
    receipt: Option<&VdfReceipt>,
    verifier: &impl VdfVerifier,
) -> Result<JobDid, AdmitError> {
    let receipt = receipt.ok_or(AdmitError::MissingVdf)?;
    admit_job(receipt, verifier)
}

// --- internal hashing helpers for the stub path (no extra crate deps) --------

fn domain_seed() -> u64 {
    // Fixed 64-bit fold of HASH_VDF_STUB_DOMAIN for domain separation.
    let mut h = 0xcbf2_9ce4_8422_2325u64; // FNV-1a offset basis
    for &b in HASH_VDF_STUB_DOMAIN {
        h ^= b as u64;
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    h
}

/// SplitMix64-style mix; deterministic across platforms.
fn mix64(state: u64, step: u64, modulus_id: u32) -> u64 {
    let mut z = state
        .wrapping_add(0x9E37_79B9_7F4A_7C15)
        .wrapping_add(step)
        .wrapping_add(u64::from(modulus_id));
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Expand `(final_x, job_meta)` into a 32-byte Job DID via domain-separated mixing.
fn mint_job_did_from_output(final_x: u64, job_meta: &[u8]) -> JobDid {
    let mut out = [0u8; 32];
    // Four 64-bit lanes, each seeded differently, absorb meta bytes.
    let seeds = [
        final_x,
        final_x ^ domain_seed(),
        final_x.wrapping_mul(0x9E37_79B9_7F4A_7C15),
        !final_x,
    ];
    for (lane, seed) in seeds.iter().enumerate() {
        let mut state = mix64(*seed, lane as u64, 0);
        for (i, &b) in job_meta.iter().enumerate() {
            state = mix64(state ^ (b as u64), (i as u64).wrapping_add(1), lane as u32);
        }
        // Absorb length to avoid simple suffix collisions.
        state = mix64(state, job_meta.len() as u64, 0xffff_ffff);
        out[lane * 8..(lane + 1) * 8].copy_from_slice(&state.to_be_bytes());
    }
    JobDid(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn honest_receipt_admits() {
        let stub = DomainSeparatedHashVdfStub::new(8);
        let receipt = stub.issue(42, 16, 1, b"job-meta-A");
        let did = admit_job(&receipt, &stub).expect("honest receipt must admit");
        assert_eq!(did, receipt.job_did);
    }

    #[test]
    fn tampered_final_x_rejected() {
        let stub = DomainSeparatedHashVdfStub::default();
        let mut receipt = stub.issue(7, 12, 2, b"meta");
        receipt.final_x = receipt.final_x.wrapping_add(1);
        assert_eq!(
            admit_job(&receipt, &stub).unwrap_err(),
            AdmitError::InvalidVdf
        );
        assert_eq!(AdmitError::InvalidVdf.code(), "INVALID_VDF");
    }

    #[test]
    fn did_mismatch_rejected() {
        let stub = DomainSeparatedHashVdfStub::default();
        let mut receipt = stub.issue(1, 10, 0, b"meta");
        receipt.job_did.0[0] ^= 0xff;
        assert_eq!(
            admit_job(&receipt, &stub).unwrap_err(),
            AdmitError::DidMismatch
        );
        assert_eq!(AdmitError::DidMismatch.code(), "DID_MISMATCH");
    }

    #[test]
    fn insufficient_steps_rejected() {
        let stub = DomainSeparatedHashVdfStub::new(32);
        let receipt = stub.issue(9, 8, 0, b"meta"); // steps < min
        assert_eq!(
            admit_job(&receipt, &stub).unwrap_err(),
            AdmitError::InsufficientSteps
        );
        assert_eq!(AdmitError::InsufficientSteps.code(), "INSUFFICIENT_STEPS");
    }

    #[test]
    fn missing_vdf_rejected_with_stable_code() {
        let stub = DomainSeparatedHashVdfStub::default();
        let err = admit_job_required(None, &stub).unwrap_err();
        assert_eq!(err, AdmitError::MissingVdf);
        assert_eq!(err.code(), "MISSING_VDF");
    }

    #[test]
    fn admit_job_required_accepts_present_receipt() {
        let stub = DomainSeparatedHashVdfStub::new(8);
        let receipt = stub.issue(3, 16, 0, b"present");
        let did = admit_job_required(Some(&receipt), &stub).unwrap();
        assert_eq!(did, receipt.job_did);
    }

    #[test]
    fn did_binds_meta_and_output() {
        let a = DomainSeparatedHashVdfStub::mint_job_did(100, b"A");
        let b = DomainSeparatedHashVdfStub::mint_job_did(100, b"B");
        let c = DomainSeparatedHashVdfStub::mint_job_did(101, b"A");
        assert_ne!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn evaluate_is_deterministic() {
        let x = DomainSeparatedHashVdfStub::evaluate(5, 20, 3);
        let y = DomainSeparatedHashVdfStub::evaluate(5, 20, 3);
        assert_eq!(x, y);
        // Different steps → different output (with overwhelming probability /
        // for this mix; check against shorter prefix chain).
        let shorter = DomainSeparatedHashVdfStub::evaluate(5, 19, 3);
        assert_ne!(x, shorter);
    }

    #[test]
    fn minroot_honest_receipt_admits() {
        let v = MinRootVdfVerifier::for_tests(8);
        let receipt = v.issue_test(42, 16, b"minroot-job").unwrap();
        let did = admit_job(&receipt, &v).expect("minroot receipt must admit");
        assert_eq!(did, receipt.job_did);
        assert_eq!(receipt.modulus_id, MODULUS_ID_TEST_MINROOT);
    }

    #[test]
    fn minroot_tampered_output_rejected() {
        let v = MinRootVdfVerifier::for_tests(4);
        let mut receipt = v.issue_test(7, 8, b"m").unwrap();
        receipt.final_x = receipt.final_x.wrapping_add(1);
        assert_eq!(
            admit_job(&receipt, &v).unwrap_err().code(),
            "INVALID_VDF"
        );
    }

    #[test]
    fn minroot_invalid_modulus_id_rejected() {
        let v = MinRootVdfVerifier::for_tests(4);
        let mut receipt = v.issue_test(1, 8, b"m").unwrap();
        receipt.modulus_id = MODULUS_ID_HASH_STUB; // stub id, not MinRoot
        assert_eq!(
            admit_job(&receipt, &v).unwrap_err(),
            AdmitError::InvalidModulus
        );
        assert_eq!(AdmitError::InvalidModulus.code(), "INVALID_MODULUS");
    }

    #[test]
    fn minroot_insufficient_steps_uses_test_floor_not_production() {
        let v = MinRootVdfVerifier::for_tests(32);
        let receipt = v.issue_test(9, 8, b"meta").unwrap(); // steps < min
        assert_eq!(
            admit_job(&receipt, &v).unwrap_err(),
            AdmitError::InsufficientSteps
        );
        // Production floor must remain documented and separated.
        assert_eq!(PRODUCTION_ITERATIONS, 50_000_000);
        assert!(DEFAULT_TEST_ITERATIONS < PRODUCTION_ITERATIONS);
        assert!(MAX_TEST_ITERATIONS < PRODUCTION_ITERATIONS);
        let prod_policy = MinRootVdfVerifier::production();
        assert_eq!(prod_policy.min_steps, PRODUCTION_ITERATIONS);
        assert_eq!(prod_policy.required_modulus, Some(PRODUCTION_MODULUS));
    }

    #[test]
    fn validate_minroot_modulus_surface() {
        assert!(validate_minroot_modulus(DEFAULT_TEST_MODULUS));
        assert!(validate_minroot_modulus(PRODUCTION_MODULUS));
        assert!(!validate_minroot_modulus(11));
        assert!(!validate_minroot_modulus(9));
    }

    #[test]
    fn stable_error_codes_exhaustive() {
        assert_eq!(AdmitError::MissingVdf.code(), "MISSING_VDF");
        assert_eq!(AdmitError::InvalidVdf.code(), "INVALID_VDF");
        assert_eq!(AdmitError::DidMismatch.code(), "DID_MISMATCH");
        assert_eq!(AdmitError::InsufficientSteps.code(), "INSUFFICIENT_STEPS");
        assert_eq!(AdmitError::InvalidModulus.code(), "INVALID_MODULUS");
        assert_eq!(
            AdmitError::Rejected("x".into()).code(),
            "REJECTED"
        );
    }
}
