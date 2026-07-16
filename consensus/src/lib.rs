//! Consensus module for MossyMesh.
//!
//! Decentralized offline ledger built on an incremental **Merkle-Patricia Trie**,
//! Blake3 cryptographic pointers, and compact CBOR (DAG-CBOR-ish) node encoding.
//!
//! # Hashing policy
//! Digests are **Blake3** (32-byte). Chosen over SHA-2 for higher throughput on
//! constrained edge hardware while remaining fully deterministic across platforms.
//! Node hashes are `Blake3(tag || CBOR(payload))` with domain tags defined in
//! [`ipld_codec`].
//!
//! # Module layout
//! - [`trie`] — insert/get, root hash, size budget, legacy `TrieNode`
//! - [`proof`] — Merkle inclusion proofs + verification
//! - [`ipld_codec`] — CBOR encode/decode and tagged hashing
//! - [`error`] — [`ConsensusError`]
//! - [`crdt`] — YATA/RGA document merge (peer agent surface; already present)
//! - [`erasure`] / [`ring_buffer`] — DA helpers (peer agent surface)
//!
//! # Reserved for peer agents
//! - `snark.rs` — Nova folding / MicroSpartan (use [`SnarkFolder`] as the hook)

pub mod crdt;
pub mod error;
pub mod erasure;
pub mod ipld_codec;
pub mod proof;
pub mod ring_buffer;
pub mod trie;

// Peer-agent module (create when implementing nova-snark):
// pub mod snark;

pub use error::ConsensusError;
pub use ipld_codec::{decode_cbor, empty_root, encode_cbor, CryptoPointer};
pub use proof::{verify_proof, verify_proof_bool, MerkleProof, ProofStep, ProofTerminal};
pub use trie::{bytes_to_nibbles, MerklePatriciaTrie, MptNode, StateMerge, TrieNode};

/// 32-byte cryptographic digest / content pointer.
pub type Hash32 = [u8; 32];

/// DOC 35: Strict 10 MB memory limit for the active ledger on edge devices.
pub const MAX_LEDGER_SIZE: usize = 10_000_000;

/// Initialize the consensus subsystem (daemon boot hook).
pub fn init_consensus() {
    println!(
        "Consensus: Merkle-Patricia Trie ledger ready (Blake3, MAX_LEDGER_SIZE={} bytes)",
        MAX_LEDGER_SIZE
    );
}

// ─── SNARK hooks (stub surface for future snark.rs / Nova) ───────────────────

/// DOC 33: Serialized zero-knowledge proof payload (structure owned by snark agent).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnarkProof {
    pub proof_bytes: Vec<u8>,
}

/// Hook for recursive SNARK folding. Full Nova/Pallas implementation belongs in `snark.rs`.
pub trait SnarkFolder {
    /// Fold an additional proof into the accumulator.
    fn fold(&mut self, proof: &SnarkProof) -> Result<(), ConsensusError>;

    /// Verify the current accumulated proof against a public root/statement.
    fn verify(&self, expected_root: &Hash32) -> Result<bool, ConsensusError>;
}

/// Placeholder folder so callers can depend on the trait without Nova wired yet.
#[derive(Debug, Default)]
pub struct StubSnarkFolder {
    pub folds: usize,
}

impl SnarkFolder for StubSnarkFolder {
    fn fold(&mut self, proof: &SnarkProof) -> Result<(), ConsensusError> {
        if proof.proof_bytes.is_empty() {
            return Err(ConsensusError::SnarkError("empty proof".into()));
        }
        self.folds += 1;
        Ok(())
    }

    fn verify(&self, _expected_root: &Hash32) -> Result<bool, ConsensusError> {
        Ok(self.folds > 0)
    }
}

/// Legacy stub; prefer [`SnarkFolder::verify`].
pub fn verify_snark() -> bool {
    false
}

/// DOC 34: Nova-SNARK folding entrypoint placeholder (implement in snark.rs).
pub fn fold_snarks() {}

// ─── Legacy thin wrappers ────────────────────────────────────────────────────

/// Hash of the empty / sentinel node. Prefer [`empty_root`] / node methods.
pub fn hash_node() -> Hash32 {
    empty_root()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proof::verify_proof;
    use crate::trie::StateMerge;

    #[test]
    fn init_does_not_panic() {
        init_consensus();
    }

    #[test]
    fn end_to_end_insert_proof_merge() {
        let mut ledger = MerklePatriciaTrie::new();
        ledger.insert(b"account/1", b"1000".to_vec()).unwrap();
        ledger.insert(b"account/2", b"500".to_vec()).unwrap();

        let root = ledger.root_hash();
        let proof = ledger.prove(b"account/1").unwrap();
        assert!(verify_proof(&proof, &root).unwrap());

        let mut island = MerklePatriciaTrie::new();
        island.insert(b"account/3", b"42".to_vec()).unwrap();
        ledger.merge_with(&island).unwrap();
        assert_eq!(ledger.get(b"account/3").unwrap(), b"42");
        assert_ne!(ledger.root_hash(), root);
    }

    #[test]
    fn snark_hook_interface() {
        let mut folder = StubSnarkFolder::default();
        assert!(folder
            .fold(&SnarkProof {
                proof_bytes: vec![]
            })
            .is_err());
        folder
            .fold(&SnarkProof {
                proof_bytes: vec![1, 2, 3],
            })
            .unwrap();
        assert!(folder.verify(&[0u8; 32]).unwrap());
    }

    #[test]
    fn crypto_pointer_cbor_roundtrip() {
        let mut t = MerklePatriciaTrie::new();
        t.insert(b"p", b"v".to_vec()).unwrap();
        let ptr = CryptoPointer::from_hash(t.root_hash());
        let bytes = ptr.to_cbor().unwrap();
        let back = CryptoPointer::from_cbor(&bytes).unwrap();
        assert_eq!(ptr, back);
    }

    #[test]
    fn legacy_api_for_integration() {
        let mut a = TrieNode::new();
        let mut b = TrieNode::new();
        a.insert_node(&[0x01, 0x02], b"alpha".to_vec());
        b.insert_node(&[0x01, 0x03], b"beta".to_vec());
        a.merge_state(&b);
        assert!(a.children.contains_key(&0x01));
    }
}
