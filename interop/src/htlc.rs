//! Hashed Timelock Contracts (HTLCs) for escrowed compute credits.
//!
//! State machine:
//! ```text
//!   Funded ──claim(preimage)──► Claimed
//!     │
//!     ├──refund(after timeout)──► Refunded
//!     │
//!     └──vdf_cancel(after VDF)──► VdfCancelled
//! ```
//!
//! Claims are hash-locked with SHA-256: `payment_hash = SHA-256(preimage)`.
//! Cancellation requires completing a sequential VDF delay (mockable for tests).

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Lifecycle state of an HTLC escrow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HtlcState {
    /// Credits locked; awaiting claim, refund, or VDF cancel.
    Funded,
    /// Receiver revealed the preimage and claimed credits.
    Claimed,
    /// Sender reclaimed credits after the timeout height.
    Refunded,
    /// Sender cancelled after completing the VDF delay.
    VdfCancelled,
}

/// Errors produced by HTLC state transitions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HtlcError {
    /// Provided preimage does not hash to the payment hash.
    InvalidPreimage,
    /// Contract is not in the `Funded` state.
    NotFunded,
    /// Timeout height has not been reached yet.
    TimeoutNotReached,
    /// Required VDF steps have not been completed.
    VdfNotComplete,
    /// Contract already settled (claimed / refunded / cancelled).
    AlreadySettled,
    /// Amount must be greater than zero.
    InvalidAmount,
    /// Payment hash must be a full 32-byte SHA-256 digest.
    InvalidPaymentHash,
}

impl std::fmt::Display for HtlcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HtlcError::InvalidPreimage => write!(f, "invalid preimage"),
            HtlcError::NotFunded => write!(f, "htlc is not funded"),
            HtlcError::TimeoutNotReached => write!(f, "timeout not reached"),
            HtlcError::VdfNotComplete => write!(f, "vdf delay not complete"),
            HtlcError::AlreadySettled => write!(f, "htlc already settled"),
            HtlcError::InvalidAmount => write!(f, "amount must be > 0"),
            HtlcError::InvalidPaymentHash => write!(f, "payment hash must be 32 bytes"),
        }
    }
}

impl std::error::Error for HtlcError {}

/// Compute `SHA-256(preimage)` as the HTLC payment hash.
pub fn hash_preimage(preimage: &[u8]) -> [u8; 32] {
    let digest = Sha256::digest(preimage);
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    out
}

/// Verify that `SHA-256(preimage) == payment_hash`.
pub fn verify_preimage(preimage: &[u8], payment_hash: &[u8; 32]) -> bool {
    &hash_preimage(preimage) == payment_hash
}

/// Mock sequential VDF used for delayed cancellation.
///
/// Production would swap this for MinRoot (`x → x^{1/5} mod p`). Tests use a
/// cheap modular step so delay can be advanced without wall-clock waits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MockVdf {
    /// Seed / starting value for the chain.
    pub start: u64,
    /// Number of sequential steps required to unlock cancel.
    pub steps_required: u64,
    /// Steps already burned.
    pub steps_completed: u64,
    /// Current chain tip after `steps_completed` evaluations.
    pub current: u64,
    /// Small modulus for the mock polynomial step (not cryptographically hard).
    pub modulus: u64,
}

impl MockVdf {
    /// Create a new mock VDF that must burn `steps_required` sequential steps.
    pub fn new(start: u64, steps_required: u64) -> Self {
        Self {
            start,
            steps_required,
            steps_completed: 0,
            current: start,
            modulus: 1_000_003, // small prime for deterministic mock steps
        }
    }

    /// One sequential mock step: `x ↦ (x² + i) mod p`.
    pub fn step_once(&mut self) {
        if self.steps_completed >= self.steps_required {
            return;
        }
        let i = self.steps_completed.wrapping_add(1);
        let x = self.current as u128;
        let next = (x * x + i as u128) % self.modulus as u128;
        self.current = next as u64;
        self.steps_completed = self.steps_completed.saturating_add(1);
    }

    /// Advance the VDF by up to `n` steps (stops at `steps_required`).
    pub fn advance(&mut self, n: u64) {
        for _ in 0..n {
            if self.is_complete() {
                break;
            }
            self.step_once();
        }
    }

    /// Whether the full sequential delay has been burned.
    pub fn is_complete(&self) -> bool {
        self.steps_completed >= self.steps_required
    }

    /// Remaining steps before cancel is allowed.
    pub fn remaining(&self) -> u64 {
        self.steps_required.saturating_sub(self.steps_completed)
    }
}

/// Parameters required to open a funded HTLC.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HtlcParams {
    /// Opaque escrow identifier (caller-chosen or random).
    pub id: [u8; 32],
    /// Party locking the credits (refund / cancel beneficiary).
    pub sender: String,
    /// Party that can claim with the preimage.
    pub receiver: String,
    /// Amount of compute credits locked.
    pub amount: u64,
    /// `SHA-256(preimage)` payment hash.
    pub payment_hash: [u8; 32],
    /// Logical height / tick after which the sender may refund.
    pub timeout_height: u64,
    /// Height at which the HTLC was funded (inclusive lower bound for time).
    pub funded_height: u64,
    /// Sequential VDF steps required for the VDF-cancel path.
    pub vdf_steps: u64,
    /// Optional seed for the mock VDF (defaults derived from `id` if not set).
    pub vdf_seed: Option<u64>,
}

/// An HTLC locking compute credits until claim, timeout refund, or VDF cancel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Htlc {
    pub id: [u8; 32],
    pub sender: String,
    pub receiver: String,
    pub amount: u64,
    pub payment_hash: [u8; 32],
    pub timeout_height: u64,
    pub funded_height: u64,
    pub state: HtlcState,
    /// Preimage revealed on a successful claim.
    pub claimed_preimage: Option<Vec<u8>>,
    /// Sequential delay gate for the cancel path.
    pub vdf: MockVdf,
}

impl Htlc {
    /// Fund a new HTLC in the `Funded` state.
    pub fn fund(params: HtlcParams) -> Result<Self, HtlcError> {
        if params.amount == 0 {
            return Err(HtlcError::InvalidAmount);
        }
        let seed = params.vdf_seed.unwrap_or_else(|| {
            let mut buf = [0u8; 8];
            buf.copy_from_slice(&params.id[0..8]);
            u64::from_le_bytes(buf)
        });
        Ok(Self {
            id: params.id,
            sender: params.sender,
            receiver: params.receiver,
            amount: params.amount,
            payment_hash: params.payment_hash,
            timeout_height: params.timeout_height,
            funded_height: params.funded_height,
            state: HtlcState::Funded,
            claimed_preimage: None,
            vdf: MockVdf::new(seed, params.vdf_steps),
        })
    }

    /// Convenience: fund from a known preimage (computes the payment hash).
    pub fn fund_with_preimage(
        id: [u8; 32],
        sender: impl Into<String>,
        receiver: impl Into<String>,
        amount: u64,
        preimage: &[u8],
        timeout_height: u64,
        funded_height: u64,
        vdf_steps: u64,
    ) -> Result<Self, HtlcError> {
        Self::fund(HtlcParams {
            id,
            sender: sender.into(),
            receiver: receiver.into(),
            amount,
            payment_hash: hash_preimage(preimage),
            timeout_height,
            funded_height,
            vdf_steps,
            vdf_seed: None,
        })
    }

    /// Claim locked credits by revealing the SHA-256 preimage.
    pub fn claim(&mut self, preimage: &[u8]) -> Result<(), HtlcError> {
        match self.state {
            HtlcState::Funded => {}
            HtlcState::Claimed | HtlcState::Refunded | HtlcState::VdfCancelled => {
                return Err(HtlcError::AlreadySettled);
            }
        }
        if !verify_preimage(preimage, &self.payment_hash) {
            return Err(HtlcError::InvalidPreimage);
        }
        self.claimed_preimage = Some(preimage.to_vec());
        self.state = HtlcState::Claimed;
        Ok(())
    }

    /// Refund to the sender once `current_height >= timeout_height`.
    pub fn refund(&mut self, current_height: u64) -> Result<(), HtlcError> {
        match self.state {
            HtlcState::Funded => {}
            HtlcState::Claimed | HtlcState::Refunded | HtlcState::VdfCancelled => {
                return Err(HtlcError::AlreadySettled);
            }
        }
        if current_height < self.timeout_height {
            return Err(HtlcError::TimeoutNotReached);
        }
        self.state = HtlcState::Refunded;
        Ok(())
    }

    /// Advance the VDF cancel delay by `steps` sequential evaluations.
    pub fn advance_vdf(&mut self, steps: u64) -> Result<(), HtlcError> {
        if self.state != HtlcState::Funded {
            return Err(HtlcError::AlreadySettled);
        }
        self.vdf.advance(steps);
        Ok(())
    }

    /// Cancel via the VDF-delayed path once the sequential delay is complete.
    ///
    /// Unlike timeout refund, this does not wait on chain height — it requires
    /// burning sequential VDF work (or an equivalent step counter), which cannot
    /// be parallelized. Immediate cancel is never allowed.
    pub fn vdf_cancel(&mut self) -> Result<(), HtlcError> {
        match self.state {
            HtlcState::Funded => {}
            HtlcState::Claimed | HtlcState::Refunded | HtlcState::VdfCancelled => {
                return Err(HtlcError::AlreadySettled);
            }
        }
        if !self.vdf.is_complete() {
            return Err(HtlcError::VdfNotComplete);
        }
        self.state = HtlcState::VdfCancelled;
        Ok(())
    }

    /// Delayed cancel alias: only succeeds after the VDF / step-counter delay.
    ///
    /// Equivalent to [`Self::vdf_cancel`]. Prefer this name when the delay
    /// backend is a generic step counter rather than a cryptographic VDF.
    pub fn cancel_after_delay(&mut self) -> Result<(), HtlcError> {
        self.vdf_cancel()
    }

    /// Whether the VDF / step-counter delay condition for cancel is satisfied.
    pub fn delay_satisfied(&self) -> bool {
        self.vdf.is_complete()
    }

    /// Whether the contract still holds escrowed funds.
    pub fn is_open(&self) -> bool {
        self.state == HtlcState::Funded
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_id() -> [u8; 32] {
        let mut id = [0u8; 32];
        id[0] = 0x10;
        id[1] = 0xec;
        id
    }

    fn funded_htlc(preimage: &[u8], timeout: u64, vdf_steps: u64) -> Htlc {
        Htlc::fund_with_preimage(
            sample_id(),
            "alice",
            "bob",
            1_000,
            preimage,
            timeout,
            0,
            vdf_steps,
        )
        .expect("fund")
    }

    /// Happy path: receiver claims with the correct SHA-256 preimage.
    #[test]
    fn happy_path_claim() {
        let preimage = b"secret-compute-receipt-v1";
        let mut htlc = funded_htlc(preimage, 100, 10);
        assert_eq!(htlc.state, HtlcState::Funded);

        htlc.claim(preimage).expect("claim should succeed");
        assert_eq!(htlc.state, HtlcState::Claimed);
        assert_eq!(htlc.claimed_preimage.as_deref(), Some(preimage.as_slice()));
        assert!(!htlc.is_open());
    }

    /// Timeout refund: sender reclaims after timeout height.
    #[test]
    fn timeout_refund() {
        let preimage = b"unused-secret";
        let mut htlc = funded_htlc(preimage, 50, 100);

        assert_eq!(
            htlc.refund(49),
            Err(HtlcError::TimeoutNotReached),
            "refund before timeout must fail"
        );

        htlc.refund(50).expect("refund at timeout");
        assert_eq!(htlc.state, HtlcState::Refunded);

        assert_eq!(
            htlc.claim(preimage),
            Err(HtlcError::AlreadySettled),
            "claim after refund must fail"
        );
    }

    /// Invalid preimage is rejected; HTLC remains Funded.
    #[test]
    fn invalid_preimage_reject() {
        let preimage = b"correct-preimage";
        let mut htlc = funded_htlc(preimage, 100, 10);

        assert_eq!(
            htlc.claim(b"wrong-preimage"),
            Err(HtlcError::InvalidPreimage)
        );
        assert_eq!(htlc.state, HtlcState::Funded);
        assert!(htlc.is_open());

        // Correct preimage still works afterwards.
        htlc.claim(preimage).expect("valid claim after reject");
        assert_eq!(htlc.state, HtlcState::Claimed);
    }

    /// VDF-delayed cancellation path with mock sequential steps.
    #[test]
    fn vdf_cancel_path() {
        let preimage = b"vdf-cancel-test";
        let mut htlc = funded_htlc(preimage, 10_000, 5);

        assert_eq!(htlc.vdf_cancel(), Err(HtlcError::VdfNotComplete));

        htlc.advance_vdf(3).unwrap();
        assert!(!htlc.vdf.is_complete());
        assert_eq!(htlc.vdf.remaining(), 2);
        assert_eq!(htlc.vdf_cancel(), Err(HtlcError::VdfNotComplete));

        htlc.advance_vdf(2).unwrap();
        assert!(htlc.vdf.is_complete());
        htlc.vdf_cancel().expect("cancel after VDF complete");
        assert_eq!(htlc.state, HtlcState::VdfCancelled);

        assert_eq!(htlc.refund(10_000), Err(HtlcError::AlreadySettled));
        assert_eq!(htlc.claim(preimage), Err(HtlcError::AlreadySettled));
    }

    #[test]
    fn hash_preimage_is_sha256() {
        // SHA-256("abc") known vector
        let hash = hash_preimage(b"abc");
        let expected_hex = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        let expected: Vec<u8> = (0..expected_hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&expected_hex[i..i + 2], 16).unwrap())
            .collect();
        assert_eq!(hash.as_slice(), expected.as_slice());
        assert!(verify_preimage(b"abc", &hash));
        assert!(!verify_preimage(b"abd", &hash));
    }

    #[test]
    fn zero_amount_rejected() {
        let err = Htlc::fund_with_preimage(sample_id(), "a", "b", 0, b"x", 10, 0, 1).unwrap_err();
        assert_eq!(err, HtlcError::InvalidAmount);
    }

    #[test]
    fn mock_vdf_is_sequential_and_deterministic() {
        let mut a = MockVdf::new(42, 4);
        let mut b = MockVdf::new(42, 4);
        a.advance(4);
        for _ in 0..4 {
            b.step_once();
        }
        assert_eq!(a.current, b.current);
        assert!(a.is_complete());
        assert_eq!(a.remaining(), 0);
        // Extra advance is a no-op.
        let tip = a.current;
        a.advance(10);
        assert_eq!(a.current, tip);
        assert_eq!(a.steps_completed, 4);
    }

    /// No double claim: second claim after a successful preimage reveal fails hard.
    #[test]
    fn no_double_claim() {
        let preimage = b"once-only-preimage";
        let mut htlc = funded_htlc(preimage, 100, 10);

        htlc.claim(preimage).expect("first claim");
        assert_eq!(htlc.state, HtlcState::Claimed);

        assert_eq!(
            htlc.claim(preimage),
            Err(HtlcError::AlreadySettled),
            "second claim with same preimage must fail"
        );
        assert_eq!(
            htlc.claim(b"other"),
            Err(HtlcError::AlreadySettled),
            "second claim with any preimage must fail"
        );
        // State unchanged; cannot refund or cancel after claim either.
        assert_eq!(htlc.refund(100), Err(HtlcError::AlreadySettled));
        assert_eq!(htlc.vdf_cancel(), Err(HtlcError::AlreadySettled));
        assert_eq!(htlc.state, HtlcState::Claimed);
    }

    /// Cancel is rejected until the VDF / step-counter delay is fully burned.
    #[test]
    fn cancel_requires_delay_then_succeeds() {
        let preimage = b"delay-gate";
        let mut htlc = funded_htlc(preimage, 10_000, 3);

        assert!(!htlc.delay_satisfied());
        assert_eq!(
            htlc.cancel_after_delay(),
            Err(HtlcError::VdfNotComplete),
            "cancel before delay must fail"
        );

        // Partial progress still blocked.
        htlc.advance_vdf(1).unwrap();
        assert!(!htlc.delay_satisfied());
        assert_eq!(htlc.cancel_after_delay(), Err(HtlcError::VdfNotComplete));

        // Complete the remaining steps.
        htlc.advance_vdf(2).unwrap();
        assert!(htlc.delay_satisfied());
        htlc.cancel_after_delay().expect("cancel after delay");
        assert_eq!(htlc.state, HtlcState::VdfCancelled);

        // No double cancel / no claim after cancel.
        assert_eq!(htlc.cancel_after_delay(), Err(HtlcError::AlreadySettled));
        assert_eq!(htlc.claim(preimage), Err(HtlcError::AlreadySettled));
    }

    /// Step-counter delay (zero-seed sequential counter) gates cancel the same way.
    #[test]
    fn step_counter_delay_gates_cancel() {
        let preimage = b"step-counter";
        let mut htlc = funded_htlc(preimage, 999, 8);
        // Treat MockVdf purely as a step counter.
        assert_eq!(htlc.vdf.steps_completed, 0);
        assert_eq!(htlc.vdf.remaining(), 8);

        for i in 1..=7 {
            htlc.advance_vdf(1).unwrap();
            assert_eq!(htlc.vdf.steps_completed, i);
            assert_eq!(htlc.vdf_cancel(), Err(HtlcError::VdfNotComplete));
        }
        htlc.advance_vdf(1).unwrap();
        assert_eq!(htlc.vdf.steps_completed, 8);
        htlc.vdf_cancel().expect("cancel after step counter complete");
        assert_eq!(htlc.state, HtlcState::VdfCancelled);
    }
}
