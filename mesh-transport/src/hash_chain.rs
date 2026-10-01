//! Execution trace hash chains to prove nodes forwarded traffic legitimately.
//!
//! Free-Rider Prevention: each node must submit Cryptographic Hash Chains of
//! their WASM execution trace to prove actual computation occurred, rather than
//! simple data forwarding.
//!
//! All digests are SHA-256 (fixes #200: the previous FNV-1a construction was
//! forgeable). Chain steps must be strictly increasing, enforced by both
//! [`ExecutionHashChain::append`] and [`ExecutionHashChain::verify`].

use sha2::{Digest, Sha256};

/// Domain-separated genesis salt for WASM execution chains.
pub const CHAIN_GENESIS_DOMAIN: &[u8] = b"mossymesh-wasm-exec-v1";

/// SHA-256 of `data`, returned as a fixed 32-byte array.
fn sha256_32(data: &[u8]) -> [u8; 32] {
    let digest = Sha256::digest(data);
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    out
}

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
    ///
    /// Steps must be strictly increasing (fixes #200: a 1-link chain could
    /// otherwise "prove" a full execution). Returns `None` and leaves the
    /// chain unchanged when `step` is not greater than the previous step.
    pub fn append(&mut self, step: u64, exec_digest: [u8; 32]) -> Option<[u8; 32]> {
        if let Some(prev) = self.links.last() {
            if step <= prev.step {
                return None;
            }
        }
        let link_hash = hash_link(&self.head, step, &exec_digest);
        self.links.push(ChainLink {
            step,
            exec_digest,
            link_hash,
        });
        self.head = link_hash;
        Some(link_hash)
    }

    /// Verify the entire chain from genesis through head.
    /// Returns `Ok(())` or the index of the first broken link (a link that
    /// breaks the strictly-increasing step order counts as broken).
    pub fn verify(&self) -> Result<(), usize> {
        let mut prev = genesis_hash(&self.peer_id, self.job_id);
        let mut prev_step: Option<u64> = None;
        for (i, link) in self.links.iter().enumerate() {
            if let Some(p) = prev_step {
                if link.step <= p {
                    return Err(i);
                }
            }
            let expected = hash_link(&prev, link.step, &link.exec_digest);
            if expected != link.link_hash {
                return Err(i);
            }
            prev = link.link_hash;
            prev_step = Some(link.step);
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

/// Genesis hash = SHA-256(domain || peer_id || job_id_le).
pub fn genesis_hash(peer_id: &str, job_id: u64) -> [u8; 32] {
    let mut data = Vec::with_capacity(CHAIN_GENESIS_DOMAIN.len() + peer_id.len() + 8);
    data.extend_from_slice(CHAIN_GENESIS_DOMAIN);
    data.extend_from_slice(peer_id.as_bytes());
    data.extend_from_slice(&job_id.to_le_bytes());
    sha256_32(&data)
}

/// H(prev || step_le || exec_digest), via SHA-256.
pub fn hash_link(prev: &[u8; 32], step: u64, exec_digest: &[u8; 32]) -> [u8; 32] {
    let mut data = Vec::with_capacity(32 + 8 + 32);
    data.extend_from_slice(prev);
    data.extend_from_slice(&step.to_le_bytes());
    data.extend_from_slice(exec_digest);
    sha256_32(&data)
}

/// Build a synthetic WASM exec digest from step-local bytes (test / stub helper).
pub fn wasm_exec_digest(opcode_trace: &[u8]) -> [u8; 32] {
    sha256_32(opcode_trace)
}

/// Verify a peer-submitted chain against an independently recomputed expected head.
pub fn verify_submitted_chain(chain: &ExecutionHashChain, expected_head: &[u8; 32]) -> bool {
    chain.verify().is_ok() && chain.head == *expected_head
}

pub fn init_hash_chain() {
    println!("Initializing Hash Chains to prove actual computation/routing.");
    let mut chain = ExecutionHashChain::new("boot-peer", 0);
    let d = wasm_exec_digest(b"init");
    let _ = chain.append(0, d);
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
            assert!(chain.append(step, dig).is_some());
        }
        assert_eq!(chain.len(), 5);
        assert!(chain.verify().is_ok());
        assert!(!chain.detect_tamper());
    }

    #[test]
    fn test_tamper_detect_mutated_digest() {
        let mut chain = ExecutionHashChain::new("worker-2", 7);
        let _ = chain.append(0, wasm_exec_digest(b"op-a"));
        let _ = chain.append(1, wasm_exec_digest(b"op-b"));
        let _ = chain.append(2, wasm_exec_digest(b"op-c"));
        assert!(chain.verify().is_ok());

        chain.links[1].exec_digest = wasm_exec_digest(b"TAMPERED");
        assert!(chain.detect_tamper());
        assert_eq!(chain.verify(), Err(1));
    }

    #[test]
    fn test_tamper_detect_mutated_link_hash() {
        let mut chain = ExecutionHashChain::new("worker-3", 99);
        let _ = chain.append(0, wasm_exec_digest(b"x"));
        let _ = chain.append(1, wasm_exec_digest(b"y"));
        chain.links[0].link_hash = [0xFF; 32];
        chain.head = chain.links.last().unwrap().link_hash;
        assert!(chain.detect_tamper());
    }

    #[test]
    fn test_tamper_detect_head_mismatch() {
        let mut chain = ExecutionHashChain::new("worker-4", 1);
        let _ = chain.append(0, wasm_exec_digest(b"step0"));
        chain.head = [0u8; 32];
        assert!(chain.detect_tamper());
    }

    #[test]
    fn test_verify_submitted_chain_expected_head() {
        let mut honest = ExecutionHashChain::new("w", 3);
        let _ = honest.append(0, wasm_exec_digest(b"a"));
        let _ = honest.append(1, wasm_exec_digest(b"b"));
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

    /// Regression test for #200: a 1-link chain must not "prove" a full
    /// execution. Non-increasing steps are rejected by `append`.
    #[test]
    fn test_append_rejects_non_increasing_step() {
        let mut chain = ExecutionHashChain::new("lazy", 1);
        assert!(chain.append(5, wasm_exec_digest(b"only")).is_some());
        // Equal and decreasing steps: rejected, chain unchanged.
        assert!(chain.append(5, wasm_exec_digest(b"dup")).is_none());
        assert!(chain.append(3, wasm_exec_digest(b"back")).is_none());
        assert_eq!(chain.len(), 1);
        assert!(chain.verify().is_ok());
        // A larger step is still accepted afterwards.
        assert!(chain.append(6, wasm_exec_digest(b"next")).is_some());
        assert_eq!(chain.len(), 2);
        assert!(chain.verify().is_ok());
    }

    /// Regression test for #200: `verify` catches chains whose steps were
    /// smuggled in out of order (bypassing `append`).
    #[test]
    fn test_verify_rejects_non_increasing_steps() {
        let mut chain = ExecutionHashChain::new("sneaky", 2);
        let d0 = wasm_exec_digest(b"s0");
        let d1 = wasm_exec_digest(b"s1");
        let h0 = hash_link(&chain.head, 7, &d0);
        chain.links.push(ChainLink {
            step: 7,
            exec_digest: d0,
            link_hash: h0,
        });
        chain.head = h0;
        // Second link with an equal step: hash binding is valid, but the
        // step order is not.
        let h1 = hash_link(&chain.head, 7, &d1);
        chain.links.push(ChainLink {
            step: 7,
            exec_digest: d1,
            link_hash: h1,
        });
        chain.head = h1;
        assert_eq!(chain.verify(), Err(1));
        assert!(chain.detect_tamper());
    }

    /// Digests are real SHA-256 outputs (32 bytes), not widened FNV-1a.
    #[test]
    fn test_digests_are_sha256() {
        use sha2::{Digest, Sha256};
        let expected = Sha256::digest(b"probe");
        assert_eq!(wasm_exec_digest(b"probe"), expected.as_slice());
        let g = genesis_hash("p1", 1);
        assert_eq!(g.len(), 32);
        // Domain separation is preserved across the swap.
        assert_ne!(genesis_hash("p1", 1), genesis_hash("p2", 1));
        assert_ne!(genesis_hash("p1", 1), genesis_hash("p1", 2));
    }
}
