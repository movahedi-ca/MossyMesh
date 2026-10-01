//! Swappable folding-verifier interface for Nova-style recursive proofs.
//!
//! The only working backend today is [`MockFoldingVerifier`]: a deterministic
//! hash-based mock that preserves the constant-size public layout so wiring,
//! radio anchoring, and edge tests can be built against a stable API. It is
//! NOT a zero-knowledge proof and MUST NOT be presented as one.
//!
//! [`NovaSnarkVerifier`] is the structural placeholder for the real backend
//! (nova-snark over the Pallas/Vesta cycle, with the
//! [`MicroSpartanPreprocessing`](crate::snark::MicroSpartanPreprocessing)
//! circuit description). It is deliberately inert: every method returns
//! [`ConsensusError::SnarkError`] until the `nova-snark` dependency is wired.

use crate::folding::{fold_proofs, verify_folded_proof};
use crate::snark::{MicroSpartanPreprocessing, PublicInput, SnarkProof, StepInstance};
use crate::ConsensusError;

/// Verifier backend for folded execution proofs.
///
/// Backends fold steps into an accumulator and decide acceptance of folded
/// proofs against public inputs. The mock backend implements the structural
/// checks; the real Nova backend will implement recursive proof verification.
pub trait FoldingVerifier {
    /// Human-readable backend label (e.g. `"mock-sha256"`, `"nova-snark/pallas"`).
    fn backend_name(&self) -> &'static str;

    /// Fold one new step into an accumulator proof.
    fn fold(&self, old: &SnarkProof, new_step: &StepInstance)
        -> Result<SnarkProof, ConsensusError>;

    /// Verify a folded proof against public inputs.
    fn verify(&self, proof: &SnarkProof, public_input: &PublicInput) -> Result<(), ConsensusError>;
}

/// Deterministic mock backend: structural checks only, no cryptography.
///
/// Delegates to the domain-separated SHA-256 mock in [`crate::folding`].
/// Safe for layout, serialization, radio-anchor, and determinism tests.
/// Anything that needs actual soundness must use a real backend.
#[derive(Debug, Default, Clone, Copy)]
pub struct MockFoldingVerifier;

impl FoldingVerifier for MockFoldingVerifier {
    fn backend_name(&self) -> &'static str {
        "mock-sha256"
    }

    fn fold(
        &self,
        old: &SnarkProof,
        new_step: &StepInstance,
    ) -> Result<SnarkProof, ConsensusError> {
        fold_proofs(old, new_step)
    }

    fn verify(&self, proof: &SnarkProof, public_input: &PublicInput) -> Result<(), ConsensusError> {
        verify_folded_proof(proof, public_input)
    }
}

/// Structural placeholder for the production Nova-SNARK backend.
///
/// Intended wiring (not yet implemented):
/// - add `nova-snark` as an optional dependency of the `consensus` crate,
/// - define the recursive step circuit (R1CS over Pallas) with the gate budget
///   recorded in [`MicroSpartanPreprocessing`],
/// - implement [`FoldingVerifier`] for this type against
///   `nova_snark::RecursiveSNARK`, and
/// - switch [`default_verifier`] to return it behind a cargo feature.
///
/// Until then every method returns [`ConsensusError::SnarkError`]: the type
/// exists so call sites can be written against the trait today without
/// pretending the mock is a proof.
#[derive(Debug, Clone)]
pub struct NovaSnarkVerifier {
    preprocessing: MicroSpartanPreprocessing,
}

impl NovaSnarkVerifier {
    /// Describe the verifier circuit the real backend will check proofs against.
    pub fn new(preprocessing: MicroSpartanPreprocessing) -> Self {
        Self { preprocessing }
    }

    /// The preprocessing artifact the real backend would verify against.
    pub fn preprocessing(&self) -> &MicroSpartanPreprocessing {
        &self.preprocessing
    }
}

impl FoldingVerifier for NovaSnarkVerifier {
    fn backend_name(&self) -> &'static str {
        "nova-snark/pallas (unwired)"
    }

    fn fold(
        &self,
        _old: &SnarkProof,
        _new_step: &StepInstance,
    ) -> Result<SnarkProof, ConsensusError> {
        Err(ConsensusError::SnarkError(
            "nova-snark backend is not wired: add the nova-snark dependency and implement RecursiveSNARK folding"
                .into(),
        ))
    }

    fn verify(
        &self,
        _proof: &SnarkProof,
        _public_input: &PublicInput,
    ) -> Result<(), ConsensusError> {
        Err(ConsensusError::SnarkError(
            "nova-snark backend is not wired: the mock layout is not a proof".into(),
        ))
    }
}

/// The backend the node uses today. Swap this constructor when the Nova
/// backend lands; all call sites go through [`FoldingVerifier`] and keep working.
pub fn default_verifier() -> MockFoldingVerifier {
    MockFoldingVerifier
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chain() -> (SnarkProof, PublicInput) {
        let genesis = [11u8; 32];
        let v = MockFoldingVerifier;
        let mut acc = SnarkProof::genesis(genesis);
        for i in 0..3u8 {
            let step = StepInstance {
                prev_state_root: acc.claimed_state_root,
                next_state_root: [i + 20; 32],
                witness_digest: [i + 40; 32],
            };
            acc = v.fold(&acc, &step).expect("mock fold");
        }
        let pi = PublicInput {
            genesis_state_root: genesis,
            final_state_root: acc.claimed_state_root,
            min_fold_count: 3,
        };
        (acc, pi)
    }

    #[test]
    fn mock_verifier_accepts_folded_chain() {
        let v = MockFoldingVerifier;
        let (proof, pi) = chain();
        assert!(v.verify(&proof, &pi).is_ok());
        assert_eq!(v.backend_name(), "mock-sha256");
    }

    #[test]
    fn mock_verifier_rejects_wrong_final_root() {
        let v = MockFoldingVerifier;
        let (proof, mut pi) = chain();
        pi.final_state_root = [0xFF; 32];
        assert!(matches!(
            v.verify(&proof, &pi),
            Err(ConsensusError::InvalidProof)
        ));
    }

    #[test]
    fn trait_object_is_swappable() {
        let backends: Vec<Box<dyn FoldingVerifier>> = vec![
            Box::new(MockFoldingVerifier),
            Box::new(NovaSnarkVerifier::new(
                MicroSpartanPreprocessing::preprocess(b"swap"),
            )),
        ];
        let (proof, pi) = chain();
        assert!(backends[0].verify(&proof, &pi).is_ok());
        // The real backend is structured but unwired: honest error, never a
        // fake accept.
        assert!(matches!(
            backends[1].verify(&proof, &pi),
            Err(ConsensusError::SnarkError(_))
        ));
    }

    #[test]
    fn nova_backend_fold_is_unwired() {
        let v = NovaSnarkVerifier::new(MicroSpartanPreprocessing::preprocess(b"fold"));
        let (proof, _) = chain();
        let step = StepInstance {
            prev_state_root: proof.claimed_state_root,
            next_state_root: [9u8; 32],
            witness_digest: [8u8; 32],
        };
        assert!(matches!(
            v.fold(&proof, &step),
            Err(ConsensusError::SnarkError(_))
        ));
    }

    #[test]
    fn default_verifier_is_mock() {
        let v = default_verifier();
        let (proof, pi) = chain();
        assert!(v.verify(&proof, &pi).is_ok());
    }
}
