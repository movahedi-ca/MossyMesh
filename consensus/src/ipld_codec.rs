//! Compact CBOR encoding for IPLD-style cryptographic pointers and trie nodes.
//!
//! Uses `serde_cbor` as a lightweight DAG-CBOR-ish encoding.
//! Full IPLD CID resolution is deferred; hashes are raw 32-byte cryptographic pointers.

use serde::{Deserialize, Serialize};

use crate::error::ConsensusError;
use crate::Hash32;

/// Domain tags for deterministic node hashing (must match trie hashing).
pub const TAG_LEAF: u8 = 0x00;
pub const TAG_EXTENSION: u8 = 0x01;
pub const TAG_BRANCH: u8 = 0x02;
pub const TAG_EMPTY: u8 = 0xff;
pub const TAG_LEGACY_NODE: u8 = 0x10;

/// IPLD-oriented leaf payload (path nibbles + value bytes).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LeafCodec {
    pub path: Vec<u8>,
    pub value: Vec<u8>,
}

/// IPLD-oriented extension payload (shared path + child pointer).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExtensionCodec {
    pub path: Vec<u8>,
    pub child: Hash32,
}

/// IPLD-oriented branch payload (16 optional child pointers + optional value).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BranchCodec {
    /// Exactly 16 slots; `None` means empty child.
    pub children: Vec<Option<Hash32>>,
    pub value: Option<Vec<u8>>,
}

/// Encode an arbitrary serde value as compact CBOR bytes.
pub fn encode_cbor<T: Serialize>(value: &T) -> Result<Vec<u8>, ConsensusError> {
    serde_cbor::to_vec(value).map_err(|e| ConsensusError::CodecError(e.to_string()))
}

/// Decode CBOR bytes into a typed value.
pub fn decode_cbor<T: for<'de> Deserialize<'de>>(bytes: &[u8]) -> Result<T, ConsensusError> {
    serde_cbor::from_slice(bytes).map_err(|e| ConsensusError::CodecError(e.to_string()))
}

/// Deterministic Blake3 hash of domain-tagged CBOR payload.
///
/// Layout: `tag || cbor_bytes` → Blake3 → 32-byte digest.
pub fn hash_tagged(tag: u8, cbor_payload: &[u8]) -> Hash32 {
    let mut hasher = blake3::Hasher::new();
    hasher.update(&[tag]);
    hasher.update(cbor_payload);
    *hasher.finalize().as_bytes()
}

/// Hash of the empty trie (fixed sentinel).
pub fn empty_root() -> Hash32 {
    hash_tagged(TAG_EMPTY, b"mossymesh-empty-trie")
}

/// Encode + hash a leaf node.
pub fn hash_leaf(path: &[u8], value: &[u8]) -> Result<Hash32, ConsensusError> {
    let payload = LeafCodec {
        path: path.to_vec(),
        value: value.to_vec(),
    };
    let cbor = encode_cbor(&payload)?;
    Ok(hash_tagged(TAG_LEAF, &cbor))
}

/// Encode + hash an extension node.
pub fn hash_extension(path: &[u8], child: &Hash32) -> Result<Hash32, ConsensusError> {
    let payload = ExtensionCodec {
        path: path.to_vec(),
        child: *child,
    };
    let cbor = encode_cbor(&payload)?;
    Ok(hash_tagged(TAG_EXTENSION, &cbor))
}

/// Encode + hash a branch node.
pub fn hash_branch(
    children: &[Option<Hash32>; 16],
    value: Option<&[u8]>,
) -> Result<Hash32, ConsensusError> {
    let payload = BranchCodec {
        children: children.iter().copied().collect(),
        value: value.map(|v| v.to_vec()),
    };
    let cbor = encode_cbor(&payload)?;
    Ok(hash_tagged(TAG_BRANCH, &cbor))
}

/// Cryptographic pointer: 32-byte content hash used as an IPLD-style link.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CryptoPointer(pub Hash32);

impl CryptoPointer {
    pub fn from_hash(hash: Hash32) -> Self {
        Self(hash)
    }

    pub fn as_bytes(&self) -> &Hash32 {
        &self.0
    }

    /// Compact CBOR encoding of the pointer (raw 32-byte array).
    pub fn to_cbor(&self) -> Result<Vec<u8>, ConsensusError> {
        encode_cbor(&self.0)
    }

    pub fn from_cbor(bytes: &[u8]) -> Result<Self, ConsensusError> {
        let h: Hash32 = decode_cbor(bytes)?;
        Ok(Self(h))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cbor_roundtrip_leaf() {
        let leaf = LeafCodec {
            path: vec![0x0a, 0x0b],
            value: b"hello".to_vec(),
        };
        let enc = encode_cbor(&leaf).unwrap();
        let dec: LeafCodec = decode_cbor(&enc).unwrap();
        assert_eq!(leaf, dec);
    }

    #[test]
    fn tagged_hash_is_deterministic() {
        let a = hash_leaf(&[1, 2], b"v").unwrap();
        let b = hash_leaf(&[1, 2], b"v").unwrap();
        let c = hash_leaf(&[1, 2], b"w").unwrap();
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn empty_root_stable() {
        assert_eq!(empty_root(), empty_root());
        assert_ne!(empty_root(), [0u8; 32]);
    }
}
