//! Light client: verify state without downloading the full trie (issue #34).
//!
//! A [`LightClient`] pins a trusted state root (genesis, checkpoint, or mesh
//! anchor) and verifies Merkle inclusion proofs against it. A mobile device
//! therefore only needs the 32-byte pinned root plus small per-key proofs,
//! never the trie itself. [`LightClient::retarget`] rotates the pinned root
//! when a newer trusted checkpoint arrives.

use crate::error::ConsensusError;
use crate::proof::{verify_proof, MerkleProof};
use crate::Hash32;

/// Light client over the ledger state trie.
///
/// Trust model: whoever supplies the pinned root is trusted (out-of-band
/// checkpoint, mesh anchor, or validator signature verified elsewhere).
/// Given that root, inclusion proofs are self-authenticating.
#[derive(Debug, Clone)]
pub struct LightClient {
    /// Trusted state root; every proof is checked against this.
    trusted_root: Hash32,
}

impl LightClient {
    /// Pin a trusted state root.
    pub fn new(trusted_root: Hash32) -> Self {
        Self { trusted_root }
    }

    /// The currently pinned root.
    pub fn trusted_root(&self) -> Hash32 {
        self.trusted_root
    }

    /// Rotate trust to a newer checkpoint root.
    pub fn retarget(&mut self, new_root: Hash32) {
        self.trusted_root = new_root;
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

    fn fixture() -> (MerklePatriciaTrie, LightClient) {
        let mut t = MerklePatriciaTrie::new();
        t.insert(b"alice", b"100".to_vec()).unwrap();
        t.insert(b"bob", b"200".to_vec()).unwrap();
        let root = t.root_hash();
        (t, LightClient::new(root))
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
    fn retarget_rotates_trust() {
        let (mut t, mut lc) = fixture();
        let old_proof = t.prove(b"bob").unwrap();
        assert!(lc.verify_inclusion_bool(&old_proof));

        t.insert(b"carol", b"300".to_vec()).unwrap();
        lc.retarget(t.root_hash());

        // Old proof is stale under the new root; new proofs verify.
        assert!(!lc.verify_inclusion_bool(&old_proof));
        let new_proof = t.prove(b"carol").unwrap();
        assert_eq!(lc.verify_inclusion(&new_proof).unwrap(), b"300");
    }
}
