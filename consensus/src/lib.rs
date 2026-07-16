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
//! SNARK folding commitments (in [`snark`] / [`folding`]) use domain-separated
//! SHA-256 mock hashes so proof layouts stay independent of the trie hash function.
//!
//! # Module layout
//! - [`trie`] — insert/get, root hash, size budget, legacy `TrieNode`
//! - [`proof`] — Merkle inclusion proofs + verification
//! - [`ipld_codec`] — CBOR encode/decode and tagged hashing
//! - [`error`] — [`ConsensusError`]
//! - [`crdt`] — YATA/RGA document merge (peer agent surface; already present)
//! - [`erasure`] / [`ring_buffer`] — DA helpers (peer agent surface)
//! - [`snark`] — constant-size public SNARK types (200-byte radio anchors)
//! - [`folding`] — Nova-style recursive fold of ledger steps (constant proof size)

pub mod crdt;
pub mod error;
pub mod erasure;
pub mod folding;
pub mod ipld_codec;
pub mod proof;
pub mod ring_buffer;
pub mod snark;
pub mod trie;

pub use error::ConsensusError;
pub use folding::{
    fold_proofs, fold_sequence, fold_snarks, verify_folded_proof, verify_preprocessing,
};
pub use ipld_codec::{decode_cbor, empty_root, encode_cbor, CryptoPointer};
pub use proof::{verify_proof, verify_proof_bool, MerkleProof, ProofStep, ProofTerminal};
pub use snark::{
    MicroSpartanPreprocessing, PublicInput, SnarkProof, StepInstance, ANCHOR_PROOF_SIZE,
    MAX_VERIFICATION_PAYLOAD_BYTES, MICROSPARTAN_GATE_COUNT, MICROSPARTAN_PREPROCESS_META_BYTES,
};
pub use trie::{bytes_to_nibbles, MerklePatriciaTrie, MptNode, StateMerge, TrieNode};

/// 32-byte cryptographic digest / content pointer.
pub type Hash32 = [u8; 32];

/// DOC 35: Strict 10 MB memory limit for the active ledger on edge devices.
pub const MAX_LEDGER_SIZE: usize = 10_000_000;

/// Initialize the consensus subsystem (daemon boot hook).
pub fn init_consensus() {
    println!(
        "Consensus: Merkle-Patricia Trie ledger ready (Blake3, MAX_LEDGER_SIZE={} bytes; SNARK fold constant {}B)",
        MAX_LEDGER_SIZE,
        ANCHOR_PROOF_SIZE
    );
}

// ─── SNARK folding hooks ─────────────────────────────────────────────────────

/// Hook for recursive SNARK folding. Prefer [`fold_proofs`] / [`fold_sequence`]
/// for direct use; this trait lets callers own an accumulator handle.
pub trait SnarkFolder {
    /// Fold an additional single-step (or already-folded) proof into the accumulator.
    fn fold(&mut self, proof: &SnarkProof) -> Result<(), ConsensusError>;

    /// Fold an explicit state-transition step into the accumulator.
    fn fold_step(&mut self, step: &StepInstance) -> Result<(), ConsensusError>;

    /// Verify the current accumulated proof against a public root/statement.
    fn verify(&self, expected_root: &Hash32) -> Result<bool, ConsensusError>;

    /// Borrow the current constant-size accumulator proof.
    fn accumulator(&self) -> &SnarkProof;
}

/// Production-path folder: Nova-style constant-size accumulation via [`fold_proofs`].
#[derive(Debug, Clone)]
pub struct AccumulatorSnarkFolder {
    acc: SnarkProof,
    genesis: Hash32,
}

impl AccumulatorSnarkFolder {
    /// Start from a genesis/checkpoint state root (fold_count = 0).
    pub fn new(genesis_root: Hash32) -> Self {
        Self {
            acc: SnarkProof::genesis(genesis_root),
            genesis: genesis_root,
        }
    }
}

impl Default for AccumulatorSnarkFolder {
    fn default() -> Self {
        Self::new([0u8; 32])
    }
}

impl SnarkFolder for AccumulatorSnarkFolder {
    fn fold(&mut self, proof: &SnarkProof) -> Result<(), ConsensusError> {
        if !proof.is_well_formed() {
            return Err(ConsensusError::SnarkError("ill-formed proof".into()));
        }
        // Single-step proofs fold via the DOC 34 compatibility path.
        if proof.fold_count == 1 {
            self.acc = fold_snarks(&self.acc, proof)?;
            return Ok(());
        }
        // Accept replacement only from a pure genesis accumulator.
        if self.acc.fold_count == 0 && proof.fold_count >= 1 {
            if proof.claimed_state_root == self.acc.claimed_state_root && proof.fold_count == 0 {
                return Ok(());
            }
            // Require the new proof to claim a chain that started at our genesis.
            let pi = PublicInput {
                genesis_state_root: self.genesis,
                final_state_root: proof.claimed_state_root,
                min_fold_count: 1,
            };
            verify_folded_proof(proof, &pi)?;
            self.acc = proof.clone();
            return Ok(());
        }
        Err(ConsensusError::SnarkError(
            "fold: expected single-step proof or empty accumulator; use fold_step".into(),
        ))
    }

    fn fold_step(&mut self, step: &StepInstance) -> Result<(), ConsensusError> {
        self.acc = fold_proofs(&self.acc, step)?;
        Ok(())
    }

    fn verify(&self, expected_root: &Hash32) -> Result<bool, ConsensusError> {
        let pi = PublicInput {
            genesis_state_root: self.genesis,
            final_state_root: *expected_root,
            min_fold_count: 0,
        };
        match verify_folded_proof(&self.acc, &pi) {
            Ok(()) => Ok(true),
            Err(ConsensusError::InvalidProof) => Ok(false),
            Err(e) => Err(e),
        }
    }

    fn accumulator(&self) -> &SnarkProof {
        &self.acc
    }
}

/// Lightweight counter folder for callers that only need the trait surface
/// (no commitment chain). Prefer [`AccumulatorSnarkFolder`] for real folds.
#[derive(Debug, Clone)]
pub struct StubSnarkFolder {
    pub folds: usize,
    acc: SnarkProof,
}

impl Default for StubSnarkFolder {
    fn default() -> Self {
        Self {
            folds: 0,
            acc: SnarkProof::genesis([0u8; 32]),
        }
    }
}

impl SnarkFolder for StubSnarkFolder {
    fn fold(&mut self, proof: &SnarkProof) -> Result<(), ConsensusError> {
        if !proof.is_well_formed() {
            return Err(ConsensusError::SnarkError("ill-formed proof".into()));
        }
        self.folds += 1;
        self.acc = proof.clone();
        Ok(())
    }

    fn fold_step(&mut self, step: &StepInstance) -> Result<(), ConsensusError> {
        self.fold(&SnarkProof::from_step(step))
    }

    fn verify(&self, expected_root: &Hash32) -> Result<bool, ConsensusError> {
        Ok(self.folds > 0 && self.acc.claimed_state_root == *expected_root)
    }

    fn accumulator(&self) -> &SnarkProof {
        &self.acc
    }
}

/// DOC 33: Verify a SNARK public blob is well-formed and constant-size.
pub fn verify_snark(proof: &SnarkProof) -> bool {
    snark::verify_snark(proof)
}

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
        // Empty public is never well-formed (must be 200-byte layout).
        let mut bad = SnarkProof::genesis([0u8; 32]);
        bad.public[64] ^= 0x01; // desync embedded fold_count
        assert!(folder.fold(&bad).is_err());

        let good = SnarkProof::genesis([9u8; 32]);
        folder.fold(&good).unwrap();
        assert!(folder.verify(&[9u8; 32]).unwrap());
    }

    #[test]
    fn accumulator_folder_folds_steps_constant_size() {
        let genesis = [0x11u8; 32];
        let mut folder = AccumulatorSnarkFolder::new(genesis);
        assert_eq!(folder.accumulator().public_bytes().len(), ANCHOR_PROOF_SIZE);
        assert_eq!(folder.accumulator().fold_count, 0);

        let step = StepInstance {
            prev_state_root: genesis,
            next_state_root: [0x22u8; 32],
            witness_digest: [0x33u8; 32],
        };
        folder.fold_step(&step).unwrap();
        assert_eq!(folder.accumulator().fold_count, 1);
        assert_eq!(folder.accumulator().public_bytes().len(), ANCHOR_PROOF_SIZE);
        assert!(folder.verify(&[0x22u8; 32]).unwrap());
        assert!(!folder.verify(&[0xFFu8; 32]).unwrap());
    }

    #[test]
    fn snark_and_trie_coexist() {
        let mut ledger = MerklePatriciaTrie::new();
        ledger.insert(b"k", b"v".to_vec()).unwrap();
        let root = ledger.root_hash();
        let proof = SnarkProof::genesis(root);
        assert!(verify_snark(&proof));
        let step = StepInstance {
            prev_state_root: root,
            next_state_root: {
                ledger.insert(b"k2", b"v2".to_vec()).unwrap();
                ledger.root_hash()
            },
            witness_digest: [7u8; 32],
        };
        let folded = fold_proofs(&proof, &step).unwrap();
        assert_eq!(folded.public_bytes().len(), ANCHOR_PROOF_SIZE);
        assert_eq!(folded.fold_count, 1);
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
