//! Constant-size SNARK public representations for MossyMesh ledger compression.
//!
//! Real `nova-snark` (Pallas/Vesta recursive folding) is heavy for edge targets.
//! This module exposes a stable interface and size bounds so radio anchoring and
//! edge verification can depend on fixed layouts. A deterministic mock prover
//! preserves those bounds; a future soft-dep can plug into the same types.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Public SNARK representation size for HF/Ham radio anchoring (~200 bytes).
/// README: "Securely anchor 200-byte ZK-SNARK ledger proofs to neighboring macro-islands".
pub const ANCHOR_PROOF_SIZE: usize = 200;

/// Hard cap on any verification payload (sub-megabyte constant proof for edge nodes).
pub const MAX_VERIFICATION_PAYLOAD_BYTES: usize = 1_048_576; // 1 MiB

/// MicroSpartan-style verification circuit gate budget (constant-sized preprocessing).
/// README risk mitigation: "~10,000 gates".
pub const MICROSPARTAN_GATE_COUNT: usize = 10_000;

/// Maximum bytes retained for MicroSpartan preprocessing metadata (not the full circuit).
pub const MICROSPARTAN_PREPROCESS_META_BYTES: usize = 512;

/// Domain separator for mock commitment chains (keeps folds deterministic across devices).
const DOMAIN_PROOF: &[u8] = b"mossymesh/snark/v1";
const DOMAIN_STEP: &[u8] = b"mossymesh/step/v1";
const DOMAIN_FOLD: &[u8] = b"mossymesh/fold/v1";

/// Fixed-size public SNARK proof used for ledger history compression and radio anchors.
///
/// The on-wire / public representation is always [`ANCHOR_PROOF_SIZE`] bytes so
/// proof size does not grow with the number of folded state transitions.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnarkProof {
    /// Constant-size public representation (exactly 200 bytes via [`Self::public_bytes`]).
    /// Layout: commitment(32) || step_digest(32) || fold_count(8 LE) || flags(1) || padding.
    #[serde(with = "serde_bytes_array_200")]
    pub public: [u8; ANCHOR_PROOF_SIZE],
    /// Number of incremental steps folded into this proof (also embedded in `public`).
    pub fold_count: u64,
    /// Digest of the claimed final state root after the folded sequence.
    pub claimed_state_root: [u8; 32],
}

mod serde_bytes_array_200 {
    use super::ANCHOR_PROOF_SIZE;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S>(bytes: &[u8; ANCHOR_PROOF_SIZE], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        bytes.as_slice().serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<[u8; ANCHOR_PROOF_SIZE], D::Error>
    where
        D: Deserializer<'de>,
    {
        let v = Vec::<u8>::deserialize(deserializer)?;
        if v.len() != ANCHOR_PROOF_SIZE {
            return Err(serde::de::Error::custom(format!(
                "SnarkProof public must be {} bytes, got {}",
                ANCHOR_PROOF_SIZE,
                v.len()
            )));
        }
        let mut arr = [0u8; ANCHOR_PROOF_SIZE];
        arr.copy_from_slice(&v);
        Ok(arr)
    }
}

/// One incremental state-transition step to be folded into an existing proof.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepInstance {
    /// Merkle / trie root before the step.
    pub prev_state_root: [u8; 32],
    /// Merkle / trie root after the step.
    pub next_state_root: [u8; 32],
    /// Opaque step witness digest (e.g. WASM execution trace hash). Not expanded here.
    pub witness_digest: [u8; 32],
}

/// Public inputs checked by [`crate::folding::verify_folded_proof`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicInput {
    /// Expected initial (genesis or checkpoint) state root before any folded steps.
    pub genesis_state_root: [u8; 32],
    /// Expected final state root claimed by the folded proof.
    pub final_state_root: [u8; 32],
    /// Minimum number of steps that must have been folded (0 allows empty identity).
    pub min_fold_count: u64,
}

/// MicroSpartan-style preprocessing artifact: gate-count metadata only (not a full SRS dump).
///
/// Keeps verification circuit description constant-sized (~10k gates) so edge RAM
/// is not exhausted by growing circuits as ledger history lengthens.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MicroSpartanPreprocessing {
    /// Declared arithmetic gate count for the verification circuit.
    pub gate_count: usize,
    /// Compact circuit / R1CS metadata (hashes, parameter ids) — bounded size.
    pub metadata: Vec<u8>,
    /// Hash of the metadata for integrity checks.
    pub metadata_digest: [u8; 32],
}

impl SnarkProof {
    /// Build a genesis (identity) proof for `state_root` with fold_count = 0.
    pub fn genesis(state_root: [u8; 32]) -> Self {
        let commitment = hash_parts(&[DOMAIN_PROOF, &state_root, &0u64.to_le_bytes()]);
        let step_digest = hash_parts(&[DOMAIN_STEP, &state_root]);
        let public = encode_public(&commitment, &step_digest, 0, 0);
        Self {
            public,
            fold_count: 0,
            claimed_state_root: state_root,
        }
    }

    /// Produce a single-step proof for `step` (fold_count = 1). Used as the `new_step` leaf.
    pub fn from_step(step: &StepInstance) -> Self {
        let step_digest = step.digest();
        let commitment = hash_parts(&[
            DOMAIN_PROOF,
            &step.prev_state_root,
            &step.next_state_root,
            &step.witness_digest,
            &1u64.to_le_bytes(),
        ]);
        let public = encode_public(&commitment, &step_digest, 1, 0);
        Self {
            public,
            fold_count: 1,
            claimed_state_root: step.next_state_root,
        }
    }

    /// Exact 200-byte public representation suitable for radio anchoring.
    pub fn public_bytes(&self) -> &[u8; ANCHOR_PROOF_SIZE] {
        &self.public
    }

    /// Legacy-compatible view of the public bytes (`proof_bytes` field from older stub).
    pub fn proof_bytes(&self) -> Vec<u8> {
        self.public.to_vec()
    }

    /// Total bytes of the verification payload (public proof + lightweight public inputs).
    pub fn verification_payload_len(&self, public_input: &PublicInput) -> usize {
        ANCHOR_PROOF_SIZE
            + public_input.genesis_state_root.len()
            + public_input.final_state_root.len()
            + 8 // min_fold_count
            + 8 // fold_count
            + 32 // claimed_state_root
    }

    /// Decode commitment and step digest embedded in the public blob.
    pub fn parse_public(&self) -> ParsedPublic {
        let mut commitment = [0u8; 32];
        let mut step_digest = [0u8; 32];
        commitment.copy_from_slice(&self.public[0..32]);
        step_digest.copy_from_slice(&self.public[32..64]);
        let mut fold_buf = [0u8; 8];
        fold_buf.copy_from_slice(&self.public[64..72]);
        let fold_count = u64::from_le_bytes(fold_buf);
        let flags = self.public[72];
        ParsedPublic {
            commitment,
            step_digest,
            fold_count,
            flags,
        }
    }

    /// Internal consistency: embedded fold_count matches struct field and layout is full 200 bytes.
    pub fn is_well_formed(&self) -> bool {
        self.public.len() == ANCHOR_PROOF_SIZE && self.parse_public().fold_count == self.fold_count
    }
}

/// Fields recovered from the constant-size public blob.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedPublic {
    pub commitment: [u8; 32],
    pub step_digest: [u8; 32],
    pub fold_count: u64,
    pub flags: u8,
}

impl StepInstance {
    pub fn digest(&self) -> [u8; 32] {
        hash_parts(&[
            DOMAIN_STEP,
            &self.prev_state_root,
            &self.next_state_root,
            &self.witness_digest,
        ])
    }
}

impl MicroSpartanPreprocessing {
    /// Build constant-size preprocessing metadata for a MicroSpartan-style verifier circuit.
    pub fn preprocess(circuit_id: &[u8]) -> Self {
        let gate_count = MICROSPARTAN_GATE_COUNT;
        let mut metadata = Vec::with_capacity(MICROSPARTAN_PREPROCESS_META_BYTES);
        metadata.extend_from_slice(b"microspar/v1");
        metadata.extend_from_slice(&(gate_count as u32).to_le_bytes());
        let id_hash = hash_parts(&[b"circuit", circuit_id]);
        metadata.extend_from_slice(&id_hash);
        // Pad/truncate to fixed metadata budget so size never drifts with circuit_id length.
        metadata.resize(MICROSPARTAN_PREPROCESS_META_BYTES, 0);
        let metadata_digest = hash_parts(&[b"preprocess", &metadata]);
        Self {
            gate_count,
            metadata,
            metadata_digest,
        }
    }

    /// Serialized size of this preprocessing artifact (must stay well under 1 MiB).
    pub fn payload_len(&self) -> usize {
        8 + self.metadata.len() + 32 // gate_count + metadata + digest
    }

    pub fn is_within_bounds(&self) -> bool {
        self.gate_count == MICROSPARTAN_GATE_COUNT
            && self.metadata.len() <= MICROSPARTAN_PREPROCESS_META_BYTES
            && self.payload_len() < MAX_VERIFICATION_PAYLOAD_BYTES
    }
}

/// Encode the fixed 200-byte public layout.
fn encode_public(
    commitment: &[u8; 32],
    step_digest: &[u8; 32],
    fold_count: u64,
    flags: u8,
) -> [u8; ANCHOR_PROOF_SIZE] {
    let mut out = [0u8; ANCHOR_PROOF_SIZE];
    out[0..32].copy_from_slice(commitment);
    out[32..64].copy_from_slice(step_digest);
    out[64..72].copy_from_slice(&fold_count.to_le_bytes());
    out[72] = flags;
    // Remainder is deterministic padding derived from the commitment so unused
    // bytes are not free-form attacker-controlled zeros only.
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

pub(crate) fn hash_parts(parts: &[&[u8]]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    for p in parts {
        hasher.update((p.len() as u64).to_le_bytes());
        hasher.update(p);
    }
    let digest = hasher.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    out
}

pub(crate) fn fold_domain() -> &'static [u8] {
    DOMAIN_FOLD
}

/// Deterministic mock verify of a single-step or genesis proof structure (no folding).
pub fn verify_snark(proof: &SnarkProof) -> bool {
    proof.is_well_formed() && proof.public_bytes().len() == ANCHOR_PROOF_SIZE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_representation_is_exactly_200_bytes() {
        let root = [7u8; 32];
        let proof = SnarkProof::genesis(root);
        assert_eq!(proof.public_bytes().len(), ANCHOR_PROOF_SIZE);
        assert_eq!(proof.proof_bytes().len(), ANCHOR_PROOF_SIZE);
        assert!(proof.is_well_formed());
        assert_eq!(proof.fold_count, 0);
        assert_eq!(proof.claimed_state_root, root);
    }

    #[test]
    fn microsparatan_preprocess_is_constant_and_bounded() {
        let prep = MicroSpartanPreprocessing::preprocess(b"ledger-step-v1");
        assert_eq!(prep.gate_count, MICROSPARTAN_GATE_COUNT);
        assert_eq!(prep.metadata.len(), MICROSPARTAN_PREPROCESS_META_BYTES);
        assert!(prep.is_within_bounds());
        assert!(prep.payload_len() < MAX_VERIFICATION_PAYLOAD_BYTES);
        // Different circuit ids → different digests, same sizes.
        let prep2 = MicroSpartanPreprocessing::preprocess(b"other");
        assert_eq!(prep2.metadata.len(), prep.metadata.len());
        assert_ne!(prep.metadata_digest, prep2.metadata_digest);
    }

    #[test]
    fn step_proof_embeds_fold_count_one() {
        let step = StepInstance {
            prev_state_root: [1u8; 32],
            next_state_root: [2u8; 32],
            witness_digest: [3u8; 32],
        };
        let p = SnarkProof::from_step(&step);
        assert_eq!(p.fold_count, 1);
        assert_eq!(p.parse_public().fold_count, 1);
        assert!(verify_snark(&p));
    }
}
