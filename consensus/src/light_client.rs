//! Light client: verify state without downloading the full trie (issue #34).
//!
//! A [`LightClient`] pins a trusted state root (genesis, checkpoint, or mesh
//! anchor) and verifies Merkle inclusion proofs against it. A mobile device
//! therefore only needs the 32-byte pinned root plus small per-key proofs,
//! never the trie itself.
//!
//! [`LightClient::retarget`] rotates the pinned root when a newer trusted
//! checkpoint arrives. Retargeting is gated (issue #185): the new root must
//! arrive inside a [`Checkpoint`] carrying a quorum of validator-set ed25519
//! signatures over a canonical message, a strictly greater height than the
//! pinned one, and an issuance time inside the freshness window. A bare,
//! unsigned, replayed, or backdated root can no longer eclipse the client.

use std::collections::HashSet;

use ed25519_dalek::{Signature, Verifier, VerifyingKey};

use crate::error::ConsensusError;
use crate::proof::{verify_proof, MerkleProof};
use crate::Hash32;

/// Default freshness window for checkpoints: 24h.
pub const DEFAULT_FRESHNESS_SECS: u64 = 24 * 3600;

/// Allowed clock skew for checkpoint issuance times.
pub const MAX_CLOCK_SKEW_SECS: u64 = 300;

/// Wall-clock time as unix seconds. Used only for checkpoint freshness, not
/// for consensus-critical ordering.
fn unix_now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// A signed checkpoint: a state root bound to a monotonic height and an
/// issuance time, endorsed by the validator set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checkpoint {
    /// Monotonic checkpoint height. Must exceed the pinned height to retarget.
    pub height: u64,
    /// State root the validators endorse at this height.
    pub root: Hash32,
    /// Unix timestamp (seconds) when validators issued this checkpoint.
    pub issued_at_secs: u64,
}

impl Checkpoint {
    /// Canonical message bytes covered by every validator signature.
    pub fn message_bytes(&self) -> Vec<u8> {
        let mut m = Vec::with_capacity(22 + 8 + 32 + 8);
        m.extend_from_slice(b"mossymesh/checkpoint/v1");
        m.extend_from_slice(&self.height.to_le_bytes());
        m.extend_from_slice(&self.root);
        m.extend_from_slice(&self.issued_at_secs.to_le_bytes());
        m
    }
}

/// One validator's endorsement of a checkpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckpointSignature {
    /// Index into the light client's validator set. The key is looked up
    /// client-side, so a peer cannot smuggle in its own key.
    pub validator_index: usize,
    /// ed25519 signature over [`Checkpoint::message_bytes`].
    pub signature: [u8; 64],
}

/// Light client over the ledger state trie.
///
/// Trust model: the pinned root is trusted, and rotation to a new root is
/// trusted only when a quorum of the configured validator set signed the
/// checkpoint. Without a configured validator set, [`LightClient::retarget`]
/// refuses every rotation.
#[derive(Debug, Clone)]
pub struct LightClient {
    /// Trusted state root; every proof is checked against this.
    trusted_root: Hash32,
    /// Height of the pinned checkpoint (genesis pin = 0).
    trusted_height: u64,
    /// Validator set whose signatures endorse checkpoints.
    validators: Vec<VerifyingKey>,
    /// Distinct valid signatures required to accept a checkpoint.
    quorum: usize,
    /// Max age of a checkpoint relative to wall-clock (seconds).
    freshness_secs: u64,
}

impl LightClient {
    /// Pin a trusted (genesis) state root with no validator set yet.
    ///
    /// Call [`with_validators`](Self::with_validators) before
    /// [`retarget`](Self::retarget): rotation without a configured validator
    /// set is rejected (issue #185).
    pub fn new(trusted_root: Hash32) -> Self {
        Self {
            trusted_root,
            trusted_height: 0,
            validators: Vec::new(),
            quorum: 0,
            freshness_secs: DEFAULT_FRESHNESS_SECS,
        }
    }

    /// Configure the validator set and quorum (e.g. 2f+1 of 3f+1).
    pub fn with_validators(
        mut self,
        validators: Vec<VerifyingKey>,
        quorum: usize,
    ) -> Result<Self, ConsensusError> {
        if validators.is_empty() {
            return Err(ConsensusError::InvalidInput(
                "validator set must not be empty",
            ));
        }
        if quorum == 0 || quorum > validators.len() {
            return Err(ConsensusError::InvalidInput(
                "quorum must be between 1 and the validator set size",
            ));
        }
        self.validators = validators;
        self.quorum = quorum;
        Ok(self)
    }

    /// Override the checkpoint freshness window (seconds).
    pub fn with_freshness_secs(mut self, freshness_secs: u64) -> Self {
        self.freshness_secs = freshness_secs;
        self
    }

    /// The currently pinned root.
    pub fn trusted_root(&self) -> Hash32 {
        self.trusted_root
    }

    /// Height of the currently pinned checkpoint.
    pub fn trusted_height(&self) -> u64 {
        self.trusted_height
    }

    /// Rotate trust to a newer checkpoint root.
    ///
    /// Accepts only if ALL of the following hold (issue #185):
    /// - the checkpoint height is strictly greater than the pinned height
    ///   (monotonic; replays and rollbacks rejected),
    /// - the checkpoint was issued within the freshness window and is not
    ///   from the future beyond clock skew,
    /// - at least `quorum` distinct validators signed the canonical
    ///   checkpoint message.
    ///
    /// The pinned root changes only after every check passes.
    pub fn retarget(
        &mut self,
        checkpoint: &Checkpoint,
        signatures: &[CheckpointSignature],
    ) -> Result<(), ConsensusError> {
        if self.validators.is_empty() || self.quorum == 0 {
            return Err(ConsensusError::InvalidInput(
                "retarget requires a configured validator set",
            ));
        }
        if checkpoint.height <= self.trusted_height {
            return Err(ConsensusError::InvalidInput(
                "checkpoint height must exceed the pinned height",
            ));
        }
        let now = unix_now_secs();
        if checkpoint
            .issued_at_secs
            .saturating_add(self.freshness_secs)
            < now
        {
            return Err(ConsensusError::InvalidInput(
                "checkpoint is older than the freshness bound",
            ));
        }
        if checkpoint.issued_at_secs > now.saturating_add(MAX_CLOCK_SKEW_SECS) {
            return Err(ConsensusError::InvalidInput(
                "checkpoint issued too far in the future",
            ));
        }
        self.verify_quorum(checkpoint, signatures)?;

        self.trusted_root = checkpoint.root;
        self.trusted_height = checkpoint.height;
        Ok(())
    }

    /// Count distinct valid validator endorsements; require quorum.
    fn verify_quorum(
        &self,
        checkpoint: &Checkpoint,
        signatures: &[CheckpointSignature],
    ) -> Result<(), ConsensusError> {
        let msg = checkpoint.message_bytes();
        let mut seen = HashSet::new();
        let mut valid = 0usize;
        for sig in signatures {
            let key =
                self.validators
                    .get(sig.validator_index)
                    .ok_or(ConsensusError::InvalidInput(
                        "signature references an unknown validator",
                    ))?;
            // Each validator counts once; duplicates do not inflate the tally.
            if !seen.insert(sig.validator_index) {
                continue;
            }
            let signature = Signature::from_bytes(&sig.signature);
            if key.verify(&msg, &signature).is_ok() {
                valid += 1;
            }
        }
        if valid < self.quorum {
            return Err(ConsensusError::InvalidProof);
        }
        Ok(())
    }

    /// Verify that `proof` authenticates `proof.key` → `proof.value` under
    /// the pinned root. Returns the proven value on success, so callers get
    /// the value and its authentication in one step.
    pub fn verify_inclusion(&self, proof: &MerkleProof) -> Result<Vec<u8>, ConsensusError> {
        if !verify_proof(proof, &self.trusted_root)? {
            return Err(ConsensusError::InvalidProof);
        }
        Ok(proof.value.clone())
    }

    /// Convenience boolean API (never errors; malformed proofs are `false`).
    pub fn verify_inclusion_bool(&self, proof: &MerkleProof) -> bool {
        self.verify_inclusion(proof).is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trie::MerklePatriciaTrie;
    use ed25519_dalek::{Signer, SigningKey};

    fn fixture() -> (MerklePatriciaTrie, LightClient) {
        let mut t = MerklePatriciaTrie::new();
        t.insert(b"alice", b"100".to_vec()).unwrap();
        t.insert(b"bob", b"200".to_vec()).unwrap();
        let root = t.root_hash();
        (t, LightClient::new(root))
    }

    /// Deterministic test validator keys.
    fn signing_keys(n: usize) -> Vec<SigningKey> {
        (0..n)
            .map(|i| SigningKey::from_bytes(&[(i as u8).wrapping_add(1); 32]))
            .collect()
    }

    fn verifying_keys(sks: &[SigningKey]) -> Vec<VerifyingKey> {
        sks.iter().map(|sk| sk.verifying_key()).collect()
    }

    /// Client trusting genesis, with a 3-validator set and quorum 2.
    fn gated_client(root: Hash32) -> (LightClient, Vec<SigningKey>) {
        let sks = signing_keys(3);
        let lc = LightClient::new(root)
            .with_validators(verifying_keys(&sks), 2)
            .expect("valid quorum");
        (lc, sks)
    }

    fn sign(sks: &[SigningKey], idx: usize, cp: &Checkpoint) -> CheckpointSignature {
        let sig = sks[idx].sign(&cp.message_bytes());
        CheckpointSignature {
            validator_index: idx,
            signature: sig.to_bytes(),
        }
    }

    fn fresh_checkpoint(height: u64, root: Hash32) -> Checkpoint {
        Checkpoint {
            height,
            root,
            issued_at_secs: unix_now_secs(),
        }
    }

    #[test]
    fn inclusion_proof_verifies_against_pinned_root() {
        let (t, lc) = fixture();
        let proof = t.prove(b"alice").unwrap();
        assert_eq!(lc.verify_inclusion(&proof).unwrap(), b"100");
    }

    #[test]
    fn wrong_root_rejects_proof() {
        let (t, _) = fixture();
        let proof = t.prove(b"alice").unwrap();
        let lc = LightClient::new([0xFFu8; 32]);
        assert!(matches!(
            lc.verify_inclusion(&proof),
            Err(ConsensusError::InvalidProof)
        ));
        assert!(!lc.verify_inclusion_bool(&proof));
    }

    #[test]
    fn retarget_rotates_trust_with_quorum() {
        let (_t, lc) = fixture();
        let root = lc.trusted_root();
        let (mut lc, sks) = gated_client(root);
        let old_proof = {
            let mut t2 = MerklePatriciaTrie::new();
            t2.insert(b"alice", b"100".to_vec()).unwrap();
            t2.insert(b"bob", b"200".to_vec()).unwrap();
            t2.prove(b"bob").unwrap()
        };
        assert!(lc.verify_inclusion_bool(&old_proof));

        let mut t3 = MerklePatriciaTrie::new();
        t3.insert(b"alice", b"100".to_vec()).unwrap();
        t3.insert(b"bob", b"200".to_vec()).unwrap();
        t3.insert(b"carol", b"300".to_vec()).unwrap();
        let cp = fresh_checkpoint(1, t3.root_hash());
        let sigs = vec![sign(&sks, 0, &cp), sign(&sks, 1, &cp)];
        lc.retarget(&cp, &sigs).expect("quorum-signed retarget");
        assert_eq!(lc.trusted_height(), 1);

        // Old proof is stale under the new root; new proofs verify.
        assert!(!lc.verify_inclusion_bool(&old_proof));
        let new_proof = t3.prove(b"carol").unwrap();
        assert_eq!(lc.verify_inclusion(&new_proof).unwrap(), b"300");
    }

    #[test]
    fn retarget_rejects_unsigned_checkpoint() {
        // Issue #185: a bare root with no signatures must not eclipse the client.
        let (t, _) = fixture();
        let (mut lc, _) = gated_client(t.root_hash());
        let cp = fresh_checkpoint(1, [0xAAu8; 32]);
        assert_eq!(lc.retarget(&cp, &[]), Err(ConsensusError::InvalidProof));
        assert_eq!(lc.trusted_root(), t.root_hash());
    }

    #[test]
    fn retarget_requires_quorum() {
        let (t, _) = fixture();
        let (mut lc, sks) = gated_client(t.root_hash());
        let cp = fresh_checkpoint(1, [0xBBu8; 32]);
        // Only 1 of the required 2 signatures.
        let sigs = vec![sign(&sks, 0, &cp)];
        assert_eq!(lc.retarget(&cp, &sigs), Err(ConsensusError::InvalidProof));
        assert_eq!(lc.trusted_root(), t.root_hash());
    }

    #[test]
    fn retarget_rejects_stale_height() {
        let (t, _) = fixture();
        let (mut lc, sks) = gated_client(t.root_hash());
        // Same height as the pinned genesis (0): replay rejected.
        let cp = fresh_checkpoint(0, [0xCCu8; 32]);
        let sigs = vec![sign(&sks, 0, &cp), sign(&sks, 1, &cp)];
        assert_eq!(
            lc.retarget(&cp, &sigs),
            Err(ConsensusError::InvalidInput(
                "checkpoint height must exceed the pinned height"
            ))
        );
        assert_eq!(lc.trusted_root(), t.root_hash());
    }

    #[test]
    fn retarget_rejects_rollback_after_advance() {
        let (t, _) = fixture();
        let (mut lc, sks) = gated_client(t.root_hash());
        let cp1 = fresh_checkpoint(1, [0xDDu8; 32]);
        let sigs1 = vec![sign(&sks, 0, &cp1), sign(&sks, 1, &cp1)];
        lc.retarget(&cp1, &sigs1).unwrap();
        // An attacker replays the older height-1 checkpoint after we moved on.
        let rollback = fresh_checkpoint(1, [0xEEu8; 32]);
        let sigs2 = vec![sign(&sks, 0, &rollback), sign(&sks, 1, &rollback)];
        assert!(lc.retarget(&rollback, &sigs2).is_err());
        assert_eq!(lc.trusted_root(), [0xDDu8; 32]);
    }

    #[test]
    fn retarget_rejects_ancient_checkpoint() {
        // A validly signed but ancient checkpoint (replay of old state).
        let (t, _) = fixture();
        let (mut lc, sks) = gated_client(t.root_hash());
        let ancient = Checkpoint {
            height: 1,
            root: [0xFFu8; 32],
            issued_at_secs: unix_now_secs().saturating_sub(DEFAULT_FRESHNESS_SECS + 3600),
        };
        let sigs = vec![sign(&sks, 0, &ancient), sign(&sks, 1, &ancient)];
        assert_eq!(
            lc.retarget(&ancient, &sigs),
            Err(ConsensusError::InvalidInput(
                "checkpoint is older than the freshness bound"
            ))
        );
        assert_eq!(lc.trusted_root(), t.root_hash());
    }

    #[test]
    fn retarget_rejects_future_checkpoint() {
        let (t, _) = fixture();
        let (mut lc, sks) = gated_client(t.root_hash());
        let future = Checkpoint {
            height: 1,
            root: [0x11u8; 32],
            issued_at_secs: unix_now_secs() + MAX_CLOCK_SKEW_SECS + 3600,
        };
        let sigs = vec![sign(&sks, 0, &future), sign(&sks, 1, &future)];
        assert!(lc.retarget(&future, &sigs).is_err());
        assert_eq!(lc.trusted_root(), t.root_hash());
    }

    #[test]
    fn retarget_rejects_duplicate_validator_signatures() {
        // Same validator signing twice counts once: quorum not reached.
        let (t, _) = fixture();
        let (mut lc, sks) = gated_client(t.root_hash());
        let cp = fresh_checkpoint(1, [0x22u8; 32]);
        let sigs = vec![sign(&sks, 0, &cp), sign(&sks, 0, &cp)];
        assert_eq!(lc.retarget(&cp, &sigs), Err(ConsensusError::InvalidProof));
    }

    #[test]
    fn retarget_rejects_foreign_key_signature() {
        // Signature from a key outside the validator set does not count.
        let (t, _) = fixture();
        let (mut lc, sks) = gated_client(t.root_hash());
        let outsider = SigningKey::from_bytes(&[0x9Bu8; 32]);
        let cp = fresh_checkpoint(1, [0x33u8; 32]);
        let mut sigs = vec![sign(&sks, 0, &cp)];
        let foreign = outsider.sign(&cp.message_bytes()).to_bytes();
        sigs.push(CheckpointSignature {
            validator_index: 1,
            signature: foreign,
        });
        assert_eq!(lc.retarget(&cp, &sigs), Err(ConsensusError::InvalidProof));
    }

    #[test]
    fn retarget_without_validators_is_rejected() {
        // The old unconditional-retarget API is gone: no validator set, no rotation.
        let (mut t, mut lc) = fixture();
        t.insert(b"carol", b"300".to_vec()).unwrap();
        let cp = fresh_checkpoint(1, t.root_hash());
        assert_eq!(
            lc.retarget(&cp, &[]),
            Err(ConsensusError::InvalidInput(
                "retarget requires a configured validator set"
            ))
        );
    }

    #[test]
    fn with_validators_rejects_bad_quorum() {
        let (_, lc) = fixture();
        let sks = signing_keys(3);
        assert!(lc.with_validators(verifying_keys(&sks), 0).is_err());
        let (_, lc2) = fixture();
        assert!(lc2.with_validators(verifying_keys(&sks), 4).is_err());
        let (_, lc3) = fixture();
        assert!(lc3.with_validators(vec![], 1).is_err());
    }
}
