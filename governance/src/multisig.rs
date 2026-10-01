//! Admin multi-sig with mathematically decaying authority.
//!
//! Genesis: 3-of-5 admin multi-sig. Authority weight decays linearly from 1.0
//! at day 0 to 0.0 at day 90 ([`crate::ADMIN_DECAY_DAYS`]). After the window,
//! multi-sig proposals no longer pass regardless of signatures — control
//! belongs to ZK-blinded edge voting.
//!
//! # Cryptography
//!
//! Signatures are real ed25519 detached signatures over a domain-separated
//! message digest (`SHA-256("mm-multisig-sig-v1" || signer || proposal_id ||
//! description_hash)`). Each admin registers a verifying key at construction;
//! `sign` verifies the detached signature against the claimed signer's
//! registered key, so signatures cannot be forged by non-key-holders (#62).
//! [`Signature::forge`] remains as a test-only helper that builds the old
//! deterministic digest, and is not callable in production builds.
//!
//! Threshold counting, proposal validation, and the decay schedule are fully
//! enforced and covered by tests independent of the signature backend.

use std::collections::{HashMap, HashSet};

use ed25519_dalek::{Signature as DalekSignature, Signer, SigningKey, Verifier, VerifyingKey};
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

/// Opaque admin signature over a proposal.
///
/// `bytes` holds a 64-byte ed25519 detached signature over the
/// domain-separated message digest (see [`Signature::message_digest`]).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Signature {
    pub signer: NodeId,
    pub proposal_id: u64,
    /// Detached ed25519 signature bytes (64 bytes).
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

    /// Sign a proposal with an admin's ed25519 signing key (production path).
    ///
    /// Signs the domain-separated message digest; anyone holding only public
    /// inputs cannot produce valid bytes (fixes #62).
    pub fn sign_proposal(
        key: &SigningKey,
        signer: NodeId,
        proposal_id: u64,
        description: &str,
    ) -> Self {
        let dh = Self::description_hash(description);
        let digest = Self::message_digest(&signer, proposal_id, &dh);
        let sig: DalekSignature = key.sign(&digest);
        Self {
            signer,
            proposal_id,
            bytes: sig.to_bytes().to_vec(),
        }
    }

    /// Build the legacy stub "signature" (deterministic digest, not real PK
    /// crypto). Test-only: simulates what an attacker could previously forge.
    #[cfg(test)]
    pub fn forge(signer: NodeId, proposal_id: u64, description: &str) -> Self {
        let dh = Self::description_hash(description);
        let digest = Self::message_digest(&signer, proposal_id, &dh);
        Self {
            signer,
            proposal_id,
            bytes: digest.to_vec(),
        }
    }

    /// Verify ed25519 signature bytes against the proposal description and the
    /// signer's registered verifying key.
    pub fn verify_against(&self, description: &str, key: &VerifyingKey) -> bool {
        if self.bytes.len() != 64 {
            return false;
        }
        let sig = match DalekSignature::try_from(self.bytes.as_slice()) {
            Ok(sig) => sig,
            Err(_) => return false,
        };
        let dh = Self::description_hash(description);
        let digest = Self::message_digest(&self.signer, self.proposal_id, &dh);
        key.verify(&digest, &sig).is_ok()
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
    /// Registered ed25519 verifying key per admin; signatures are checked
    /// against the claimed signer's key.
    admin_keys: HashMap<NodeId, VerifyingKey>,
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
    pub fn new(
        admins: Vec<(NodeId, VerifyingKey)>,
        genesis_day: u64,
    ) -> Result<Self, MultisigError> {
        if admins.len() != MULTISIG_SIGNERS {
            return Err(MultisigError::WrongSignerCount);
        }
        let unique: HashSet<_> = admins.iter().map(|(n, _)| *n).collect();
        if unique.len() != MULTISIG_SIGNERS {
            return Err(MultisigError::DuplicateSigner);
        }
        let admin_keys: HashMap<NodeId, VerifyingKey> = admins.iter().cloned().collect();
        let admin_ids: Vec<NodeId> = admins.into_iter().map(|(n, _)| n).collect();
        Ok(Self {
            admins: admin_ids,
            admin_keys,
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
    /// Verifies the ed25519 signature bytes against the stored proposal
    /// description and the signer's registered verifying key, enforces admin
    /// membership, and counts each admin at most once (threshold counting
    /// uses distinct signers).
    pub fn sign(&mut self, sig: Signature) -> Result<(), MultisigError> {
        if !self.is_admin(&sig.signer) {
            return Err(MultisigError::NotAnAdmin);
        }
        let key: VerifyingKey = self
            .admin_keys
            .get(&sig.signer)
            .cloned()
            .ok_or(MultisigError::NotAnAdmin)?;
        let proposal = self
            .proposals
            .get(&sig.proposal_id)
            .ok_or(MultisigError::UnknownProposal)?;
        if proposal.executed {
            return Err(MultisigError::AlreadyExecuted);
        }
        if !sig.verify_against(&proposal.description, &key) {
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

    /// Five admins with real ed25519 keys: (node id, signing key).
    fn five_admin_keys() -> Vec<(NodeId, SigningKey)> {
        (0..5)
            .map(|i| {
                let node = NodeId::from_label(&format!("admin{i}"));
                let sk = SigningKey::from_bytes(&[(i as u8) + 1; 32]);
                (node, sk)
            })
            .collect()
    }

    fn five_admins() -> Vec<(NodeId, VerifyingKey)> {
        five_admin_keys()
            .into_iter()
            .map(|(n, sk)| (n, sk.verifying_key()))
            .collect()
    }

    fn sign_n(ms: &mut AdminMultisig, keys: &[(NodeId, SigningKey)], id: u64, n: usize) {
        let desc = ms.proposal(id).unwrap().description.clone();
        for (node, sk) in keys.iter().take(n) {
            ms.sign(Signature::sign_proposal(sk, *node, id, &desc))
                .unwrap();
        }
    }

    #[test]
    fn decay_schedule_endpoints_and_midpoint() {
        // Day 0 of network life (elapsed 0): full weight
        assert_eq!(authority_weight_at(0), WEIGHT_SCALE);
        // Day 45: half remaining → 50%
        assert_eq!(authority_weight_at(45), WEIGHT_SCALE / 2);
        // Day 89: 1/90 remaining
        assert_eq!(authority_weight_at(89), (WEIGHT_SCALE as u128 / 90) as u64);
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
            (89, (WEIGHT_SCALE as u128 / 90) as u64),
        ];
        for (day, expected) in samples {
            assert_eq!(authority_weight_at(day), expected, "mismatch at day {day}");
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
        let keys = five_admin_keys();
        let mut ms = AdminMultisig::new(five_admins(), 0).unwrap();
        let id = ms.propose("bootstrap params").unwrap();
        sign_n(&mut ms, &keys, id, 3);
        assert!(ms.threshold_met(id));
        assert_eq!(ms.signature_count(id), 3);
        // Day 10 still has authority
        ms.execute(id, 10).expect("execute");
        assert!(ms.proposal(id).unwrap().executed);
    }

    #[test]
    fn threshold_counting_exact_boundary() {
        let keys = five_admin_keys();
        let mut ms = AdminMultisig::new(five_admins(), 0).unwrap();
        let id = ms.propose("boundary").unwrap();
        assert_eq!(ms.signature_count(id), 0);
        assert!(!ms.threshold_met(id));

        sign_n(&mut ms, &keys, id, 2);
        assert_eq!(ms.signature_count(id), 2);
        assert!(!ms.threshold_met(id));
        assert_eq!(ms.execute(id, 0), Err(MultisigError::ThresholdNotMet));

        // Third distinct signer crosses threshold
        let desc = ms.proposal(id).unwrap().description.clone();
        let (node2, sk2) = &keys[2];
        ms.sign(Signature::sign_proposal(sk2, *node2, id, &desc))
            .unwrap();
        assert_eq!(ms.signature_count(id), 3);
        assert!(ms.threshold_met(id));
        ms.execute(id, 0).unwrap();
    }

    #[test]
    fn threshold_counts_distinct_signers_only() {
        let keys = five_admin_keys();
        let mut ms = AdminMultisig::new(five_admins(), 0).unwrap();
        let id = ms.propose("no double count").unwrap();
        let desc = ms.proposal(id).unwrap().description.clone();
        let (node0, sk0) = &keys[0];
        let s0 = Signature::sign_proposal(sk0, *node0, id, &desc);
        ms.sign(s0.clone()).unwrap();
        assert_eq!(ms.sign(s0), Err(MultisigError::DuplicateSigner));
        assert_eq!(ms.signature_count(id), 1);
        assert!(!ms.threshold_met(id));
    }

    #[test]
    fn five_of_five_still_executes_once() {
        let keys = five_admin_keys();
        let mut ms = AdminMultisig::new(five_admins(), 0).unwrap();
        let id = ms.propose("full quorum").unwrap();
        sign_n(&mut ms, &keys, id, 5);
        assert_eq!(ms.signature_count(id), 5);
        assert!(ms.threshold_met(id));
        ms.execute(id, 0).unwrap();
        assert_eq!(ms.execute(id, 0), Err(MultisigError::AlreadyExecuted));
    }

    #[test]
    fn execution_fails_after_full_decay() {
        let keys = five_admin_keys();
        let mut ms = AdminMultisig::new(five_admins(), 0).unwrap();
        let id = ms.propose("too late").unwrap();
        sign_n(&mut ms, &keys, id, 5);
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
        let keys = five_admin_keys();
        let mut ms = AdminMultisig::new(five_admins(), 0).unwrap();
        let id = ms.propose("day 89 ok").unwrap();
        sign_n(&mut ms, &keys, id, 3);
        assert!(ms.has_authority(89));
        ms.execute(id, 89).unwrap();
        assert!(ms.proposal(id).unwrap().executed);
    }

    #[test]
    fn threshold_not_met() {
        let keys = five_admin_keys();
        let mut ms = AdminMultisig::new(five_admins(), 0).unwrap();
        let id = ms.propose("needs more sigs").unwrap();
        sign_n(&mut ms, &keys, id, 2);
        assert_eq!(ms.execute(id, 0), Err(MultisigError::ThresholdNotMet));
    }

    #[test]
    fn non_admin_cannot_sign() {
        let _keys = five_admin_keys();
        let mut ms = AdminMultisig::new(five_admins(), 0).unwrap();
        let id = ms.propose("x").unwrap();
        let desc = ms.proposal(id).unwrap().description.clone();
        let intruder_sk = SigningKey::from_bytes(&[0xAA; 32]);
        let intruder = NodeId::from_label("intruder");
        assert_eq!(
            ms.sign(Signature::sign_proposal(&intruder_sk, intruder, id, &desc)),
            Err(MultisigError::NotAnAdmin)
        );
    }

    #[test]
    fn reject_invalid_signature_bytes() {
        let keys = five_admin_keys();
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
        // Signed over a different body
        let (node0, sk0) = &keys[0];
        assert_eq!(
            ms.sign(Signature::sign_proposal(sk0, *node0, id, "different body")),
            Err(MultisigError::InvalidSignature)
        );
        // Tampered signature bytes
        let mut bad = Signature::sign_proposal(sk0, *node0, id, "signed body");
        bad.bytes[0] ^= 0xff;
        assert_eq!(ms.sign(bad), Err(MultisigError::InvalidSignature));
    }

    #[test]
    fn reject_invalid_proposals() {
        let mut ms = AdminMultisig::new(five_admins(), 0).unwrap();
        assert_eq!(ms.propose(""), Err(MultisigError::InvalidProposal));
        let too_long = "x".repeat(MAX_PROPOSAL_DESCRIPTION_LEN + 1);
        assert_eq!(ms.propose(too_long), Err(MultisigError::InvalidProposal));
        // Boundary: exactly max length is ok
        let ok = "y".repeat(MAX_PROPOSAL_DESCRIPTION_LEN);
        let id = ms.propose(ok).unwrap();
        assert_eq!(id, 1);
    }

    #[test]
    fn reject_unknown_proposal_and_sign_after_execute() {
        let keys = five_admin_keys();
        let mut ms = AdminMultisig::new(five_admins(), 0).unwrap();
        let (node0, sk0) = &keys[0];
        let sig = Signature::sign_proposal(sk0, *node0, 99, "ghost");
        assert_eq!(ms.sign(sig), Err(MultisigError::UnknownProposal));
        assert_eq!(ms.execute(99, 0), Err(MultisigError::UnknownProposal));

        let id = ms.propose("live").unwrap();
        sign_n(&mut ms, &keys, id, 3);
        ms.execute(id, 0).unwrap();
        let desc = ms.proposal(id).unwrap().description.clone();
        let (node3, sk3) = &keys[3];
        assert_eq!(
            ms.sign(Signature::sign_proposal(sk3, *node3, id, &desc)),
            Err(MultisigError::AlreadyExecuted)
        );
    }

    #[test]
    fn wrong_signer_count_and_duplicates_rejected_at_construction() {
        let four: Vec<_> = (0..4)
            .map(|i| {
                let node = NodeId::from_label(&format!("a{i}"));
                let sk = SigningKey::from_bytes(&[(i as u8) + 1; 32]);
                (node, sk.verifying_key())
            })
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
    fn ed25519_signatures_are_deterministic_and_bound() {
        let keys = five_admin_keys();
        let (node0, sk0) = &keys[0];
        let s1 = Signature::sign_proposal(sk0, *node0, 7, "params");
        let s2 = Signature::sign_proposal(sk0, *node0, 7, "params");
        assert_eq!(s1, s2);
        assert_eq!(s1.bytes.len(), 64);
        let vk = sk0.verifying_key();
        assert!(s1.verify_against("params", &vk));
        assert!(!s1.verify_against("params!", &vk));
        // Different proposal id signs a different message
        let s3 = Signature::sign_proposal(sk0, *node0, 8, "params");
        assert_ne!(s1.bytes, s3.bytes);
        // A different admin's key does not verify admin0's signature
        let other_vk = keys[1].1.verifying_key();
        assert!(!s1.verify_against("params", &other_vk));
    }

    #[test]
    fn forged_stub_digest_no_longer_verifies() {
        // The old attack: anyone could build the deterministic digest for any
        // admin (Signature::forge, now test-only) and pass verification.
        let keys = five_admin_keys();
        let mut ms = AdminMultisig::new(five_admins(), 0).unwrap();
        let id = ms.propose("takeover").unwrap();
        let desc = ms.proposal(id).unwrap().description.clone();
        let admin0 = NodeId::from_label("admin0");
        let forged = Signature::forge(admin0, id, &desc);
        assert_eq!(
            ms.sign(forged.clone()),
            Err(MultisigError::InvalidSignature)
        );
        // Three forged "signatures" cannot reach threshold either.
        for i in 0..3 {
            let f = Signature::forge(NodeId::from_label(&format!("admin{i}")), id, &desc);
            assert_eq!(ms.sign(f), Err(MultisigError::InvalidSignature));
        }
        assert!(!ms.threshold_met(id));
        assert_eq!(ms.execute(id, 0), Err(MultisigError::ThresholdNotMet));
        // Real signatures still work after the attack attempt.
        sign_n(&mut ms, &keys, id, 3);
        ms.execute(id, 0).unwrap();
    }

    #[test]
    fn signature_from_wrong_key_claiming_admin_rejected() {
        let _keys = five_admin_keys();
        let mut ms = AdminMultisig::new(five_admins(), 0).unwrap();
        let id = ms.propose("impersonate").unwrap();
        let desc = ms.proposal(id).unwrap().description.clone();
        // Attacker signs with their own key but claims to be admin0.
        let attacker_sk = SigningKey::from_bytes(&[0xBB; 32]);
        let admin0 = NodeId::from_label("admin0");
        let impersonated = Signature::sign_proposal(&attacker_sk, admin0, id, &desc);
        assert_eq!(ms.sign(impersonated), Err(MultisigError::InvalidSignature));
    }
}
