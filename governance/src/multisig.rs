//! Admin multi-sig with mathematically decaying authority.
//!
//! Genesis: 3-of-5 admin multi-sig. Authority weight decays linearly from 1.0
//! at day 0 to 0.0 at day 90 ([`crate::ADMIN_DECAY_DAYS`]). After the window,
//! multi-sig proposals no longer pass regardless of signatures — control
//! belongs to ZK-blinded edge voting.
//!
//! # Cryptography status (honest stub)
//!
//! Signature verification is **deterministic domain-separated hashing**, not
//! production public-key crypto. [`Signature::forge`] builds
//! `SHA-256("mm-multisig-sig-v1" || signer || proposal_id || description_hash)`
//! and [`Signature::verify_against`] checks that digest plus admin membership.
//!
//! This is intentional for offline unit tests (no network, no key material).
//! Production must replace `forge` / `verify_against` with real ed25519 (or
//! similar) detached signatures over the same domain-separated message.
//! Threshold counting, proposal validation, and the decay schedule are fully
//! enforced and covered by tests independent of the crypto backend.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::wot::NodeId;
use crate::{ADMIN_DECAY_DAYS, MULTISIG_SIGNERS, MULTISIG_THRESHOLD};

/// Fixed-point scale for authority weight (weight = raw / WEIGHT_SCALE).
pub const WEIGHT_SCALE: u64 = 1_000_000;

/// Maximum UTF-8 bytes allowed in a proposal description.
pub const MAX_PROPOSAL_DESCRIPTION_LEN: usize = 512;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MultisigError {
    NotAnAdmin,
    DuplicateSigner,
    UnknownProposal,
    AlreadyExecuted,
    InsufficientAuthority,
    ThresholdNotMet,
    WrongSignerCount,
    /// Empty or oversize proposal description.
    InvalidProposal,
    /// Detached signature bytes failed stub (or future real) verification.
    InvalidSignature,
}

/// Opaque admin signature over a proposal (stub crypto — see module docs).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Signature {
    pub signer: NodeId,
    pub proposal_id: u64,
    /// Detached signature bytes. Stub scheme: 32-byte SHA-256 digest.
    pub bytes: Vec<u8>,
}

impl Signature {
    /// Domain-separated message hash bound to signer, proposal id, and body.
    pub fn message_digest(
        signer: &NodeId,
        proposal_id: u64,
        description_hash: &[u8; 32],
    ) -> [u8; 32] {
        let mut h = Sha256::new();
        h.update(b"mm-multisig-sig-v1");
        h.update(signer.0);
        h.update(proposal_id.to_le_bytes());
        h.update(description_hash);
        let out = h.finalize();
        let mut digest = [0u8; 32];
        digest.copy_from_slice(&out);
        digest
    }

    /// Hash of proposal description (length-prefixed UTF-8).
    pub fn description_hash(description: &str) -> [u8; 32] {
        let mut h = Sha256::new();
        h.update(b"mm-multisig-desc-v1");
        let bytes = description.as_bytes();
        h.update((bytes.len() as u64).to_le_bytes());
        h.update(bytes);
        let out = h.finalize();
        let mut digest = [0u8; 32];
        digest.copy_from_slice(&out);
        digest
    }

    /// Build a **stub** signature (deterministic hash, not real PK crypto).
    pub fn forge(signer: NodeId, proposal_id: u64, description: &str) -> Self {
        let dh = Self::description_hash(description);
        let digest = Self::message_digest(&signer, proposal_id, &dh);
        Self {
            signer,
            proposal_id,
            bytes: digest.to_vec(),
        }
    }

    /// Verify stub signature bytes against proposal description.
    pub fn verify_against(&self, description: &str) -> bool {
        if self.bytes.len() != 32 {
            return false;
        }
        let dh = Self::description_hash(description);
        let expected = Self::message_digest(&self.signer, self.proposal_id, &dh);
        self.bytes.as_slice() == expected.as_slice()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MultisigProposal {
    pub id: u64,
    pub description: String,
    pub executed: bool,
}

/// 3-of-5 admin multi-sig with time-decaying authority weight.
#[derive(Clone, Debug)]
pub struct AdminMultisig {
    admins: Vec<NodeId>,
    threshold: usize,
    /// Network day when the multi-sig was activated (genesis day).
    genesis_day: u64,
    proposals: HashMap<u64, MultisigProposal>,
    /// Distinct admin signers collected per proposal (threshold counting).
    signatures: HashMap<u64, HashSet<NodeId>>,
    next_id: u64,
}

impl AdminMultisig {
    /// Create a multi-sig with exactly [`MULTISIG_SIGNERS`] distinct admin keys.
    pub fn new(admins: Vec<NodeId>, genesis_day: u64) -> Result<Self, MultisigError> {
        if admins.len() != MULTISIG_SIGNERS {
            return Err(MultisigError::WrongSignerCount);
        }
        let unique: HashSet<_> = admins.iter().copied().collect();
        if unique.len() != MULTISIG_SIGNERS {
            return Err(MultisigError::DuplicateSigner);
        }
        Ok(Self {
            admins,
            threshold: MULTISIG_THRESHOLD,
            genesis_day,
            proposals: HashMap::new(),
            signatures: HashMap::new(),
            next_id: 1,
        })
    }

    pub fn admins(&self) -> &[NodeId] {
        &self.admins
    }

    pub fn threshold(&self) -> usize {
        self.threshold
    }

    pub fn is_admin(&self, node: &NodeId) -> bool {
        self.admins.contains(node)
    }

    /// Days elapsed since genesis at `current_day` (saturating).
    pub fn days_elapsed(&self, current_day: u64) -> u64 {
        current_day.saturating_sub(self.genesis_day)
    }

    /// Authority weight in fixed-point units: full `WEIGHT_SCALE` at day 0,
    /// linearly to 0 at day [`ADMIN_DECAY_DAYS`].
    ///
    /// `weight(t) = max(0, WEIGHT_SCALE * (DECAY_DAYS - t) / DECAY_DAYS)`
    pub fn authority_weight(&self, current_day: u64) -> u64 {
        authority_weight_at(self.days_elapsed(current_day))
    }

    /// True iff multi-sig still has non-zero authority.
    pub fn has_authority(&self, current_day: u64) -> bool {
        self.authority_weight(current_day) > 0
    }

    /// Validate proposal description rules (non-empty, within length bound).
    pub fn validate_description(description: &str) -> Result<(), MultisigError> {
        if description.is_empty() {
            return Err(MultisigError::InvalidProposal);
        }
        if description.len() > MAX_PROPOSAL_DESCRIPTION_LEN {
            return Err(MultisigError::InvalidProposal);
        }
        Ok(())
    }

    /// Create a proposal. Rejects empty or oversize descriptions.
    pub fn propose(&mut self, description: impl Into<String>) -> Result<u64, MultisigError> {
        let description = description.into();
        Self::validate_description(&description)?;
        let id = self.next_id;
        self.next_id += 1;
        self.proposals.insert(
            id,
            MultisigProposal {
                id,
                description,
                executed: false,
            },
        );
        self.signatures.insert(id, HashSet::new());
        Ok(id)
    }

    /// Record an admin signature on a proposal.
    ///
    /// Verifies stub signature bytes against the stored proposal description,
    /// enforces admin membership, and counts each admin at most once
    /// (threshold counting uses distinct signers).
    pub fn sign(&mut self, sig: Signature) -> Result<(), MultisigError> {
        if !self.is_admin(&sig.signer) {
            return Err(MultisigError::NotAnAdmin);
        }
        let proposal = self
            .proposals
            .get(&sig.proposal_id)
            .ok_or(MultisigError::UnknownProposal)?;
        if proposal.executed {
            return Err(MultisigError::AlreadyExecuted);
        }
        if !sig.verify_against(&proposal.description) {
            return Err(MultisigError::InvalidSignature);
        }
        let set = self
            .signatures
            .get_mut(&sig.proposal_id)
            .ok_or(MultisigError::UnknownProposal)?;
        if !set.insert(sig.signer) {
            return Err(MultisigError::DuplicateSigner);
        }
        Ok(())
    }

    /// Number of distinct admin signers recorded for `proposal_id`.
    pub fn signature_count(&self, proposal_id: u64) -> usize {
        self.signatures
            .get(&proposal_id)
            .map(|s| s.len())
            .unwrap_or(0)
    }

    /// True when distinct signer count meets or exceeds the threshold.
    pub fn threshold_met(&self, proposal_id: u64) -> bool {
        self.signature_count(proposal_id) >= self.threshold
    }

    /// Signers recorded for a proposal (empty if unknown).
    pub fn signers_of(&self, proposal_id: u64) -> Vec<NodeId> {
        self.signatures
            .get(&proposal_id)
            .map(|s| s.iter().copied().collect())
            .unwrap_or_default()
    }

    /// Execute if threshold met AND multi-sig still has authority at `current_day`.
    pub fn execute(&mut self, proposal_id: u64, current_day: u64) -> Result<(), MultisigError> {
        if self.authority_weight(current_day) == 0 {
            return Err(MultisigError::InsufficientAuthority);
        }
        let count = self.signature_count(proposal_id);
        let threshold = self.threshold;
        let proposal = self
            .proposals
            .get_mut(&proposal_id)
            .ok_or(MultisigError::UnknownProposal)?;
        if proposal.executed {
            return Err(MultisigError::AlreadyExecuted);
        }
        if count < threshold {
            return Err(MultisigError::ThresholdNotMet);
        }
        proposal.executed = true;
        Ok(())
    }

    pub fn proposal(&self, id: u64) -> Option<&MultisigProposal> {
        self.proposals.get(&id)
    }
}

/// Pure decay schedule used by [`AdminMultisig::authority_weight`].
///
/// Linear: `WEIGHT_SCALE * remaining / ADMIN_DECAY_DAYS`, clamped to 0 after the window.
pub fn authority_weight_at(days_elapsed: u64) -> u64 {
    if days_elapsed >= ADMIN_DECAY_DAYS {
        return 0;
    }
    let remaining = ADMIN_DECAY_DAYS - days_elapsed;
    (WEIGHT_SCALE as u128 * remaining as u128 / ADMIN_DECAY_DAYS as u128) as u64
}

/// Fractional authority in basis points (0–10_000) for display / tests.
pub fn authority_bps(days_elapsed: u64) -> u64 {
    authority_weight_at(days_elapsed) * 10_000 / WEIGHT_SCALE
}

#[cfg(test)]
mod tests {
    use super::*;

    fn five_admins() -> Vec<NodeId> {
        (0..5)
            .map(|i| NodeId::from_label(&format!("admin{i}")))
            .collect()
    }

    fn sign_n(ms: &mut AdminMultisig, id: u64, n: usize) {
        let desc = ms.proposal(id).unwrap().description.clone();
        for i in 0..n {
            let signer = NodeId::from_label(&format!("admin{i}"));
            ms.sign(Signature::forge(signer, id, &desc)).unwrap();
        }
    }

    #[test]
    fn decay_schedule_endpoints_and_midpoint() {
        // Day 0 of network life (elapsed 0): full weight
        assert_eq!(authority_weight_at(0), WEIGHT_SCALE);
        // Day 45: half remaining → 50%
        assert_eq!(authority_weight_at(45), WEIGHT_SCALE / 2);
        // Day 89: 1/90 remaining
        assert_eq!(
            authority_weight_at(89),
            (WEIGHT_SCALE as u128 * 1 / 90) as u64
        );
        // Day 90 and beyond: zero
        assert_eq!(authority_weight_at(90), 0);
        assert_eq!(authority_weight_at(1000), 0);

        assert_eq!(authority_bps(0), 10_000);
        assert_eq!(authority_bps(45), 5_000);
        assert_eq!(authority_bps(90), 0);
    }

    #[test]
    fn decay_is_monotone_non_increasing() {
        let mut prev = authority_weight_at(0);
        for d in 1..=ADMIN_DECAY_DAYS {
            let w = authority_weight_at(d);
            assert!(w <= prev, "day {d}: {w} > {prev}");
            prev = w;
        }
    }

    #[test]
    fn decay_timeline_exact_weights_at_key_days() {
        // weight(t) = WEIGHT_SCALE * (90 - t) / 90
        let samples: [(u64, u64); 7] = [
            (0, WEIGHT_SCALE),
            (9, (WEIGHT_SCALE as u128 * 81 / 90) as u64),
            (18, (WEIGHT_SCALE as u128 * 72 / 90) as u64),
            (30, (WEIGHT_SCALE as u128 * 60 / 90) as u64),
            (45, WEIGHT_SCALE / 2),
            (60, (WEIGHT_SCALE as u128 * 30 / 90) as u64),
            (89, (WEIGHT_SCALE as u128 * 1 / 90) as u64),
        ];
        for (day, expected) in samples {
            assert_eq!(
                authority_weight_at(day),
                expected,
                "mismatch at day {day}"
            );
        }
        // Last non-zero day still has authority; day 90 does not.
        let ms = AdminMultisig::new(five_admins(), 0).unwrap();
        assert!(ms.has_authority(89));
        assert!(!ms.has_authority(90));
        assert!(!ms.has_authority(91));
    }

    #[test]
    fn decay_respects_genesis_offset() {
        // Genesis at day 100 → elapsed at calendar day 145 is 45.
        let ms = AdminMultisig::new(five_admins(), 100).unwrap();
        assert_eq!(ms.days_elapsed(100), 0);
        assert_eq!(ms.days_elapsed(145), 45);
        assert_eq!(ms.authority_weight(145), WEIGHT_SCALE / 2);
        assert_eq!(ms.authority_weight(190), 0);
        // Before genesis: saturating elapsed = 0 → full weight
        assert_eq!(ms.days_elapsed(50), 0);
        assert_eq!(ms.authority_weight(50), WEIGHT_SCALE);
    }

    #[test]
    fn three_of_five_executes_while_authority_remains() {
        let mut ms = AdminMultisig::new(five_admins(), 0).unwrap();
        let id = ms.propose("bootstrap params").unwrap();
        sign_n(&mut ms, id, 3);
        assert!(ms.threshold_met(id));
        assert_eq!(ms.signature_count(id), 3);
        // Day 10 still has authority
        ms.execute(id, 10).expect("execute");
        assert!(ms.proposal(id).unwrap().executed);
    }

    #[test]
    fn threshold_counting_exact_boundary() {
        let mut ms = AdminMultisig::new(five_admins(), 0).unwrap();
        let id = ms.propose("boundary").unwrap();
        assert_eq!(ms.signature_count(id), 0);
        assert!(!ms.threshold_met(id));

        sign_n(&mut ms, id, 2);
        assert_eq!(ms.signature_count(id), 2);
        assert!(!ms.threshold_met(id));
        assert_eq!(ms.execute(id, 0), Err(MultisigError::ThresholdNotMet));

        // Third distinct signer crosses threshold
        let desc = ms.proposal(id).unwrap().description.clone();
        ms.sign(Signature::forge(NodeId::from_label("admin2"), id, &desc))
            .unwrap();
        assert_eq!(ms.signature_count(id), 3);
        assert!(ms.threshold_met(id));
        ms.execute(id, 0).unwrap();
    }

    #[test]
    fn threshold_counts_distinct_signers_only() {
        let mut ms = AdminMultisig::new(five_admins(), 0).unwrap();
        let id = ms.propose("no double count").unwrap();
        let desc = ms.proposal(id).unwrap().description.clone();
        let s0 = Signature::forge(NodeId::from_label("admin0"), id, &desc);
        ms.sign(s0.clone()).unwrap();
        assert_eq!(
            ms.sign(s0),
            Err(MultisigError::DuplicateSigner)
        );
        assert_eq!(ms.signature_count(id), 1);
        assert!(!ms.threshold_met(id));
    }

    #[test]
    fn five_of_five_still_executes_once() {
        let mut ms = AdminMultisig::new(five_admins(), 0).unwrap();
        let id = ms.propose("full quorum").unwrap();
        sign_n(&mut ms, id, 5);
        assert_eq!(ms.signature_count(id), 5);
        assert!(ms.threshold_met(id));
        ms.execute(id, 0).unwrap();
        assert_eq!(ms.execute(id, 0), Err(MultisigError::AlreadyExecuted));
    }

    #[test]
    fn execution_fails_after_full_decay() {
        let mut ms = AdminMultisig::new(five_admins(), 0).unwrap();
        let id = ms.propose("too late").unwrap();
        sign_n(&mut ms, id, 5);
        assert_eq!(
            ms.execute(id, 90),
            Err(MultisigError::InsufficientAuthority)
        );
        assert_eq!(
            ms.execute(id, 120),
            Err(MultisigError::InsufficientAuthority)
        );
        // Still not executed
        assert!(!ms.proposal(id).unwrap().executed);
    }

    #[test]
    fn execution_ok_on_last_day_with_authority() {
        let mut ms = AdminMultisig::new(five_admins(), 0).unwrap();
        let id = ms.propose("day 89 ok").unwrap();
        sign_n(&mut ms, id, 3);
        assert!(ms.has_authority(89));
        ms.execute(id, 89).unwrap();
        assert!(ms.proposal(id).unwrap().executed);
    }

    #[test]
    fn threshold_not_met() {
        let mut ms = AdminMultisig::new(five_admins(), 0).unwrap();
        let id = ms.propose("needs more sigs").unwrap();
        sign_n(&mut ms, id, 2);
        assert_eq!(ms.execute(id, 0), Err(MultisigError::ThresholdNotMet));
    }

    #[test]
    fn non_admin_cannot_sign() {
        let mut ms = AdminMultisig::new(five_admins(), 0).unwrap();
        let id = ms.propose("x").unwrap();
        let desc = ms.proposal(id).unwrap().description.clone();
        assert_eq!(
            ms.sign(Signature::forge(NodeId::from_label("intruder"), id, &desc)),
            Err(MultisigError::NotAnAdmin)
        );
    }

    #[test]
    fn reject_invalid_signature_bytes() {
        let mut ms = AdminMultisig::new(five_admins(), 0).unwrap();
        let id = ms.propose("signed body").unwrap();
        // Empty / wrong length
        assert_eq!(
            ms.sign(Signature {
                signer: NodeId::from_label("admin0"),
                proposal_id: id,
                bytes: vec![],
            }),
            Err(MultisigError::InvalidSignature)
        );
        // Wrong description binding
        assert_eq!(
            ms.sign(Signature::forge(
                NodeId::from_label("admin0"),
                id,
                "different body"
            )),
            Err(MultisigError::InvalidSignature)
        );
        // Tampered digest
        let mut bad = Signature::forge(
            NodeId::from_label("admin0"),
            id,
            "signed body",
        );
        bad.bytes[0] ^= 0xff;
        assert_eq!(ms.sign(bad), Err(MultisigError::InvalidSignature));
    }

    #[test]
    fn reject_invalid_proposals() {
        let mut ms = AdminMultisig::new(five_admins(), 0).unwrap();
        assert_eq!(ms.propose(""), Err(MultisigError::InvalidProposal));
        let too_long = "x".repeat(MAX_PROPOSAL_DESCRIPTION_LEN + 1);
        assert_eq!(
            ms.propose(too_long),
            Err(MultisigError::InvalidProposal)
        );
        // Boundary: exactly max length is ok
        let ok = "y".repeat(MAX_PROPOSAL_DESCRIPTION_LEN);
        let id = ms.propose(ok).unwrap();
        assert_eq!(id, 1);
    }

    #[test]
    fn reject_unknown_proposal_and_sign_after_execute() {
        let mut ms = AdminMultisig::new(five_admins(), 0).unwrap();
        let forged = Signature::forge(NodeId::from_label("admin0"), 99, "ghost");
        assert_eq!(ms.sign(forged), Err(MultisigError::UnknownProposal));
        assert_eq!(
            ms.execute(99, 0),
            Err(MultisigError::UnknownProposal)
        );

        let id = ms.propose("live").unwrap();
        sign_n(&mut ms, id, 3);
        ms.execute(id, 0).unwrap();
        let desc = ms.proposal(id).unwrap().description.clone();
        assert_eq!(
            ms.sign(Signature::forge(NodeId::from_label("admin3"), id, &desc)),
            Err(MultisigError::AlreadyExecuted)
        );
    }

    #[test]
    fn wrong_signer_count_and_duplicates_rejected_at_construction() {
        let four: Vec<_> = (0..4)
            .map(|i| NodeId::from_label(&format!("a{i}")))
            .collect();
        assert_eq!(
            AdminMultisig::new(four, 0).err(),
            Some(MultisigError::WrongSignerCount)
        );
        let mut dup = five_admins();
        dup[4] = dup[0];
        assert_eq!(
            AdminMultisig::new(dup, 0).err(),
            Some(MultisigError::DuplicateSigner)
        );
    }

    #[test]
    fn stub_signature_is_deterministic() {
        let a = NodeId::from_label("admin0");
        let s1 = Signature::forge(a, 7, "params");
        let s2 = Signature::forge(a, 7, "params");
        assert_eq!(s1, s2);
        assert!(s1.verify_against("params"));
        assert!(!s1.verify_against("params!"));
        // Different proposal id → different digest
        let s3 = Signature::forge(a, 8, "params");
        assert_ne!(s1.bytes, s3.bytes);
    }
}
