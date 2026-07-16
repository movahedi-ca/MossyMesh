//! Recursive folding abstraction (Nova-style) for constant-size ledger proofs.
//!
//! `fold_proofs` compresses an existing accumulator proof with a new step into a
//! single proof whose public representation remains [`crate::snark::ANCHOR_PROOF_SIZE`]
//! bytes regardless of history length.

use crate::snark::{
    fold_domain, hash_parts, MicroSpartanPreprocessing, PublicInput, SnarkProof, StepInstance,
    ANCHOR_PROOF_SIZE, MAX_VERIFICATION_PAYLOAD_BYTES, MICROSPARTAN_GATE_COUNT,
};
use crate::ConsensusError;

/// Fold `old` accumulator with a new incremental `new_step` into one constant-size proof.
///
/// Interface mirrors Nova IVC: the recursive verifier circuit is represented only by
/// [`MicroSpartanPreprocessing`] metadata (~10k gates); the mock prover hashes
/// commitments deterministically so all nodes derive the same folded public bytes.
pub fn fold_proofs(old: &SnarkProof, new_step: &StepInstance) -> Result<SnarkProof, ConsensusError> {
    if !old.is_well_formed() {
        return Err(ConsensusError::InvalidProof);
    }
    // Linking: the step must extend the state claimed by the accumulator.
    if old.claimed_state_root != new_step.prev_state_root {
        return Err(ConsensusError::InvalidProof);
    }

    let old_parsed = old.parse_public();
    let step_digest = new_step.digest();
    let new_fold = old
        .fold_count
        .checked_add(1)
        .ok_or(ConsensusError::InvalidProof)?;

    // Mock folding: H(fold || old_commitment || step_digest || new_root || fold_count).
    let commitment = hash_parts(&[
        fold_domain(),
        &old_parsed.commitment,
        &step_digest,
        &new_step.next_state_root,
        &new_fold.to_le_bytes(),
    ]);
    // Aggregate step digest chains prior steps into a constant digest.
    let agg_step = hash_parts(&[
        b"agg",
        &old_parsed.step_digest,
        &step_digest,
        &new_fold.to_le_bytes(),
    ]);

    let public = encode_folded_public(&commitment, &agg_step, new_fold);

    let folded = SnarkProof {
        public,
        fold_count: new_fold,
        claimed_state_root: new_step.next_state_root,
    };

    if folded.public_bytes().len() != ANCHOR_PROOF_SIZE {
        return Err(ConsensusError::InvalidProof);
    }
    Ok(folded)
}

/// Verify a folded (or genesis) proof against public inputs.
///
/// Checks structural well-formedness, constant size, fold-count lower bound,
/// claimed final root, and that the verification payload stays sub-megabyte.
pub fn verify_folded_proof(
    proof: &SnarkProof,
    public_input: &PublicInput,
) -> Result<(), ConsensusError> {
    if !proof.is_well_formed() {
        return Err(ConsensusError::InvalidProof);
    }
    if proof.public_bytes().len() != ANCHOR_PROOF_SIZE {
        return Err(ConsensusError::InvalidProof);
    }
    if proof.fold_count < public_input.min_fold_count {
        return Err(ConsensusError::InvalidProof);
    }
    if proof.claimed_state_root != public_input.final_state_root {
        return Err(ConsensusError::InvalidProof);
    }

    let payload = proof.verification_payload_len(public_input);
    if payload >= MAX_VERIFICATION_PAYLOAD_BYTES {
        return Err(ConsensusError::OutOfMemory);
    }

    // For fold_count == 0, claimed root must match genesis.
    if proof.fold_count == 0 && proof.claimed_state_root != public_input.genesis_state_root {
        return Err(ConsensusError::InvalidProof);
    }

    // Structural recompute of public padding/commitment consistency for mock proofs:
    // re-parse and ensure embedded fold_count matches.
    let parsed = proof.parse_public();
    if parsed.fold_count != proof.fold_count {
        return Err(ConsensusError::InvalidProof);
    }

    Ok(())
}

/// Convenience: fold a sequence of steps starting from a genesis root.
pub fn fold_sequence(
    genesis_root: [u8; 32],
    steps: &[StepInstance],
) -> Result<SnarkProof, ConsensusError> {
    let mut acc = SnarkProof::genesis(genesis_root);
    for step in steps {
        acc = fold_proofs(&acc, step)?;
    }
    Ok(acc)
}

/// DOC 34 compatibility: fold two proofs when the second encodes a single step.
/// Prefer [`fold_proofs`] with an explicit [`StepInstance`].
pub fn fold_snarks(old: &SnarkProof, step_proof: &SnarkProof) -> Result<SnarkProof, ConsensusError> {
    if step_proof.fold_count != 1 {
        return Err(ConsensusError::InvalidProof);
    }
    // Recover a step-like instance from claimed roots is incomplete without witness;
    // this path is only for API surface — callers should use `fold_proofs`.
    let step = StepInstance {
        prev_state_root: old.claimed_state_root,
        next_state_root: step_proof.claimed_state_root,
        witness_digest: step_proof.parse_public().step_digest,
    };
    fold_proofs(old, &step)
}

/// Ensure MicroSpartan preprocessing used for the recursive verifier meets size SLAs.
pub fn verify_preprocessing(prep: &MicroSpartanPreprocessing) -> Result<(), ConsensusError> {
    if prep.gate_count != MICROSPARTAN_GATE_COUNT {
        return Err(ConsensusError::InvalidProof);
    }
    if !prep.is_within_bounds() {
        return Err(ConsensusError::OutOfMemory);
    }
    Ok(())
}

fn encode_folded_public(
    commitment: &[u8; 32],
    step_digest: &[u8; 32],
    fold_count: u64,
) -> [u8; ANCHOR_PROOF_SIZE] {
    // Same layout as snark::encode_public (kept local to avoid exposing encoder).
    let mut out = [0u8; ANCHOR_PROOF_SIZE];
    out[0..32].copy_from_slice(commitment);
    out[32..64].copy_from_slice(step_digest);
    out[64..72].copy_from_slice(&fold_count.to_le_bytes());
    out[72] = 1; // folded flag
    let pad = hash_parts(&[b"pad", commitment, step_digest, &fold_count.to_le_bytes()]);
    let mut offset = 73;
    let mut pad_block = pad;
    while offset < ANCHOR_PROOF_SIZE {
        let n = (ANCHOR_PROOF_SIZE - offset).min(32);
        out[offset..offset + n].copy_from_slice(&pad_block[..n]);
        offset += n;
        if offset < ANCHOR_PROOF_SIZE {
            pad_block = hash_parts(&[b"pad/next", &pad_block]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snark::{
        MicroSpartanPreprocessing, ANCHOR_PROOF_SIZE, MAX_VERIFICATION_PAYLOAD_BYTES,
        MICROSPARTAN_GATE_COUNT,
    };

    /// Build N linked steps from `genesis` for property tests.
    fn linked_steps(genesis: [u8; 32], n: u64) -> Vec<StepInstance> {
        let mut steps = Vec::with_capacity(n as usize);
        let mut prev = genesis;
        for i in 0..n {
            let mut next = [0u8; 32];
            next[..8].copy_from_slice(&(i + 1).to_le_bytes());
            let mut w = [0u8; 32];
            w[..8].copy_from_slice(&i.to_le_bytes());
            steps.push(StepInstance {
                prev_state_root: prev,
                next_state_root: next,
                witness_digest: w,
            });
            prev = next;
        }
        steps
    }

    #[test]
    fn fold_preserves_constant_public_size() {
        let genesis = [0xAAu8; 32];
        let mut acc = SnarkProof::genesis(genesis);
        assert_eq!(acc.public_bytes().len(), ANCHOR_PROOF_SIZE);

        for i in 0..64u8 {
            let s = StepInstance {
                prev_state_root: acc.claimed_state_root,
                next_state_root: [i.wrapping_add(1); 32],
                witness_digest: [0x10u8.wrapping_add(i); 32],
            };
            acc = fold_proofs(&acc, &s).expect("fold");
            assert_eq!(
                acc.public_bytes().len(),
                ANCHOR_PROOF_SIZE,
                "proof grew after fold {}",
                i
            );
            assert_eq!(acc.fold_count, (i as u64) + 1);
        }
    }

    /// Lemma 1: |π_pub| = 200 after N folds for several depths (property-style).
    #[test]
    fn size_invariant_holds_after_n_folds() {
        let genesis = [0x11u8; 32];
        for &n in &[0u64, 1, 2, 7, 31, 100, 512] {
            let steps = linked_steps(genesis, n);
            let proof = fold_sequence(genesis, &steps).expect("fold_sequence");
            assert_eq!(
                proof.public_bytes().len(),
                ANCHOR_PROOF_SIZE,
                "size drifted at n={n}"
            );
            assert_eq!(proof.fold_count, n);
            assert!(proof.is_well_formed());
            if n > 0 {
                assert_eq!(proof.parse_public().flags, 1, "folded flag at n={n}");
            }
            // Payload formula stays 312 bytes regardless of n.
            let pi = PublicInput {
                genesis_state_root: genesis,
                final_state_root: proof.claimed_state_root,
                min_fold_count: n,
            };
            assert_eq!(proof.verification_payload_len(&pi), 312);
            verify_folded_proof(&proof, &pi).unwrap();
        }
    }

    #[test]
    fn verify_folded_proof_accepts_valid_chain() {
        let genesis = [1u8; 32];
        let steps = vec![
            StepInstance {
                prev_state_root: genesis,
                next_state_root: [2u8; 32],
                witness_digest: [9u8; 32],
            },
            StepInstance {
                prev_state_root: [2u8; 32],
                next_state_root: [3u8; 32],
                witness_digest: [8u8; 32],
            },
        ];
        let proof = fold_sequence(genesis, &steps).unwrap();
        let pi = PublicInput {
            genesis_state_root: genesis,
            final_state_root: [3u8; 32],
            min_fold_count: 2,
        };
        assert!(verify_folded_proof(&proof, &pi).is_ok());
        assert!(proof.verification_payload_len(&pi) < MAX_VERIFICATION_PAYLOAD_BYTES);
    }

    #[test]
    fn verify_rejects_wrong_final_root() {
        let genesis = [1u8; 32];
        let steps = [StepInstance {
            prev_state_root: genesis,
            next_state_root: [2u8; 32],
            witness_digest: [9u8; 32],
        }];
        let proof = fold_sequence(genesis, &steps).unwrap();
        let pi = PublicInput {
            genesis_state_root: genesis,
            final_state_root: [0xFFu8; 32],
            min_fold_count: 1,
        };
        assert!(matches!(
            verify_folded_proof(&proof, &pi),
            Err(ConsensusError::InvalidProof)
        ));
    }

    #[test]
    fn fold_rejects_unlinked_step() {
        let old = SnarkProof::genesis([1u8; 32]);
        let bad = StepInstance {
            prev_state_root: [9u8; 32], // prev != claimed
            next_state_root: [2u8; 32],
            witness_digest: [3u8; 32],
        };
        assert!(matches!(
            fold_proofs(&old, &bad),
            Err(ConsensusError::InvalidProof)
        ));
    }

    /// Lemma 3: breaking prev continuity mid-chain is rejected; good prefix still size-constant.
    #[test]
    fn fold_rejects_broken_mid_chain_linkage() {
        let genesis = [0xABu8; 32];
        let steps = linked_steps(genesis, 5);
        let mut acc = fold_sequence(genesis, &steps[..3]).unwrap();
        assert_eq!(acc.fold_count, 3);
        assert_eq!(acc.public_bytes().len(), ANCHOR_PROOF_SIZE);

        // Skip root continuity: use step 4's next as prev but wrong claimed link.
        let broken = StepInstance {
            prev_state_root: {
                let mut bad = acc.claimed_state_root;
                bad[0] ^= 0xFF;
                bad
            },
            next_state_root: [0xCDu8; 32],
            witness_digest: [0xEFu8; 32],
        };
        assert!(matches!(
            fold_proofs(&acc, &broken),
            Err(ConsensusError::InvalidProof)
        ));

        // Correct link still works and preserves size.
        let ok = StepInstance {
            prev_state_root: acc.claimed_state_root,
            next_state_root: [0x10u8; 32],
            witness_digest: [0x20u8; 32],
        };
        acc = fold_proofs(&acc, &ok).unwrap();
        assert_eq!(acc.fold_count, 4);
        assert_eq!(acc.public_bytes().len(), ANCHOR_PROOF_SIZE);
    }

    #[test]
    fn verification_payload_stays_sub_megabyte_after_many_folds() {
        let genesis = [0u8; 32];
        let mut acc = SnarkProof::genesis(genesis);
        for i in 0..256u64 {
            let s = StepInstance {
                prev_state_root: acc.claimed_state_root,
                next_state_root: {
                    let mut r = [0u8; 32];
                    r[..8].copy_from_slice(&(i + 1).to_le_bytes());
                    r
                },
                witness_digest: {
                    let mut w = [0u8; 32];
                    w[..8].copy_from_slice(&i.to_le_bytes());
                    w
                },
            };
            acc = fold_proofs(&acc, &s).unwrap();
        }
        let pi = PublicInput {
            genesis_state_root: genesis,
            final_state_root: acc.claimed_state_root,
            min_fold_count: 256,
        };
        let payload = acc.verification_payload_len(&pi);
        assert!(
            payload < MAX_VERIFICATION_PAYLOAD_BYTES,
            "payload {payload} exceeded 1 MiB"
        );
        // Anchor remains ~200 bytes even after 256 steps.
        assert_eq!(acc.public_bytes().len(), 200);
        verify_folded_proof(&acc, &pi).unwrap();
    }

    #[test]
    fn microsparatan_gate_budget_is_10k() {
        let prep = MicroSpartanPreprocessing::preprocess(b"fold-verifier");
        assert_eq!(prep.gate_count, MICROSPARTAN_GATE_COUNT);
        verify_preprocessing(&prep).unwrap();
        assert!(prep.payload_len() < MAX_VERIFICATION_PAYLOAD_BYTES);
    }

    #[test]
    fn fold_is_deterministic() {
        let genesis = [5u8; 32];
        let s = StepInstance {
            prev_state_root: genesis,
            next_state_root: [6u8; 32],
            witness_digest: [7u8; 32],
        };
        let a = fold_proofs(&SnarkProof::genesis(genesis), &s).unwrap();
        let b = fold_proofs(&SnarkProof::genesis(genesis), &s).unwrap();
        assert_eq!(a, b);
    }

    /// Lemma 5: multi-step fold_sequence is deterministic (same inputs → same proof).
    #[test]
    fn multi_step_fold_is_deterministic() {
        let genesis = [0x55u8; 32];
        let steps = linked_steps(genesis, 17);
        let a = fold_sequence(genesis, &steps).unwrap();
        let b = fold_sequence(genesis, &steps).unwrap();
        assert_eq!(a, b);
        assert_eq!(a.public, b.public);
        assert_eq!(a.fold_count, 17);
        assert_eq!(a.public_bytes().len(), ANCHOR_PROOF_SIZE);
    }

    /// Different witnesses at the same root positions yield different folded commitments.
    #[test]
    fn fold_commitment_binds_witness_digest() {
        let genesis = [1u8; 32];
        let s_a = StepInstance {
            prev_state_root: genesis,
            next_state_root: [2u8; 32],
            witness_digest: [0xAAu8; 32],
        };
        let s_b = StepInstance {
            prev_state_root: genesis,
            next_state_root: [2u8; 32],
            witness_digest: [0xBBu8; 32],
        };
        let p_a = fold_proofs(&SnarkProof::genesis(genesis), &s_a).unwrap();
        let p_b = fold_proofs(&SnarkProof::genesis(genesis), &s_b).unwrap();
        assert_ne!(p_a.public, p_b.public);
        assert_eq!(p_a.fold_count, p_b.fold_count);
        assert_eq!(p_a.claimed_state_root, p_b.claimed_state_root);
        assert_eq!(p_a.public_bytes().len(), ANCHOR_PROOF_SIZE);
        assert_eq!(p_b.public_bytes().len(), ANCHOR_PROOF_SIZE);
    }

    /// Fold uses DOMAIN_FOLD (via fold_domain), not the leaf DOMAIN_PROOF path alone.
    #[test]
    fn fold_uses_fold_domain_not_leaf_only() {
        let genesis = [3u8; 32];
        let step = StepInstance {
            prev_state_root: genesis,
            next_state_root: [4u8; 32],
            witness_digest: [5u8; 32],
        };
        let leaf = SnarkProof::from_step(&step);
        let folded = fold_proofs(&SnarkProof::genesis(genesis), &step).unwrap();
        // Both have fold_count 1 and same claimed root, but different commitments
        // (genesis-fold chain vs leaf-only DOMAIN_PROOF encoding).
        assert_eq!(leaf.fold_count, 1);
        assert_eq!(folded.fold_count, 1);
        assert_eq!(leaf.claimed_state_root, folded.claimed_state_root);
        assert_ne!(leaf.parse_public().commitment, folded.parse_public().commitment);
        assert_eq!(folded.parse_public().flags, 1);
        assert_eq!(leaf.parse_public().flags, 0);
    }

    #[test]
    fn verify_rejects_tampered_public_bytes() {
        let genesis = [1u8; 32];
        let steps = [StepInstance {
            prev_state_root: genesis,
            next_state_root: [2u8; 32],
            witness_digest: [9u8; 32],
        }];
        let mut proof = fold_sequence(genesis, &steps).unwrap();
        // Flip a commitment byte while leaving fold_count field alone → ill-formed or
        // still fails well-formedness if we also corrupt embedded fold_count.
        proof.public[0] ^= 0xFF;
        proof.public[64] ^= 0x01; // desync embedded fold_count from struct field
        let pi = PublicInput {
            genesis_state_root: genesis,
            final_state_root: [2u8; 32],
            min_fold_count: 1,
        };
        assert!(matches!(
            verify_folded_proof(&proof, &pi),
            Err(ConsensusError::InvalidProof)
        ));
    }

    /// Structural well-formedness alone does not stop commitment-only bit flips when
    /// fold_count embedding stays synced — documents mock malleability limits.
    #[test]
    fn mock_verify_does_not_recheck_commitment_chain() {
        let genesis = [1u8; 32];
        let steps = [StepInstance {
            prev_state_root: genesis,
            next_state_root: [2u8; 32],
            witness_digest: [9u8; 32],
        }];
        let mut proof = fold_sequence(genesis, &steps).unwrap();
        // Flip only commitment byte; keep embedded fold_count == struct fold_count.
        proof.public[0] ^= 0x01;
        // Re-derive padding would change in honest encode; we leave stale padding.
        // Well-formedness only checks fold_count sync + length.
        assert!(proof.is_well_formed());
        let pi = PublicInput {
            genesis_state_root: genesis,
            final_state_root: [2u8; 32],
            min_fold_count: 1,
        };
        // Mock verify still accepts — honest note: not knowledge-sound.
        assert!(
            verify_folded_proof(&proof, &pi).is_ok(),
            "mock verify is structural only; real Nova must re-check the fold transcript"
        );
    }

    #[test]
    fn verify_rejects_under_min_fold_count() {
        let genesis = [1u8; 32];
        let proof = SnarkProof::genesis(genesis);
        let pi = PublicInput {
            genesis_state_root: genesis,
            final_state_root: genesis,
            min_fold_count: 1,
        };
        assert!(matches!(
            verify_folded_proof(&proof, &pi),
            Err(ConsensusError::InvalidProof)
        ));
    }

    #[test]
    fn verify_rejects_genesis_with_mismatched_public_input_root() {
        let proof = SnarkProof::genesis([1u8; 32]);
        let pi = PublicInput {
            genesis_state_root: [2u8; 32],
            final_state_root: [1u8; 32], // matches claimed but not genesis when n=0
            min_fold_count: 0,
        };
        assert!(matches!(
            verify_folded_proof(&proof, &pi),
            Err(ConsensusError::InvalidProof)
        ));
    }

    #[test]
    fn fold_count_overflow_is_rejected() {
        let mut acc = SnarkProof::genesis([0u8; 32]);
        acc.fold_count = u64::MAX;
        // Keep public embedding in sync so we hit checked_add, not well-formedness.
        let mut public = acc.public;
        public[64..72].copy_from_slice(&u64::MAX.to_le_bytes());
        acc.public = public;
        assert!(acc.is_well_formed());
        let step = StepInstance {
            prev_state_root: acc.claimed_state_root,
            next_state_root: [1u8; 32],
            witness_digest: [2u8; 32],
        };
        assert!(matches!(
            fold_proofs(&acc, &step),
            Err(ConsensusError::InvalidProof)
        ));
    }

    #[test]
    fn verify_preprocessing_rejects_wrong_gate_count() {
        let mut prep = MicroSpartanPreprocessing::preprocess(b"x");
        prep.gate_count = MICROSPARTAN_GATE_COUNT + 1;
        assert!(matches!(
            verify_preprocessing(&prep),
            Err(ConsensusError::InvalidProof)
        ));
    }
}
