//! Web of Trust (WoT) voucher graph for MossyMesh onboarding.
//!
//! New nodes require a voucher who locks quadratic staking collateral.
//! If an invitee is marked malicious, the voucher is financially slashed.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::staking::{CollateralLock, QuadraticStaking, StakingError};

/// Stable node identifier (32-byte peer key material).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NodeId(pub [u8; 32]);

impl NodeId {
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        NodeId(bytes)
    }

    /// Convenience constructor for tests and demos from a short label.
    pub fn from_label(label: &str) -> Self {
        let mut bytes = [0u8; 32];
        let src = label.as_bytes();
        let n = src.len().min(32);
        bytes[..n].copy_from_slice(&src[..n]);
        NodeId(bytes)
    }
}

/// Directed voucher edge: `voucher` underwrites `invitee` with locked collateral.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoucherEdge {
    pub voucher: NodeId,
    pub invitee: NodeId,
    /// Units of voting power the voucher staked (collateral = power²).
    pub power_units: u64,
    pub slashed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WotError {
    AlreadyOnboarded,
    InviteeIsSelf,
    UnknownVoucher,
    UnknownInvitee,
    EdgeNotFound,
    AlreadySlashed,
    Staking(StakingError),
    MaliciousInvitee,
}

impl From<StakingError> for WotError {
    fn from(e: StakingError) -> Self {
        WotError::Staking(e)
    }
}

/// Web of Trust graph: genesis roots plus voucher → invitee edges.
#[derive(Clone, Debug, Default)]
pub struct WotGraph {
    /// Nodes considered part of the mesh (onboarded).
    nodes: HashSet<NodeId>,
    /// Malicious nodes (cannot onboard others; trigger slash).
    malicious: HashSet<NodeId>,
    /// Outgoing voucher edges keyed by invitee (one primary voucher per invitee).
    by_invitee: HashMap<NodeId, VoucherEdge>,
    /// All edges keyed by voucher for slash/reporting.
    by_voucher: HashMap<NodeId, Vec<NodeId>>,
    /// Shared staking ledger for collateral locks.
    pub staking: QuadraticStaking,
}

impl WotGraph {
    pub fn new() -> Self {
        Self::default()
    }

    /// Seed genesis / bootstrap nodes that need no voucher.
    pub fn add_genesis(&mut self, node: NodeId) {
        self.nodes.insert(node);
    }

    pub fn is_onboarded(&self, node: &NodeId) -> bool {
        self.nodes.contains(node)
    }

    pub fn is_malicious(&self, node: &NodeId) -> bool {
        self.malicious.contains(node)
    }

    pub fn edge_for_invitee(&self, invitee: &NodeId) -> Option<&VoucherEdge> {
        self.by_invitee.get(invitee)
    }

    pub fn invitees_of(&self, voucher: &NodeId) -> Vec<NodeId> {
        self.by_voucher
            .get(voucher)
            .cloned()
            .unwrap_or_default()
    }

    /// Onboard `invitee` under `voucher`, locking quadratic collateral for `power_units`.
    pub fn onboard(
        &mut self,
        voucher: NodeId,
        invitee: NodeId,
        power_units: u64,
    ) -> Result<CollateralLock, WotError> {
        if voucher == invitee {
            return Err(WotError::InviteeIsSelf);
        }
        if !self.nodes.contains(&voucher) {
            return Err(WotError::UnknownVoucher);
        }
        if self.malicious.contains(&voucher) {
            return Err(WotError::MaliciousInvitee);
        }
        if self.nodes.contains(&invitee) {
            return Err(WotError::AlreadyOnboarded);
        }

        let lock = self.staking.lock(voucher, power_units)?;
        let edge = VoucherEdge {
            voucher,
            invitee,
            power_units,
            slashed: false,
        };
        self.by_invitee.insert(invitee, edge);
        self.by_voucher.entry(voucher).or_default().push(invitee);
        self.nodes.insert(invitee);
        Ok(lock)
    }

    /// Mark `invitee` as malicious and slash the voucher's locked collateral for that edge.
    ///
    /// Returns the slashed collateral amount (quadratic cost of the power units).
    pub fn mark_malicious_and_slash(&mut self, invitee: NodeId) -> Result<u128, WotError> {
        if !self.nodes.contains(&invitee) && !self.by_invitee.contains_key(&invitee) {
            return Err(WotError::UnknownInvitee);
        }

        self.malicious.insert(invitee);

        let edge = self
            .by_invitee
            .get_mut(&invitee)
            .ok_or(WotError::EdgeNotFound)?;

        if edge.slashed {
            return Err(WotError::AlreadySlashed);
        }

        let voucher = edge.voucher;
        let power = edge.power_units;
        edge.slashed = true;

        let slashed = self.staking.slash(&voucher, power)?;
        Ok(slashed)
    }

    /// Total nodes currently onboarded (including genesis and malicious).
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::staking::quadratic_cost;

    #[test]
    fn onboard_requires_voucher_and_locks_collateral() {
        let mut g = WotGraph::new();
        let root = NodeId::from_label("genesis");
        let alice = NodeId::from_label("alice");
        g.add_genesis(root);

        let lock = g.onboard(root, alice, 3).expect("onboard");
        assert_eq!(lock.collateral, quadratic_cost(3));
        assert!(g.is_onboarded(&alice));
        assert_eq!(g.staking.locked_collateral(&root), quadratic_cost(3));
    }

    #[test]
    fn slash_voucher_when_invitee_malicious() {
        let mut g = WotGraph::new();
        let root = NodeId::from_label("genesis");
        let bob = NodeId::from_label("bob");
        g.add_genesis(root);
        g.onboard(root, bob, 4).unwrap();

        let expected = quadratic_cost(4);
        let slashed = g.mark_malicious_and_slash(bob).expect("slash");
        assert_eq!(slashed, expected);
        assert!(g.is_malicious(&bob));
        assert_eq!(g.staking.locked_collateral(&root), 0);
        assert_eq!(g.staking.slashed_total(&root), expected);

        let edge = g.edge_for_invitee(&bob).unwrap();
        assert!(edge.slashed);

        // Double slash rejected
        assert_eq!(
            g.mark_malicious_and_slash(bob),
            Err(WotError::AlreadySlashed)
        );
    }

    #[test]
    fn malicious_voucher_cannot_onboard() {
        let mut g = WotGraph::new();
        let root = NodeId::from_label("genesis");
        let bad = NodeId::from_label("bad");
        let victim = NodeId::from_label("victim");
        g.add_genesis(root);
        g.onboard(root, bad, 2).unwrap();
        g.mark_malicious_and_slash(bad).unwrap();

        assert_eq!(
            g.onboard(bad, victim, 1),
            Err(WotError::MaliciousInvitee)
        );
    }

    #[test]
    fn unknown_voucher_rejected() {
        let mut g = WotGraph::new();
        let stranger = NodeId::from_label("stranger");
        let n = NodeId::from_label("n");
        assert_eq!(g.onboard(stranger, n, 1), Err(WotError::UnknownVoucher));
    }
}
