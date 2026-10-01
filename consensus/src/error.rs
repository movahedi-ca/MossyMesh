//! Consensus error types for the MossyMesh ledger.

use std::fmt;

/// Errors raised by the Merkle-Patricia Trie ledger and related hooks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConsensusError {
    /// Active ledger would exceed [`crate::MAX_LEDGER_SIZE`] (10 MB edge cap).
    OutOfMemory,
    /// A Merkle (or SNARK) proof failed verification.
    InvalidProof,
    /// Requested key is not present in the trie.
    NotFound,
    /// Malformed key, nibble path, or proof structure.
    InvalidInput(&'static str),
    /// Serialization / codec failure (DAG-CBOR path).
    CodecError(String),
    /// Reserved for CRDT merge conflicts that cannot be resolved locally.
    MergeConflict,
    /// Reserved for future Nova-SNARK / folding failures.
    SnarkError(String),
}

impl fmt::Display for ConsensusError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConsensusError::OutOfMemory => {
                write!(f, "ledger would exceed MAX_LEDGER_SIZE (10_000_000 bytes)")
            }
            ConsensusError::InvalidProof => write!(f, "invalid merkle or snark proof"),
            ConsensusError::NotFound => write!(f, "key not found in trie"),
            ConsensusError::InvalidInput(msg) => write!(f, "invalid input: {msg}"),
            ConsensusError::CodecError(msg) => write!(f, "codec error: {msg}"),
            ConsensusError::MergeConflict => write!(f, "unresolved CRDT merge conflict"),
            ConsensusError::SnarkError(msg) => write!(f, "snark error: {msg}"),
        }
    }
}

impl std::error::Error for ConsensusError {}
