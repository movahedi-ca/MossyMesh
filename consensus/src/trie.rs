//! Incremental Merkle-Patricia Trie (hex-nibble radix-16).
//!
//! # Hashing
//! Node digests are **Blake3** over domain-tagged CBOR encodings (see [`crate::ipld_codec`]).
//! Blake3 was chosen over SHA-2 for speed on edge / IoT devices while remaining
//! deterministic and producing fixed 32-byte cryptographic pointers.
//!
//! # Structure
//! Keys are expanded to nibbles (high nibble first). Nodes are Leaf, Extension, or Branch.
//! Root hash updates on every successful insert; ledger byte accounting enforces
//! [`crate::MAX_LEDGER_SIZE`].

use std::collections::HashMap;

use crate::error::ConsensusError;
use crate::ipld_codec::{
    self, empty_root, hash_branch, hash_extension, hash_leaf, hash_tagged, TAG_LEGACY_NODE,
};
use crate::proof::{MerkleProof, ProofStep, ProofTerminal};
use crate::{Hash32, MAX_LEDGER_SIZE};

/// Approximate per-node structural overhead counted toward the ledger size budget.
const NODE_OVERHEAD: usize = 64;

/// Convert key bytes to a nibble path (two nibbles per byte, high nibble first).
pub fn bytes_to_nibbles(key: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(key.len() * 2);
    for &b in key {
        out.push(b >> 4);
        out.push(b & 0x0f);
    }
    out
}

// ═══════════════════════════════════════════════════════════════════════════
// Hex-nibble Merkle-Patricia nodes (primary ledger)
// ═══════════════════════════════════════════════════════════════════════════

/// Internal MPT node kinds (not the legacy integration `TrieNode`).
#[derive(Debug, Clone)]
pub enum MptNode {
    /// Terminal node: remaining nibble path + value.
    Leaf { path: Vec<u8>, value: Vec<u8> },
    /// Shared path compressed before a single child.
    Extension { path: Vec<u8>, child: Box<MptNode> },
    /// Hex-nibble branch: up to 16 children + optional value at this node.
    Branch {
        children: [Option<Box<MptNode>>; 16],
        value: Option<Vec<u8>>,
    },
}

impl MptNode {
    /// Recompute this node's Blake3 content hash from children (bottom-up).
    pub fn compute_hash(&self) -> Result<Hash32, ConsensusError> {
        match self {
            MptNode::Leaf { path, value } => hash_leaf(path, value),
            MptNode::Extension { path, child } => {
                let child_hash = child.compute_hash()?;
                hash_extension(path, &child_hash)
            }
            MptNode::Branch { children, value } => {
                let mut hashes: [Option<Hash32>; 16] = [None; 16];
                for (i, c) in children.iter().enumerate() {
                    if let Some(node) = c {
                        hashes[i] = Some(node.compute_hash()?);
                    }
                }
                hash_branch(&hashes, value.as_deref())
            }
        }
    }
}

/// Top-level ledger: Merkle-Patricia Trie with size tracking.
#[derive(Debug, Clone)]
pub struct MerklePatriciaTrie {
    root: Option<MptNode>,
    /// Cached root hash (invalidated/recomputed on mutation).
    cached_root: Hash32,
    /// Estimated active ledger footprint in bytes.
    size_bytes: usize,
}

impl Default for MerklePatriciaTrie {
    fn default() -> Self {
        Self::new()
    }
}

impl MerklePatriciaTrie {
    pub fn new() -> Self {
        Self {
            root: None,
            cached_root: empty_root(),
            size_bytes: 0,
        }
    }

    /// Current root hash (cryptographic pointer to full state).
    pub fn root_hash(&self) -> Hash32 {
        self.cached_root
    }

    /// Estimated ledger size in bytes (keys + values + node overhead).
    pub fn size_bytes(&self) -> usize {
        self.size_bytes
    }

    pub fn is_empty(&self) -> bool {
        self.root.is_none()
    }

    /// Insert or update `key` → `value`. Updates root hash.
    /// Returns [`ConsensusError::OutOfMemory`] if the 10 MB cap would be exceeded.
    pub fn insert(&mut self, key: &[u8], value: Vec<u8>) -> Result<(), ConsensusError> {
        let nibbles = bytes_to_nibbles(key);

        let old_value = self.get(key);
        let old_cost = match &old_value {
            Some(v) => key.len() + v.len() + NODE_OVERHEAD,
            None => 0,
        };
        let new_cost = key.len() + value.len() + NODE_OVERHEAD;
        let next_size = self
            .size_bytes
            .saturating_sub(old_cost)
            .saturating_add(new_cost);
        if next_size > MAX_LEDGER_SIZE {
            return Err(ConsensusError::OutOfMemory);
        }

        let new_root = match self.root.take() {
            None => MptNode::Leaf {
                path: nibbles,
                value,
            },
            Some(node) => insert_into(node, &nibbles, value)?,
        };

        self.cached_root = new_root.compute_hash()?;
        self.root = Some(new_root);
        self.size_bytes = next_size;
        Ok(())
    }

    /// Lookup value by key.
    pub fn get(&self, key: &[u8]) -> Option<Vec<u8>> {
        let nibbles = bytes_to_nibbles(key);
        match &self.root {
            None => None,
            Some(node) => get_from(node, &nibbles),
        }
    }

    /// Build a Merkle inclusion proof for `key`.
    pub fn prove(&self, key: &[u8]) -> Result<MerkleProof, ConsensusError> {
        let nibbles = bytes_to_nibbles(key);
        let root = self.root.as_ref().ok_or(ConsensusError::NotFound)?;

        // `build_proof` pushes ancestor steps after recursing → ordered terminal→root.
        let mut steps: Vec<ProofStep> = Vec::new();
        let (terminal, value) = build_proof(root, &nibbles, &mut steps)?;

        let leaf_path = match &terminal {
            ProofTerminal::Leaf { path, .. } => path.clone(),
            ProofTerminal::BranchValue { .. } => Vec::new(),
        };

        let mut siblings = Vec::new();
        for step in &steps {
            if let ProofStep::Branch {
                nibble, children, ..
            } = step
            {
                for (i, h) in children.iter().enumerate() {
                    if i != *nibble as usize {
                        if let Some(hash) = h {
                            siblings.push(*hash);
                        }
                    }
                }
            }
        }
        // Include sibling hashes from a branch-terminal as well.
        if let ProofTerminal::BranchValue { children, .. } = &terminal {
            for h in children.iter().flatten() {
                siblings.push(*h);
            }
        }

        Ok(MerkleProof {
            key: key.to_vec(),
            value,
            leaf_path,
            terminal,
            steps,
            siblings,
        })
    }

    /// Refresh cached root from the tree (normally not needed after insert).
    pub fn recompute_root(&mut self) -> Result<Hash32, ConsensusError> {
        self.cached_root = match &self.root {
            None => empty_root(),
            Some(n) => n.compute_hash()?,
        };
        Ok(self.cached_root)
    }
}

/// DOC 53 hook surface: CRDT-style merge entrypoint for island reconnection.
/// Full YATA/`yrs` lives in [`crate::crdt`]; this trait is the trie-level hook.
pub trait StateMerge {
    /// Merge remote state into self. Must be deterministic and commutative where possible.
    fn merge_with(&mut self, remote: &Self) -> Result<(), ConsensusError>;
}

impl StateMerge for MerklePatriciaTrie {
    /// Deterministic structural merge of two tries (placeholder bridge to full CRDT).
    ///
    /// Strategy: collect remote leaves and insert into local with lexical max on conflicts.
    /// Enforces `MAX_LEDGER_SIZE` on the post-merge estimated size. Leaves `self` unchanged on error.
    fn merge_with(&mut self, remote: &Self) -> Result<(), ConsensusError> {
        if self.cached_root == remote.cached_root {
            return Ok(());
        }

        let merged = match (self.root.as_ref(), remote.root.as_ref()) {
            (None, None) => None,
            (None, Some(r)) => Some(r.clone()),
            (Some(l), None) => Some(l.clone()),
            (Some(l), Some(r)) => Some(merge_nodes(l.clone(), r)?),
        };

        let new_size = estimate_node_size(merged.as_ref());
        if new_size > MAX_LEDGER_SIZE {
            return Err(ConsensusError::OutOfMemory);
        }

        let new_root_hash = match &merged {
            None => empty_root(),
            Some(n) => n.compute_hash()?,
        };

        self.root = merged;
        self.size_bytes = new_size;
        self.cached_root = new_root_hash;
        Ok(())
    }
}

fn estimate_node_size(node: Option<&MptNode>) -> usize {
    match node {
        None => 0,
        Some(MptNode::Leaf { path, value }) => path.len() + value.len() + NODE_OVERHEAD,
        Some(MptNode::Extension { path, child }) => {
            path.len() + NODE_OVERHEAD + estimate_node_size(Some(child))
        }
        Some(MptNode::Branch { children, value }) => {
            let mut s = NODE_OVERHEAD + value.as_ref().map(|v| v.len()).unwrap_or(0);
            for c in children.iter().flatten() {
                s = s.saturating_add(estimate_node_size(Some(c)));
            }
            s
        }
    }
}

fn merge_nodes(local: MptNode, remote: &MptNode) -> Result<MptNode, ConsensusError> {
    if local.compute_hash()? == remote.compute_hash()? {
        return Ok(local);
    }

    match (local, remote) {
        (
            MptNode::Leaf {
                path: lp,
                value: lv,
            },
            MptNode::Leaf {
                path: rp,
                value: rv,
            },
        ) => {
            if lp == *rp {
                let value = if rv > &lv { rv.clone() } else { lv };
                Ok(MptNode::Leaf { path: lp, value })
            } else {
                let mut node = MptNode::Leaf {
                    path: lp,
                    value: lv,
                };
                node = insert_into(node, rp, rv.clone())?;
                Ok(node)
            }
        }
        (mut local, remote) => {
            let leaves = collect_leaves(remote, &[]);
            for (full_path, val) in leaves {
                local = insert_into(local, &full_path, val)?;
            }
            Ok(local)
        }
    }
}

fn collect_leaves(node: &MptNode, prefix: &[u8]) -> Vec<(Vec<u8>, Vec<u8>)> {
    match node {
        MptNode::Leaf { path, value } => {
            let mut full = prefix.to_vec();
            full.extend_from_slice(path);
            vec![(full, value.clone())]
        }
        MptNode::Extension { path, child } => {
            let mut p = prefix.to_vec();
            p.extend_from_slice(path);
            collect_leaves(child, &p)
        }
        MptNode::Branch { children, value } => {
            let mut out = Vec::new();
            if let Some(v) = value {
                out.push((prefix.to_vec(), v.clone()));
            }
            for (i, c) in children.iter().enumerate() {
                if let Some(child) = c {
                    let mut p = prefix.to_vec();
                    p.push(i as u8);
                    out.extend(collect_leaves(child, &p));
                }
            }
            out
        }
    }
}

fn common_prefix_len(a: &[u8], b: &[u8]) -> usize {
    a.iter().zip(b.iter()).take_while(|(x, y)| x == y).count()
}

fn insert_into(node: MptNode, key: &[u8], value: Vec<u8>) -> Result<MptNode, ConsensusError> {
    match node {
        MptNode::Leaf {
            path: leaf_path,
            value: leaf_value,
        } => insert_leaf(leaf_path, leaf_value, key, value),
        MptNode::Extension { path, child } => insert_extension(path, *child, key, value),
        MptNode::Branch {
            children,
            value: branch_val,
        } => insert_branch(children, branch_val, key, value),
    }
}

fn insert_leaf(
    leaf_path: Vec<u8>,
    leaf_value: Vec<u8>,
    key: &[u8],
    value: Vec<u8>,
) -> Result<MptNode, ConsensusError> {
    if leaf_path == key {
        return Ok(MptNode::Leaf {
            path: leaf_path,
            value,
        });
    }

    let shared = common_prefix_len(&leaf_path, key);

    let mut children: [Option<Box<MptNode>>; 16] = Default::default();
    let mut branch_value: Option<Vec<u8>> = None;

    if shared == leaf_path.len() {
        branch_value = Some(leaf_value);
    } else {
        let nibble = leaf_path[shared] as usize;
        if nibble > 15 {
            return Err(ConsensusError::InvalidInput("nibble out of range"));
        }
        let rem = leaf_path[shared + 1..].to_vec();
        children[nibble] = Some(Box::new(MptNode::Leaf {
            path: rem,
            value: leaf_value,
        }));
    }

    if shared == key.len() {
        branch_value = Some(value);
    } else {
        let nibble = key[shared] as usize;
        if nibble > 15 {
            return Err(ConsensusError::InvalidInput("nibble out of range"));
        }
        let rem = key[shared + 1..].to_vec();
        children[nibble] = Some(Box::new(MptNode::Leaf { path: rem, value }));
    }

    let branch = MptNode::Branch {
        children,
        value: branch_value,
    };

    if shared == 0 {
        Ok(branch)
    } else {
        Ok(MptNode::Extension {
            path: leaf_path[..shared].to_vec(),
            child: Box::new(branch),
        })
    }
}

fn insert_extension(
    path: Vec<u8>,
    child: MptNode,
    key: &[u8],
    value: Vec<u8>,
) -> Result<MptNode, ConsensusError> {
    let shared = common_prefix_len(&path, key);

    if shared == path.len() {
        let new_child = insert_into(child, &key[shared..], value)?;
        return Ok(MptNode::Extension {
            path,
            child: Box::new(new_child),
        });
    }

    let mut children: [Option<Box<MptNode>>; 16] = Default::default();
    let mut branch_value: Option<Vec<u8>> = None;

    let ext_nibble = path[shared] as usize;
    let ext_rem = path[shared + 1..].to_vec();
    let existing_child = if ext_rem.is_empty() {
        child
    } else {
        MptNode::Extension {
            path: ext_rem,
            child: Box::new(child),
        }
    };
    children[ext_nibble] = Some(Box::new(existing_child));

    if shared == key.len() {
        branch_value = Some(value);
    } else {
        let k_nibble = key[shared] as usize;
        let k_rem = key[shared + 1..].to_vec();
        children[k_nibble] = Some(Box::new(MptNode::Leaf {
            path: k_rem,
            value,
        }));
    }

    let branch = MptNode::Branch {
        children,
        value: branch_value,
    };

    if shared == 0 {
        Ok(branch)
    } else {
        Ok(MptNode::Extension {
            path: path[..shared].to_vec(),
            child: Box::new(branch),
        })
    }
}

fn insert_branch(
    mut children: [Option<Box<MptNode>>; 16],
    mut branch_value: Option<Vec<u8>>,
    key: &[u8],
    value: Vec<u8>,
) -> Result<MptNode, ConsensusError> {
    if key.is_empty() {
        branch_value = Some(value);
        return Ok(MptNode::Branch {
            children,
            value: branch_value,
        });
    }

    let nibble = key[0] as usize;
    if nibble > 15 {
        return Err(ConsensusError::InvalidInput("nibble out of range"));
    }
    let rem = &key[1..];

    let new_child = match children[nibble].take() {
        None => MptNode::Leaf {
            path: rem.to_vec(),
            value,
        },
        Some(child) => insert_into(*child, rem, value)?,
    };
    children[nibble] = Some(Box::new(new_child));

    Ok(MptNode::Branch {
        children,
        value: branch_value,
    })
}

fn get_from(node: &MptNode, key: &[u8]) -> Option<Vec<u8>> {
    match node {
        MptNode::Leaf { path, value } => {
            if path.as_slice() == key {
                Some(value.clone())
            } else {
                None
            }
        }
        MptNode::Extension { path, child } => {
            if key.starts_with(path) {
                get_from(child, &key[path.len()..])
            } else {
                None
            }
        }
        MptNode::Branch { children, value } => {
            if key.is_empty() {
                return value.clone();
            }
            let nibble = key[0] as usize;
            if nibble > 15 {
                return None;
            }
            children[nibble]
                .as_ref()
                .and_then(|c| get_from(c, &key[1..]))
        }
    }
}

/// Walk the trie collecting terminal + ancestor steps (terminal→root order).
fn build_proof(
    node: &MptNode,
    key: &[u8],
    steps: &mut Vec<ProofStep>,
) -> Result<(ProofTerminal, Vec<u8>), ConsensusError> {
    match node {
        MptNode::Leaf { path, value } => {
            if path.as_slice() != key {
                return Err(ConsensusError::NotFound);
            }
            Ok((
                ProofTerminal::Leaf {
                    path: path.clone(),
                    value: value.clone(),
                },
                value.clone(),
            ))
        }
        MptNode::Extension { path, child } => {
            if !key.starts_with(path) {
                return Err(ConsensusError::NotFound);
            }
            let (terminal, value) = build_proof(child, &key[path.len()..], steps)?;
            steps.push(ProofStep::Extension { path: path.clone() });
            Ok((terminal, value))
        }
        MptNode::Branch { children, value } => {
            if key.is_empty() {
                // Value lives on this branch — terminal includes all child hashes.
                let v = value.clone().ok_or(ConsensusError::NotFound)?;
                let mut child_hashes: [Option<Hash32>; 16] = [None; 16];
                for (i, c) in children.iter().enumerate() {
                    if let Some(node) = c {
                        child_hashes[i] = Some(node.compute_hash()?);
                    }
                }
                return Ok((
                    ProofTerminal::BranchValue {
                        children: child_hashes,
                        value: v.clone(),
                    },
                    v,
                ));
            }
            let nibble = key[0];
            if nibble > 15 {
                return Err(ConsensusError::InvalidInput("nibble out of range"));
            }
            let child = children[nibble as usize]
                .as_ref()
                .ok_or(ConsensusError::NotFound)?;

            let (terminal, val) = build_proof(child, &key[1..], steps)?;

            let mut child_hashes: [Option<Hash32>; 16] = [None; 16];
            for (i, c) in children.iter().enumerate() {
                if i == nibble as usize {
                    continue;
                }
                if let Some(node) = c {
                    child_hashes[i] = Some(node.compute_hash()?);
                }
            }

            steps.push(ProofStep::Branch {
                nibble,
                children: child_hashes,
                value: value.clone(),
            });
            Ok((terminal, val))
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Legacy byte-keyed radix node (integration smoke tests + DOC 27 surface)
// ═══════════════════════════════════════════════════════════════════════════

/// DOC 27: Legacy trie node used by integration harness and simple mesh demos.
///
/// Children are mapped by path **byte** (not nibble). Hashing is real Blake3 over
/// a deterministic CBOR encoding of sorted child hashes + optional value.
#[derive(Debug, Clone)]
pub struct TrieNode {
    pub hash: Hash32,
    /// DOC 28: Children mapped via path byte, allowing O(log N) state lookups.
    pub children: HashMap<u8, Box<TrieNode>>,
    pub value: Option<Vec<u8>>,
}

impl Default for TrieNode {
    fn default() -> Self {
        Self::new()
    }
}

impl TrieNode {
    pub fn new() -> Self {
        TrieNode {
            hash: empty_root(),
            children: HashMap::new(),
            value: None,
        }
    }

    /// Insert a key-value pair (byte-path). Recomputes hashes bottom-up with Blake3.
    /// DOC 29: Recursive insertion updates the root hash incrementally.
    pub fn insert_node(&mut self, key: &[u8], value: Vec<u8>) {
        if key.is_empty() {
            self.value = Some(value);
            self.rehash();
            return;
        }

        // DOC 30: The first byte dictates the branch path through the recursive tree.
        let first = key[0];
        let child = self
            .children
            .entry(first)
            .or_insert_with(|| Box::new(TrieNode::new()));
        child.insert_node(&key[1..], value);
        self.rehash();
    }

    /// Lookup along a byte path.
    pub fn get_node(&self, key: &[u8]) -> Option<&[u8]> {
        if key.is_empty() {
            return self.value.as_deref();
        }
        self.children
            .get(&key[0])
            .and_then(|c| c.get_node(&key[1..]))
    }

    /// DOC 53: Deterministic merge when mesh islands reconnect.
    pub fn merge_state(&mut self, remote_node: &TrieNode) {
        if self.hash == remote_node.hash {
            return;
        }

        if self.children.is_empty() && remote_node.children.is_empty() {
            if let (Some(local_val), Some(remote_val)) = (&self.value, &remote_node.value) {
                if remote_val > local_val {
                    self.value = Some(remote_val.clone());
                }
            } else if remote_node.value.is_some() {
                self.value = remote_node.value.clone();
            }
        } else if let Some(remote_val) = &remote_node.value {
            match &self.value {
                None => self.value = Some(remote_val.clone()),
                Some(local_val) if remote_val > local_val => {
                    self.value = Some(remote_val.clone());
                }
                _ => {}
            }
        }

        for (key, remote_child) in &remote_node.children {
            let local_child = self
                .children
                .entry(*key)
                .or_insert_with(|| Box::new(TrieNode::new()));
            local_child.merge_state(remote_child);
        }

        self.rehash();
    }

    /// DOC 31: Parent hash is Blake3 of children's hashes + value (not keccak stub).
    fn rehash(&mut self) {
        self.hash = hash_legacy_node(&self.children, self.value.as_deref());
    }
}

/// Deterministic Blake3 hash for legacy byte-keyed nodes.
fn hash_legacy_node(children: &HashMap<u8, Box<TrieNode>>, value: Option<&[u8]>) -> Hash32 {
    // Sorted (nibble/byte, child_hash) pairs for determinism.
    let mut pairs: Vec<(u8, Hash32)> = children.iter().map(|(&k, v)| (k, v.hash)).collect();
    pairs.sort_by_key(|(k, _)| *k);

    #[derive(serde::Serialize)]
    struct LegacyPayload<'a> {
        children: &'a [(u8, Hash32)],
        value: Option<&'a [u8]>,
    }

    let payload = LegacyPayload {
        children: &pairs,
        value,
    };
    match ipld_codec::encode_cbor(&payload) {
        Ok(cbor) => hash_tagged(TAG_LEGACY_NODE, &cbor),
        Err(_) => {
            // Fallback: hash raw concatenated material (should be unreachable).
            let mut h = blake3::Hasher::new();
            h.update(&[TAG_LEGACY_NODE]);
            for (k, hash) in &pairs {
                h.update(&[*k]);
                h.update(hash);
            }
            if let Some(v) = value {
                h.update(v);
            }
            *h.finalize().as_bytes()
        }
    }
}

// ─── Unit tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proof::verify_proof;

    #[test]
    fn insert_get_roundtrip() {
        let mut t = MerklePatriciaTrie::new();
        t.insert(b"foo", b"bar".to_vec()).unwrap();
        t.insert(b"food", b"baz".to_vec()).unwrap();
        t.insert(b"a", b"1".to_vec()).unwrap();

        assert_eq!(t.get(b"foo").as_deref(), Some(b"bar".as_ref()));
        assert_eq!(t.get(b"food").as_deref(), Some(b"baz".as_ref()));
        assert_eq!(t.get(b"a").as_deref(), Some(b"1".as_ref()));
        assert_eq!(t.get(b"missing"), None);
    }

    #[test]
    fn root_hash_changes_on_insert() {
        let mut t = MerklePatriciaTrie::new();
        let empty = t.root_hash();
        t.insert(b"k1", b"v1".to_vec()).unwrap();
        let h1 = t.root_hash();
        assert_ne!(empty, h1);
        t.insert(b"k2", b"v2".to_vec()).unwrap();
        let h2 = t.root_hash();
        assert_ne!(h1, h2);
        t.insert(b"k1", b"v1-updated".to_vec()).unwrap();
        assert_ne!(h2, t.root_hash());
    }

    #[test]
    fn root_hash_deterministic() {
        let mut a = MerklePatriciaTrie::new();
        let mut b = MerklePatriciaTrie::new();
        for (k, v) in [
            (b"x" as &[u8], b"1" as &[u8]),
            (b"y", b"2"),
            (b"z", b"3"),
        ] {
            a.insert(k, v.to_vec()).unwrap();
            b.insert(k, v.to_vec()).unwrap();
        }
        assert_eq!(a.root_hash(), b.root_hash());
    }

    #[test]
    fn proof_verify_multiple_keys() {
        let mut t = MerklePatriciaTrie::new();
        let pairs = [
            (b"alice".as_ref(), b"100".as_ref()),
            (b"bob", b"200"),
            (b"carol", b"300"),
            (b"alice/escrow", b"50"),
        ];
        for (k, v) in pairs {
            t.insert(k, v.to_vec()).unwrap();
        }
        let root = t.root_hash();
        for (k, v) in pairs {
            let proof = t.prove(k).unwrap();
            assert_eq!(proof.value, v);
            assert!(verify_proof(&proof, &root).unwrap());
        }
    }

    #[test]
    fn out_of_memory_on_cap() {
        let mut t = MerklePatriciaTrie::new();
        let big = vec![0u8; MAX_LEDGER_SIZE - NODE_OVERHEAD - 10];
        t.insert(b"big", big).unwrap();
        let err = t.insert(b"more", vec![0u8; 100]).unwrap_err();
        assert_eq!(err, ConsensusError::OutOfMemory);
    }

    #[test]
    fn size_bytes_tracks_updates() {
        let mut t = MerklePatriciaTrie::new();
        assert_eq!(t.size_bytes(), 0);
        t.insert(b"k", b"abc".to_vec()).unwrap();
        let s1 = t.size_bytes();
        assert!(s1 > 0);
        t.insert(b"k", b"ab".to_vec()).unwrap();
        assert!(t.size_bytes() < s1);
    }

    #[test]
    fn merge_hook_converges() {
        let mut a = MerklePatriciaTrie::new();
        let mut b = MerklePatriciaTrie::new();
        a.insert(b"shared", b"1".to_vec()).unwrap();
        b.insert(b"shared", b"1".to_vec()).unwrap();
        a.insert(b"only-a", b"A".to_vec()).unwrap();
        b.insert(b"only-b", b"B".to_vec()).unwrap();

        a.merge_with(&b).unwrap();
        assert_eq!(a.get(b"only-a").as_deref(), Some(b"A".as_ref()));
        assert_eq!(a.get(b"only-b").as_deref(), Some(b"B".as_ref()));
        assert_eq!(a.get(b"shared").as_deref(), Some(b"1".as_ref()));
    }

    #[test]
    fn merge_conflict_lexical_tiebreak() {
        let mut a = MerklePatriciaTrie::new();
        let mut b = MerklePatriciaTrie::new();
        a.insert(b"k", b"aaa".to_vec()).unwrap();
        b.insert(b"k", b"zzz".to_vec()).unwrap();
        a.merge_with(&b).unwrap();
        assert_eq!(a.get(b"k").as_deref(), Some(b"zzz".as_ref()));
    }

    #[test]
    fn nibble_encoding() {
        assert_eq!(
            bytes_to_nibbles(&[0xab, 0x0f]),
            vec![0x0a, 0x0b, 0x00, 0x0f]
        );
    }

    #[test]
    fn prove_missing_key() {
        let mut t = MerklePatriciaTrie::new();
        t.insert(b"exists", b"1".to_vec()).unwrap();
        assert!(matches!(t.prove(b"nope"), Err(ConsensusError::NotFound)));
    }

    #[test]
    fn legacy_trie_insert_merge_and_hash() {
        let mut a = TrieNode::new();
        let mut b = TrieNode::new();
        a.insert_node(&[0x01, 0x02], b"alpha".to_vec());
        b.insert_node(&[0x01, 0x03], b"beta".to_vec());
        let h_before = a.hash;
        a.merge_state(&b);
        assert!(a.children.contains_key(&0x01));
        assert_ne!(a.hash, h_before);
        assert_ne!(a.hash, [0u8; 32]);
        assert_eq!(a.get_node(&[0x01, 0x02]), Some(b"alpha".as_ref()));
        assert_eq!(a.get_node(&[0x01, 0x03]), Some(b"beta".as_ref()));
    }

    #[test]
    fn legacy_hash_deterministic() {
        let mut a = TrieNode::new();
        let mut b = TrieNode::new();
        a.insert_node(b"key", b"val".to_vec());
        b.insert_node(b"key", b"val".to_vec());
        assert_eq!(a.hash, b.hash);
    }
}
