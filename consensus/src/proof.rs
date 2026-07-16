//! Merkle proof generation and verification for the MossyMesh Patricia trie.
//!
//! Proofs are path-based: each step records enough sibling material to recompute
//! the parent hash bottom-up until the expected root is recovered.

use serde::{Deserialize, Serialize};

use crate::error::ConsensusError;
use crate::ipld_codec::{self, hash_branch, hash_extension, hash_leaf};
use crate::Hash32;

/// Terminal node that anchors the bottom of a proof (leaf, or branch-held value).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ProofTerminal {
    /// Standard leaf: remaining nibble path + value.
    Leaf { path: Vec<u8>, value: Vec<u8> },
    /// Value stored on a branch (key ends at this node). All 16 child hashes present.
    BranchValue {
        children: [Option<Hash32>; 16],
        value: Vec<u8>,
    },
}

/// One ancestor step along a nibble path from the terminal toward root.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ProofStep {
    /// Extension node: shared path prefix.
    Extension { path: Vec<u8> },
    /// Branch node we ascended through via `nibble` (not a value terminal).
    Branch {
        /// Nibble index (0..15) of the child we ascended from.
        nibble: u8,
        /// Full 16 slots; active child slot is `None` (filled during verify).
        children: [Option<Hash32>; 16],
        value: Option<Vec<u8>>,
    },
}

/// Merkle inclusion proof for a key → value binding under a root hash.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MerkleProof {
    /// Full key bytes (not nibbles) that were proven.
    pub key: Vec<u8>,
    /// Value bound at the terminal.
    pub value: Vec<u8>,
    /// Remaining leaf path nibbles (empty when terminal is a branch value).
    pub leaf_path: Vec<u8>,
    /// Terminal node at the bottom of the proof.
    pub terminal: ProofTerminal,
    /// Ancestor steps ordered terminal→root (bottom-up application order).
    pub steps: Vec<ProofStep>,
    /// DOC 32: flat sibling hash list collected from branch steps.
    pub siblings: Vec<Hash32>,
}

impl MerkleProof {
    /// Compact CBOR encoding of this proof (DAG-CBOR-ish).
    pub fn to_cbor(&self) -> Result<Vec<u8>, ConsensusError> {
        ipld_codec::encode_cbor(self)
    }

    pub fn from_cbor(bytes: &[u8]) -> Result<Self, ConsensusError> {
        ipld_codec::decode_cbor(bytes)
    }
}

/// Verify that `proof` authenticates `proof.key` → `proof.value` under `expected_root`.
///
/// Recomputes hashes bottom-up using Blake3-tagged CBOR node encodings.
pub fn verify_proof(proof: &MerkleProof, expected_root: &Hash32) -> Result<bool, ConsensusError> {
    // Value consistency check.
    match &proof.terminal {
        ProofTerminal::Leaf { value, .. } | ProofTerminal::BranchValue { value, .. } => {
            if value != &proof.value {
                return Ok(false);
            }
        }
    }

    let mut current = match &proof.terminal {
        ProofTerminal::Leaf { path, value } => hash_leaf(path, value)?,
        ProofTerminal::BranchValue { children, value } => {
            hash_branch(children, Some(value.as_slice()))?
        }
    };

    for step in &proof.steps {
        match step {
            ProofStep::Extension { path } => {
                if path.is_empty() {
                    return Err(ConsensusError::InvalidInput("empty extension path in proof"));
                }
                current = hash_extension(path, &current)?;
            }
            ProofStep::Branch {
                nibble,
                children,
                value,
            } => {
                if *nibble > 15 {
                    return Err(ConsensusError::InvalidInput("branch nibble out of range"));
                }
                let mut kids = *children;
                kids[*nibble as usize] = Some(current);
                current = hash_branch(&kids, value.as_deref())?;
            }
        }
    }

    Ok(current == *expected_root)
}

/// Convenience boolean API.
pub fn verify_proof_bool(proof: &MerkleProof, expected_root: &Hash32) -> bool {
    verify_proof(proof, expected_root).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trie::MerklePatriciaTrie;

    #[test]
    fn proof_verifies_for_inserted_key() {
        let mut t = MerklePatriciaTrie::new();
        t.insert(b"alice", b"100".to_vec()).unwrap();
        t.insert(b"bob", b"200".to_vec()).unwrap();
        t.insert(b"alice/payment", b"42".to_vec()).unwrap();

        let root = t.root_hash();
        let proof = t.prove(b"alice").unwrap();
        assert!(verify_proof(&proof, &root).unwrap());
        assert_eq!(proof.value, b"100");

        let proof2 = t.prove(b"alice/payment").unwrap();
        assert!(verify_proof(&proof2, &root).unwrap());
        assert_eq!(proof2.value, b"42");
    }

    #[test]
    fn tampered_proof_fails() {
        let mut t = MerklePatriciaTrie::new();
        t.insert(b"k", b"v".to_vec()).unwrap();
        let root = t.root_hash();
        let mut proof = t.prove(b"k").unwrap();
        proof.value = b"tampered".to_vec();
        // terminal value intentionally left stale → fails consistency check
        assert!(!verify_proof(&proof, &root).unwrap());
    }

    #[test]
    fn wrong_root_fails() {
        let mut t = MerklePatriciaTrie::new();
        t.insert(b"k", b"v".to_vec()).unwrap();
        let proof = t.prove(b"k").unwrap();
        let bogus = [9u8; 32];
        assert!(!verify_proof(&proof, &bogus).unwrap());
    }

    #[test]
    fn single_key_is_leaf_root() {
        let mut t = MerklePatriciaTrie::new();
        t.insert(b"solo", b"x".to_vec()).unwrap();
        let root = t.root_hash();
        let proof = t.prove(b"solo").unwrap();
        assert!(matches!(proof.terminal, ProofTerminal::Leaf { .. }));
        assert!(proof.steps.is_empty());
        assert!(verify_proof(&proof, &root).unwrap());
    }
}
