//! Execution trace hash chains to prove nodes forwarded traffic legitimately.
//!
//! Free-Rider Prevention: each node must submit Cryptographic Hash Chains of
//! their WASM execution trace to prove actual computation occurred, rather than
//! simple data forwarding.

/// Domain-separated genesis salt for WASM execution chains.
pub const CHAIN_GENESIS_DOMAIN: &[u8] = b"mossymesh-wasm-exec-v1";

/// FNV-1a 64-bit offset basis.
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
/// FNV-1a 64-bit prime.
const FNV_PRIME: u64 = 0x1000_0000_01b3;

/// One link in a WASM execution proof chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainLink {
    /// Monotonic step index in the WASM trace.
    pub step: u64,
    /// Opaque execution fingerprint (opcode mix / memory digest) for this step.
    pub exec_digest: [u8; 32],
    /// Hash binding previous_hash || step || exec_digest.
    pub link_hash: [u8; 32],
}

/// Append-only WASM execution hash chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionHashChain {
    pub peer_id: String,
    pub job_id: u64,
    /// Head hash after the last append (genesis if empty).
    pub head: [u8; 32],
    pub links: Vec<ChainLink>,
}

impl ExecutionHashChain {
    /// Create a new chain anchored at a domain-separated genesis hash.
    pub fn new(peer_id: impl Into<String>, job_id: u64) -> Self {
        let peer_id = peer_id.into();
        let head = genesis_hash(&peer_id, job_id);
        Self {
            peer_id,
            job_id,
            head,
            links: Vec::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.links.len()
    }

    pub fn is_empty(&self) -> bool {
        self.links.is_empty()
    }

    /// Append a WASM execution step digest to the chain.
    pub fn append(&mut self, step: u64, exec_digest: [u8; 32]) -> [u8; 32] {
        let link_hash = hash_link(&self.head, step, &exec_digest);
        self.links.push(ChainLink {
            step,
            exec_digest,
            link_hash,
        });
        self.head = link_hash;
        link_hash
    }

    /// Verify the entire chain from genesis through head.
    /// Returns `Ok(())` or the index of the first broken link.
    pub fn verify(&self) -> Result<(), usize> {
        let mut prev = genesis_hash(&self.peer_id, self.job_id);
        for (i, link) in self.links.iter().enumerate() {
            let expected = hash_link(&prev, link.step, &link.exec_digest);
            if expected != link.link_hash {
                return Err(i);
            }
            prev = link.link_hash;
        }
        if prev != self.head {
            return Err(self.links.len().saturating_sub(1));
        }
        Ok(())
    }

    /// Detect tampering: any broken binding makes this return true.
    pub fn detect_tamper(&self) -> bool {
        self.verify().is_err()
    }
}

/// Genesis hash = H(domain || peer_id || job_id_le).
pub fn genesis_hash(peer_id: &str, job_id: u64) -> [u8; 32] {
    let mut data = Vec::with_capacity(CHAIN_GENESIS_DOMAIN.len() + peer_id.len() + 8);
    data.extend_from_slice(CHAIN_GENESIS_DOMAIN);
    data.extend_from_slice(peer_id.as_bytes());
    data.extend_from_slice(&job_id.to_le_bytes());
    widen_u64_hash(fnv1a_64(&data))
}

/// H(prev || step_le || exec_digest).
pub fn hash_link(prev: &[u8; 32], step: u64, exec_digest: &[u8; 32]) -> [u8; 32] {
    let mut data = Vec::with_capacity(32 + 8 + 32);
    data.extend_from_slice(prev);
    data.extend_from_slice(&step.to_le_bytes());
    data.extend_from_slice(exec_digest);
    let h1 = fnv1a_64(&data);
    let mut round2 = data;
    round2.extend_from_slice(&h1.to_le_bytes());
    let h2 = fnv1a_64(&round2);
    combine_hashes(h1, h2)
}

/// Build a synthetic WASM exec digest from step-local bytes (test / stub helper).
pub fn wasm_exec_digest(opcode_trace: &[u8]) -> [u8; 32] {
    widen_u64_hash(fnv1a_64(opcode_trace))
}

fn fnv1a_64(data: &[u8]) -> u64 {
    let mut hash = FNV_OFFSET;
    for &b in data {
        hash ^= b as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

fn widen_u64_hash(h: u64) -> [u8; 32] {
    let mut out = [0u8; 32];
    let mut x = h;
    for chunk in out.chunks_mut(8) {
        x = x
            .wrapping_mul(0x9E37_79B9_7F4A_7C15)
            .wrapping_add(0x85EB_CA6B);
        chunk.copy_from_slice(&x.to_le_bytes());
    }
    out
}

fn combine_hashes(a: u64, b: u64) -> [u8; 32] {
    let mut out = [0u8; 32];
    let mut x = a ^ b.rotate_left(17);
    for chunk in out.chunks_mut(8) {
        x = x.wrapping_mul(0xC2B2_AE3D_27D4_EB4F).wrapping_add(b);
        chunk.copy_from_slice(&x.to_le_bytes());
    }
    out
}

/// Verify a peer-submitted chain against an independently recomputed expected head.
pub fn verify_submitted_chain(chain: &ExecutionHashChain, expected_head: &[u8; 32]) -> bool {
    chain.verify().is_ok() && chain.head == *expected_head
}

pub fn init_hash_chain() {
    println!("Initializing Hash Chains to prove actual computation/routing.");
    let mut chain = ExecutionHashChain::new("boot-peer", 0);
    let d = wasm_exec_digest(b"init");
    chain.append(0, d);
    println!(
        "Hash chain demo: {} links, verify={:?}",
        chain.len(),
        chain.verify()
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_append_and_verify() {
        let mut chain = ExecutionHashChain::new("worker-1", 42);
        for step in 0..5u64 {
            let dig = wasm_exec_digest(&[step as u8, 0xAA, 0xBB]);
            chain.append(step, dig);
        }
        assert_eq!(chain.len(), 5);
        assert!(chain.verify().is_ok());
        assert!(!chain.detect_tamper());
    }

    #[test]
    fn test_tamper_detect_mutated_digest() {
        let mut chain = ExecutionHashChain::new("worker-2", 7);
        chain.append(0, wasm_exec_digest(b"op-a"));
        chain.append(1, wasm_exec_digest(b"op-b"));
        chain.append(2, wasm_exec_digest(b"op-c"));
        assert!(chain.verify().is_ok());

        chain.links[1].exec_digest = wasm_exec_digest(b"TAMPERED");
        assert!(chain.detect_tamper());
        assert_eq!(chain.verify(), Err(1));
    }

    #[test]
    fn test_tamper_detect_mutated_link_hash() {
        let mut chain = ExecutionHashChain::new("worker-3", 99);
        chain.append(0, wasm_exec_digest(b"x"));
        chain.append(1, wasm_exec_digest(b"y"));
        chain.links[0].link_hash = [0xFF; 32];
        chain.head = chain.links.last().unwrap().link_hash;
        assert!(chain.detect_tamper());
    }

    #[test]
    fn test_tamper_detect_head_mismatch() {
        let mut chain = ExecutionHashChain::new("worker-4", 1);
        chain.append(0, wasm_exec_digest(b"step0"));
        chain.head = [0u8; 32];
        assert!(chain.detect_tamper());
    }

    #[test]
    fn test_verify_submitted_chain_expected_head() {
        let mut honest = ExecutionHashChain::new("w", 3);
        honest.append(0, wasm_exec_digest(b"a"));
        honest.append(1, wasm_exec_digest(b"b"));
        let expected_head = honest.head;

        assert!(verify_submitted_chain(&honest, &expected_head));
        let wrong_head = [1u8; 32];
        assert!(!verify_submitted_chain(&honest, &wrong_head));
    }

    #[test]
    fn test_genesis_differs_by_peer_and_job() {
        let a = genesis_hash("p1", 1);
        let b = genesis_hash("p2", 1);
        let c = genesis_hash("p1", 2);
        assert_ne!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn test_empty_chain_verifies() {
        let chain = ExecutionHashChain::new("empty", 0);
        assert!(chain.is_empty());
        assert!(chain.verify().is_ok());
    }
}
