//! ZK-blinded voting for verified edge nodes.
//!
//! Ballots use a commit–reveal scheme. The commit blinds the choice with a
//! nonce and optional blinding factor; reveal opens the ballot for tallying.
//!
//! # Cryptography status (honest stub)
//!
//! [`BlindingProof`] is **not** a zero-knowledge proof. It is a deterministic
//! SHA-256 binding of `(domain || commitment || voter)` that lets unit tests
//! reject tampered commits without network or a proving system. Production
//! must replace [`BlindingProof::stub`] / [`BlindingProof::verify`] with a real
//! SNARK (or similar) that proves the commitment was well-formed without
//! revealing the choice until the reveal phase.
//!
//! Commitments themselves are fully specified and deterministic:
//! ```text
//! H( domain
//!    || le64(proposal_id)
//!    || voter[32]
//!    || choice_byte
//!    || le64(nonce_len) || nonce
//!    || le64(blinding_len) || blinding )
//! ```
//! Length prefixes make the encoding unambiguous. Commit–reveal integrity and
//! tally logic are fully tested offline (no network).

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::wot::NodeId;

/// Domain tag for ballot commitments (versioned; changing it invalidates all commits).
pub const COMMIT_DOMAIN: &[u8] = b"mm-zk-ballot-v1";

/// Domain tag for stub blinding proofs.
pub const PROOF_DOMAIN: &[u8] = b"mm-blind-proof-v1";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BallotChoice {
    Yes,
    No,
    Abstain,
}

impl BallotChoice {
    pub fn as_byte(self) -> u8 {
        match self {
            BallotChoice::Yes => 1,
            BallotChoice::No => 2,
            BallotChoice::Abstain => 3,
        }
    }

    pub fn from_byte(b: u8) -> Option<Self> {
        match b {
            1 => Some(BallotChoice::Yes),
            2 => Some(BallotChoice::No),
            3 => Some(BallotChoice::Abstain),
            _ => None,
        }
    }
}

/// Stub ZK blinding proof: attests the commit was formed under a known domain
/// binding without revealing the choice until reveal.
///
/// **Not a SNARK.** See module docs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlindingProof {
    /// `SHA-256(PROOF_DOMAIN || commitment || voter)` — placeholder "proof".
    pub proof_digest: [u8; 32],
}

impl BlindingProof {
    /// Build the deterministic stub proof for `(commitment, voter)`.
    pub fn stub(commitment: &[u8; 32], voter: &NodeId) -> Self {
        Self {
            proof_digest: Self::digest(commitment, voter),
        }
    }

    /// Pure digest used by stub and verify (deterministic, no I/O).
    pub fn digest(commitment: &[u8; 32], voter: &NodeId) -> [u8; 32] {
        let mut h = Sha256::new();
        h.update(PROOF_DOMAIN);
        h.update(commitment);
        h.update(voter.0);
        let out = h.finalize();
        let mut proof_digest = [0u8; 32];
        proof_digest.copy_from_slice(&out);
        proof_digest
    }

    /// Verify stub proof binds exactly this commitment and voter.
    pub fn verify(&self, commitment: &[u8; 32], voter: &NodeId) -> bool {
        self.proof_digest == Self::digest(commitment, voter)
    }
}

/// A committed (still blinded) ballot.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlindedBallot {
    pub voter: NodeId,
    pub proposal_id: u64,
    pub commitment: [u8; 32],
    pub proof: BlindingProof,
}

/// Opened ballot after successful reveal.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevealedBallot {
    pub voter: NodeId,
    pub proposal_id: u64,
    pub choice: BallotChoice,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VotingError {
    NotEligible,
    AlreadyCommitted,
    NoCommitment,
    AlreadyRevealed,
    InvalidReveal,
    InvalidProof,
    UnknownProposal,
    VotingClosed,
}

/// Aggregate tally for a single proposal.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoteTally {
    pub yes: u64,
    pub no: u64,
    pub abstain: u64,
    pub revealed: u64,
}

impl VoteTally {
    pub fn total_cast(&self) -> u64 {
        self.yes + self.no + self.abstain
    }

    /// Simple majority among Yes/No (abstain ignored). Tie → false.
    pub fn majority_yes(&self) -> bool {
        self.yes > self.no
    }
}

/// Commit–reveal registry for ZK-blinded edge voting.
#[derive(Clone, Debug, Default)]
pub struct ZkBlindedVoting {
    /// Eligible voters (typically WoT-onboarded non-malicious nodes).
    eligible: HashMap<NodeId, bool>,
    /// Open proposals.
    open: HashMap<u64, bool>,
    /// Commitments: (proposal_id, voter) → ballot
    commits: HashMap<(u64, NodeId), BlindedBallot>,
    /// Revealed choices
    reveals: HashMap<(u64, NodeId), BallotChoice>,
}

impl ZkBlindedVoting {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_eligible(&mut self, voter: NodeId, eligible: bool) {
        self.eligible.insert(voter, eligible);
    }

    pub fn open_proposal(&mut self, proposal_id: u64) {
        self.open.insert(proposal_id, true);
    }

    pub fn close_proposal(&mut self, proposal_id: u64) {
        self.open.insert(proposal_id, false);
    }

    fn is_open(&self, proposal_id: u64) -> bool {
        self.open.get(&proposal_id).copied().unwrap_or(false)
    }

    fn is_eligible(&self, voter: &NodeId) -> bool {
        self.eligible.get(voter).copied().unwrap_or(false)
    }

    /// Create a commitment with length-prefixed nonce/blinding (deterministic).
    ///
    /// Encoding:
    /// `H(COMMIT_DOMAIN || le64(proposal_id) || voter || choice
    ///    || le64(nonce_len) || nonce || le64(blinding_len) || blinding)`
    pub fn make_commitment(
        proposal_id: u64,
        voter: &NodeId,
        choice: BallotChoice,
        nonce: &[u8],
        blinding: &[u8],
    ) -> [u8; 32] {
        let mut h = Sha256::new();
        h.update(COMMIT_DOMAIN);
        h.update(proposal_id.to_le_bytes());
        h.update(voter.0);
        h.update([choice.as_byte()]);
        h.update((nonce.len() as u64).to_le_bytes());
        h.update(nonce);
        h.update((blinding.len() as u64).to_le_bytes());
        h.update(blinding);
        let out = h.finalize();
        let mut commitment = [0u8; 32];
        commitment.copy_from_slice(&out);
        commitment
    }

    /// Build a blinded ballot (commit phase) with stub ZK proof.
    pub fn prepare_ballot(
        voter: NodeId,
        proposal_id: u64,
        choice: BallotChoice,
        nonce: &[u8],
        blinding: &[u8],
    ) -> BlindedBallot {
        let commitment = Self::make_commitment(proposal_id, &voter, choice, nonce, blinding);
        let proof = BlindingProof::stub(&commitment, &voter);
        BlindedBallot {
            voter,
            proposal_id,
            commitment,
            proof,
        }
    }

    /// Submit commitment during the commit phase.
    pub fn commit(&mut self, ballot: BlindedBallot) -> Result<(), VotingError> {
        if !self.is_open(ballot.proposal_id) {
            return Err(VotingError::VotingClosed);
        }
        if !self.is_eligible(&ballot.voter) {
            return Err(VotingError::NotEligible);
        }
        if !ballot.proof.verify(&ballot.commitment, &ballot.voter) {
            return Err(VotingError::InvalidProof);
        }
        let key = (ballot.proposal_id, ballot.voter);
        if self.commits.contains_key(&key) {
            return Err(VotingError::AlreadyCommitted);
        }
        self.commits.insert(key, ballot);
        Ok(())
    }

    /// Reveal phase: open the ballot with the original secrets.
    pub fn reveal(
        &mut self,
        voter: NodeId,
        proposal_id: u64,
        choice: BallotChoice,
        nonce: &[u8],
        blinding: &[u8],
    ) -> Result<RevealedBallot, VotingError> {
        let key = (proposal_id, voter);
        let committed = self.commits.get(&key).ok_or(VotingError::NoCommitment)?;
        if self.reveals.contains_key(&key) {
            return Err(VotingError::AlreadyRevealed);
        }

        let expected = Self::make_commitment(proposal_id, &voter, choice, nonce, blinding);
        if expected != committed.commitment {
            return Err(VotingError::InvalidReveal);
        }

        self.reveals.insert(key, choice);
        Ok(RevealedBallot {
            voter,
            proposal_id,
            choice,
        })
    }

    /// Tally all revealed ballots for a proposal.
    pub fn tally(&self, proposal_id: u64) -> VoteTally {
        let mut t = VoteTally::default();
        for ((pid, _), choice) in &self.reveals {
            if *pid != proposal_id {
                continue;
            }
            t.revealed += 1;
            match choice {
                BallotChoice::Yes => t.yes += 1,
                BallotChoice::No => t.no += 1,
                BallotChoice::Abstain => t.abstain += 1,
            }
        }
        t
    }

    pub fn commitment_count(&self, proposal_id: u64) -> usize {
        self.commits
            .keys()
            .filter(|(p, _)| *p == proposal_id)
            .count()
    }

    pub fn has_commitment(&self, proposal_id: u64, voter: &NodeId) -> bool {
        self.commits.contains_key(&(proposal_id, *voter))
    }

    pub fn is_revealed(&self, proposal_id: u64, voter: &NodeId) -> bool {
        self.reveals.contains_key(&(proposal_id, *voter))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_three_voters() -> (ZkBlindedVoting, NodeId, NodeId, NodeId) {
        let mut v = ZkBlindedVoting::new();
        let a = NodeId::from_label("alice");
        let b = NodeId::from_label("bob");
        let c = NodeId::from_label("carol");
        v.set_eligible(a, true);
        v.set_eligible(b, true);
        v.set_eligible(c, true);
        v.open_proposal(1);
        (v, a, b, c)
    }

    #[test]
    fn commit_reveal_and_tally_majority() {
        let (mut v, a, b, c) = setup_three_voters();

        let ballots: [(NodeId, BallotChoice, &[u8], &[u8]); 3] = [
            (a, BallotChoice::Yes, b"n1", b"blind-a"),
            (b, BallotChoice::Yes, b"n2", b"blind-b"),
            (c, BallotChoice::No, b"n3", b"blind-c"),
        ];

        for (voter, choice, nonce, blind) in ballots {
            let ballot = ZkBlindedVoting::prepare_ballot(voter, 1, choice, nonce, blind);
            v.commit(ballot).unwrap();
        }
        assert_eq!(v.commitment_count(1), 3);

        // Before reveal, tally is empty
        assert_eq!(v.tally(1), VoteTally::default());

        for (voter, choice, nonce, blind) in ballots {
            v.reveal(voter, 1, choice, nonce, blind).unwrap();
        }

        let t = v.tally(1);
        assert_eq!(
            t,
            VoteTally {
                yes: 2,
                no: 1,
                abstain: 0,
                revealed: 3,
            }
        );
        assert!(t.majority_yes());
        assert_eq!(t.total_cast(), 3);
    }

    #[test]
    fn invalid_reveal_rejected() {
        let (mut v, a, _, _) = setup_three_voters();
        let ballot =
            ZkBlindedVoting::prepare_ballot(a, 1, BallotChoice::Yes, b"nonce", b"blind");
        v.commit(ballot).unwrap();

        // Wrong choice
        assert_eq!(
            v.reveal(a, 1, BallotChoice::No, b"nonce", b"blind"),
            Err(VotingError::InvalidReveal)
        );
        // Wrong nonce
        assert_eq!(
            v.reveal(a, 1, BallotChoice::Yes, b"wrong", b"blind"),
            Err(VotingError::InvalidReveal)
        );
        // Wrong blinding
        assert_eq!(
            v.reveal(a, 1, BallotChoice::Yes, b"nonce", b"wrong"),
            Err(VotingError::InvalidReveal)
        );
    }

    #[test]
    fn ineligible_cannot_commit() {
        let mut v = ZkBlindedVoting::new();
        v.open_proposal(7);
        let stranger = NodeId::from_label("stranger");
        let ballot =
            ZkBlindedVoting::prepare_ballot(stranger, 7, BallotChoice::Yes, b"n", b"b");
        assert_eq!(v.commit(ballot), Err(VotingError::NotEligible));
    }

    #[test]
    fn double_commit_rejected() {
        let (mut v, a, _, _) = setup_three_voters();
        let b1 = ZkBlindedVoting::prepare_ballot(a, 1, BallotChoice::Yes, b"n", b"b");
        v.commit(b1).unwrap();
        let b2 = ZkBlindedVoting::prepare_ballot(a, 1, BallotChoice::No, b"n2", b"b2");
        assert_eq!(v.commit(b2), Err(VotingError::AlreadyCommitted));
    }

    #[test]
    fn blinding_proof_stub_binds_commitment_and_voter() {
        let voter = NodeId::from_label("v");
        let c = ZkBlindedVoting::make_commitment(1, &voter, BallotChoice::Yes, b"n", b"b");
        let proof = BlindingProof::stub(&c, &voter);
        assert!(proof.verify(&c, &voter));
        let other = NodeId::from_label("other");
        assert!(!proof.verify(&c, &other));
        // Tampered commitment
        let mut c2 = c;
        c2[0] ^= 1;
        assert!(!proof.verify(&c2, &voter));
    }

    #[test]
    fn closed_proposal_rejects_commit() {
        let mut v = ZkBlindedVoting::new();
        let a = NodeId::from_label("alice");
        v.set_eligible(a, true);
        v.open_proposal(1);
        v.close_proposal(1);
        let ballot =
            ZkBlindedVoting::prepare_ballot(a, 1, BallotChoice::Abstain, b"n", b"b");
        assert_eq!(v.commit(ballot), Err(VotingError::VotingClosed));
    }

    #[test]
    fn commitment_is_deterministic_and_sensitive() {
        let voter = NodeId::from_label("alice");
        let c1 = ZkBlindedVoting::make_commitment(1, &voter, BallotChoice::Yes, b"n", b"b");
        let c2 = ZkBlindedVoting::make_commitment(1, &voter, BallotChoice::Yes, b"n", b"b");
        assert_eq!(c1, c2);

        // Different inputs → different commits
        assert_ne!(
            c1,
            ZkBlindedVoting::make_commitment(2, &voter, BallotChoice::Yes, b"n", b"b")
        );
        assert_ne!(
            c1,
            ZkBlindedVoting::make_commitment(1, &voter, BallotChoice::No, b"n", b"b")
        );
        assert_ne!(
            c1,
            ZkBlindedVoting::make_commitment(1, &voter, BallotChoice::Yes, b"nX", b"b")
        );
        assert_ne!(
            c1,
            ZkBlindedVoting::make_commitment(1, &voter, BallotChoice::Yes, b"n", b"bX")
        );
        let other = NodeId::from_label("bob");
        assert_ne!(
            c1,
            ZkBlindedVoting::make_commitment(1, &other, BallotChoice::Yes, b"n", b"b")
        );
    }

    #[test]
    fn length_prefix_prevents_nonce_blinding_ambiguity() {
        let voter = NodeId::from_label("alice");
        // Without length prefixes, ("ab","c") and ("a","bc") can collide.
        // With prefixes they must differ.
        let c1 = ZkBlindedVoting::make_commitment(1, &voter, BallotChoice::Yes, b"ab", b"c");
        let c2 = ZkBlindedVoting::make_commitment(1, &voter, BallotChoice::Yes, b"a", b"bc");
        assert_ne!(c1, c2);
    }

    #[test]
    fn invalid_proof_rejected_on_commit() {
        let (mut v, a, _, _) = setup_three_voters();
        let mut ballot =
            ZkBlindedVoting::prepare_ballot(a, 1, BallotChoice::Yes, b"n", b"b");
        ballot.proof.proof_digest[0] ^= 0xff;
        assert_eq!(v.commit(ballot), Err(VotingError::InvalidProof));
    }

    #[test]
    fn double_reveal_rejected() {
        let (mut v, a, _, _) = setup_three_voters();
        let ballot =
            ZkBlindedVoting::prepare_ballot(a, 1, BallotChoice::Yes, b"n", b"b");
        v.commit(ballot).unwrap();
        v.reveal(a, 1, BallotChoice::Yes, b"n", b"b").unwrap();
        assert!(v.is_revealed(1, &a));
        assert_eq!(
            v.reveal(a, 1, BallotChoice::Yes, b"n", b"b"),
            Err(VotingError::AlreadyRevealed)
        );
    }

    #[test]
    fn reveal_without_commit_fails() {
        let (mut v, a, _, _) = setup_three_voters();
        assert_eq!(
            v.reveal(a, 1, BallotChoice::Yes, b"n", b"b"),
            Err(VotingError::NoCommitment)
        );
    }

    #[test]
    fn prepare_ballot_proof_matches_commitment() {
        let voter = NodeId::from_label("edge1");
        let ballot =
            ZkBlindedVoting::prepare_ballot(voter, 42, BallotChoice::Abstain, b"nonce", b"blind");
        assert!(ballot.proof.verify(&ballot.commitment, &ballot.voter));
        let expected = ZkBlindedVoting::make_commitment(
            42,
            &voter,
            BallotChoice::Abstain,
            b"nonce",
            b"blind",
        );
        assert_eq!(ballot.commitment, expected);
    }

    #[test]
    fn ballot_choice_byte_roundtrip() {
        for c in [BallotChoice::Yes, BallotChoice::No, BallotChoice::Abstain] {
            assert_eq!(BallotChoice::from_byte(c.as_byte()), Some(c));
        }
        assert_eq!(BallotChoice::from_byte(0), None);
        assert_eq!(BallotChoice::from_byte(4), None);
    }

    #[test]
    fn blinding_proof_digest_is_deterministic() {
        let voter = NodeId::from_label("v");
        let c = [7u8; 32];
        assert_eq!(BlindingProof::digest(&c, &voter), BlindingProof::digest(&c, &voter));
        let p1 = BlindingProof::stub(&c, &voter);
        let p2 = BlindingProof::stub(&c, &voter);
        assert_eq!(p1, p2);
    }
}
