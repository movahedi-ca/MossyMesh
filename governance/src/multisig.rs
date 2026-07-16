//! Admin multi-sig with mathematically decaying authority.
//!
//! Genesis: 3-of-5 admin multi-sig. Authority weight decays linearly from 1.0
//! at day 0 to 0.0 at day 90 (`ADMIN_DECAY_DAYS`). After the window, multi-sig
//! proposals no longer pass regardless of signatures — control belongs to
//! ZK-blinded edge voting.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::wot::NodeId;
use crate::{ADMIN_DECAY_DAYS, MULTISIG_SIGNERS, MULTISIG_THRESHOLD};

/// Fixed-point scale for authority weight (weight = raw / WEIGHT_SCALE).
pub const WEIGHT_SCALE: u64 = 1_000_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MultisigError {
    NotAnAdmin,
    DuplicateSigner,
    UnknownProposal,
    AlreadyExecuted,
    InsufficientAuthority,
    ThresholdNotMet,
    WrongSignerCount,
}

/// Opaque admin signature over a proposal id (stub — full crypto later).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Signature {
    pub signer: NodeId,
    pub proposal_id: u64,
    /// Detached signature bytes (stub may be empty / deterministic).
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MultisigProposal {
    pub id: u64,
    pub description: String,
    pub executed: bool,
}

/// 3-of-5 admin multi-sig with time-decaying authority weight.
#[derive(Clone, Debug)]
pub struct AdminMultisig {
    admins: Vec<NodeId>,
    threshold: usize,
    /// Network day when the multi-sig was activated (genesis day).
    genesis_day: u64,
    proposals: HashMap<u64, MultisigProposal>,
    /// Signatures collected per proposal.
    signatures: HashMap<u64, HashSet<NodeId>>,
    next_id: u64,
}

impl AdminMultisig {
    /// Create a multi-sig with exactly [`MULTISIG_SIGNERS`] distinct admin keys.
    pub fn new(admins: Vec<NodeId>, genesis_day: u64) -> Result<Self, MultisigError> {
        if admins.len() != MULTISIG_SIGNERS {
            return Err(MultisigError::WrongSignerCount);
        }
        let unique: HashSet<_> = admins.iter().copied().collect();
        if unique.len() != MULTISIG_SIGNERS {
            return Err(MultisigError::DuplicateSigner);
        }
        Ok(Self {
            admins,
            threshold: MULTISIG_THRESHOLD,
            genesis_day,
            proposals: HashMap::new(),
            signatures: HashMap::new(),
            next_id: 1,
        })
    }

    pub fn admins(&self) -> &[NodeId] {
        &self.admins
    }

    pub fn threshold(&self) -> usize {
        self.threshold
    }

    pub fn is_admin(&self, node: &NodeId) -> bool {
        self.admins.contains(node)
    }

    /// Days elapsed since genesis at `current_day` (saturating).
    pub fn days_elapsed(&self, current_day: u64) -> u64 {
        current_day.saturating_sub(self.genesis_day)
    }

    /// Authority weight in fixed-point units: full `WEIGHT_SCALE` at day 0,
    /// linearly to 0 at day [`ADMIN_DECAY_DAYS`].
    ///
    /// `weight(t) = max(0, WEIGHT_SCALE * (DECAY_DAYS - t) / DECAY_DAYS)`
    pub fn authority_weight(&self, current_day: u64) -> u64 {
        authority_weight_at(self.days_elapsed(current_day))
    }

    /// True iff multi-sig still has non-zero authority.
    pub fn has_authority(&self, current_day: u64) -> bool {
        self.authority_weight(current_day) > 0
    }

    pub fn propose(&mut self, description: impl Into<String>) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.proposals.insert(
            id,
            MultisigProposal {
                id,
                description: description.into(),
                executed: false,
            },
        );
        self.signatures.insert(id, HashSet::new());
        id
    }

    /// Record an admin signature on a proposal (stub verification).
    pub fn sign(&mut self, sig: Signature) -> Result<(), MultisigError> {
        if !self.is_admin(&sig.signer) {
            return Err(MultisigError::NotAnAdmin);
        }
        let set = self
            .signatures
            .get_mut(&sig.proposal_id)
            .ok_or(MultisigError::UnknownProposal)?;
        if !set.insert(sig.signer) {
            return Err(MultisigError::DuplicateSigner);
        }
        Ok(())
    }

    pub fn signature_count(&self, proposal_id: u64) -> usize {
        self.signatures
            .get(&proposal_id)
            .map(|s| s.len())
            .unwrap_or(0)
    }

    /// Execute if threshold met AND multi-sig still has authority at `current_day`.
    pub fn execute(&mut self, proposal_id: u64, current_day: u64) -> Result<(), MultisigError> {
        if self.authority_weight(current_day) == 0 {
            return Err(MultisigError::InsufficientAuthority);
        }
        let count = self.signature_count(proposal_id);
        let threshold = self.threshold;
        let proposal = self
            .proposals
            .get_mut(&proposal_id)
            .ok_or(MultisigError::UnknownProposal)?;
        if proposal.executed {
            return Err(MultisigError::AlreadyExecuted);
        }
        if count < threshold {
            return Err(MultisigError::ThresholdNotMet);
        }
        proposal.executed = true;
        Ok(())
    }

    pub fn proposal(&self, id: u64) -> Option<&MultisigProposal> {
        self.proposals.get(&id)
    }
}

/// Pure decay schedule used by [`AdminMultisig::authority_weight`].
///
/// Linear: `WEIGHT_SCALE * remaining / ADMIN_DECAY_DAYS`, clamped to 0 after the window.
pub fn authority_weight_at(days_elapsed: u64) -> u64 {
    if days_elapsed >= ADMIN_DECAY_DAYS {
        return 0;
    }
    let remaining = ADMIN_DECAY_DAYS - days_elapsed;
    (WEIGHT_SCALE as u128 * remaining as u128 / ADMIN_DECAY_DAYS as u128) as u64
}

/// Fractional authority in basis points (0–10_000) for display / tests.
pub fn authority_bps(days_elapsed: u64) -> u64 {
    authority_weight_at(days_elapsed) * 10_000 / WEIGHT_SCALE
}

#[cfg(test)]
mod tests {
    use super::*;

    fn five_admins() -> Vec<NodeId> {
        (0..5)
            .map(|i| NodeId::from_label(&format!("admin{i}")))
            .collect()
    }

    #[test]
    fn decay_schedule_endpoints_and_midpoint() {
        // Day 0 of network life (elapsed 0): full weight
        assert_eq!(authority_weight_at(0), WEIGHT_SCALE);
        // Day 45: half remaining → 50%
        assert_eq!(authority_weight_at(45), WEIGHT_SCALE / 2);
        // Day 89: 1/90 remaining
        assert_eq!(
            authority_weight_at(89),
            (WEIGHT_SCALE as u128 * 1 / 90) as u64
        );
        // Day 90 and beyond: zero
        assert_eq!(authority_weight_at(90), 0);
        assert_eq!(authority_weight_at(1000), 0);

        assert_eq!(authority_bps(0), 10_000);
        assert_eq!(authority_bps(45), 5_000);
        assert_eq!(authority_bps(90), 0);
    }

    #[test]
    fn decay_is_monotone_non_increasing() {
        let mut prev = authority_weight_at(0);
        for d in 1..=ADMIN_DECAY_DAYS {
            let w = authority_weight_at(d);
            assert!(w <= prev, "day {d}: {w} > {prev}");
            prev = w;
        }
    }

    #[test]
    fn three_of_five_executes_while_authority_remains() {
        let mut ms = AdminMultisig::new(five_admins(), 0).unwrap();
        let id = ms.propose("bootstrap params");
        for i in 0..3 {
            ms.sign(Signature {
                signer: NodeId::from_label(&format!("admin{i}")),
                proposal_id: id,
                bytes: vec![],
            })
            .unwrap();
        }
        // Day 10 still has authority
        ms.execute(id, 10).expect("execute");
        assert!(ms.proposal(id).unwrap().executed);
    }

    #[test]
    fn execution_fails_after_full_decay() {
        let mut ms = AdminMultisig::new(five_admins(), 0).unwrap();
        let id = ms.propose("too late");
        for i in 0..5 {
            ms.sign(Signature {
                signer: NodeId::from_label(&format!("admin{i}")),
                proposal_id: id,
                bytes: vec![],
            })
            .unwrap();
        }
        assert_eq!(
            ms.execute(id, 90),
            Err(MultisigError::InsufficientAuthority)
        );
        assert_eq!(
            ms.execute(id, 120),
            Err(MultisigError::InsufficientAuthority)
        );
    }

    #[test]
    fn threshold_not_met() {
        let mut ms = AdminMultisig::new(five_admins(), 0).unwrap();
        let id = ms.propose("needs more sigs");
        ms.sign(Signature {
            signer: NodeId::from_label("admin0"),
            proposal_id: id,
            bytes: vec![],
        })
        .unwrap();
        ms.sign(Signature {
            signer: NodeId::from_label("admin1"),
            proposal_id: id,
            bytes: vec![],
        })
        .unwrap();
        assert_eq!(ms.execute(id, 0), Err(MultisigError::ThresholdNotMet));
    }

    #[test]
    fn non_admin_cannot_sign() {
        let mut ms = AdminMultisig::new(five_admins(), 0).unwrap();
        let id = ms.propose("x");
        assert_eq!(
            ms.sign(Signature {
                signer: NodeId::from_label("intruder"),
                proposal_id: id,
                bytes: vec![],
            }),
            Err(MultisigError::NotAnAdmin)
        );
    }
}
