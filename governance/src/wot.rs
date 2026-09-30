//! Web of Trust (WoT) voucher graph for MossyMesh onboarding.
//!
//! New nodes require a voucher who locks quadratic staking collateral.
//! If an invitee is marked malicious, the voucher is financially slashed.
//!
//! Consent model (fixes #63): collateral is locked only when `onboard` is
//! presented with a [`VoucherConsent`] carrying the voucher's ed25519
//! signature over (voucher, invitee, power_units, nonce). The signature is
//! verified against the voucher's registered verifying key and each nonce is
//! consumed once, so a third party cannot lock a victim's stake by naming
//! them as voucher, and a consent cannot be replayed.

use std::collections::{HashMap, HashSet};

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
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

/// Voucher-issued onboarding consent.
///
/// Issued on the voucher's node (holder of the signing key); verified by
/// `onboard` against the voucher's registered verifying key. The signature
/// covers a domain tag plus (voucher, invitee, power_units, nonce), binding
/// the consent to one invitee and one power level.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VoucherConsent {
    pub voucher: NodeId,
    pub invitee: NodeId,
    pub power_units: u64,
    pub nonce: [u8; 32],
    pub signature: [u8; 64],
}

impl VoucherConsent {
    fn message_bytes(&self) -> Vec<u8> {
        let mut m = Vec::with_capacity(24 + 32 + 32 + 8 + 32);
        m.extend_from_slice(b"mossymesh/wot-vouch/v1");
        m.extend_from_slice(&self.voucher.0);
        m.extend_from_slice(&self.invitee.0);
        m.extend_from_slice(&self.power_units.to_le_bytes());
        m.extend_from_slice(&self.nonce);
        m
    }

    /// Issue a consent. Call on the voucher's node, which holds the signing key.
    pub fn issue(
        signing_key: &SigningKey,
        voucher: NodeId,
        invitee: NodeId,
        power_units: u64,
        nonce: [u8; 32],
    ) -> Self {
        let mut consent = Self {
            voucher,
            invitee,
            power_units,
            nonce,
            signature: [0u8; 64],
        };
        let sig: Signature = signing_key.sign(&consent.message_bytes());
        consent.signature.copy_from_slice(&sig.to_bytes());
        consent
    }

    /// Verify the consent against the voucher's registered verifying key.
    pub fn verify(&self, verifying_key: &VerifyingKey) -> bool {
        Signature::try_from(self.signature.as_slice())
            .map(|sig| verifying_key.verify(&self.message_bytes(), &sig).is_ok())
            .unwrap_or(false)
    }
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
    /// Consent signature did not verify against the voucher's registered key.
    InvalidVoucherSignature,
    /// Consent nonce was already consumed (replay attempt).
    ConsentReplayed,
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
    /// ed25519 verifying keys members vouch with.
    voucher_keys: HashMap<NodeId, VerifyingKey>,
    /// Consumed consent nonces (replay protection).
    consumed_consents: HashSet<[u8; 32]>,
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

    /// Register the ed25519 verifying key a member vouches with.
    pub fn register_voucher_key(&mut self, node: NodeId, key: VerifyingKey) {
        self.voucher_keys.insert(node, key);
    }

    pub fn voucher_key(&self, node: &NodeId) -> Option<&VerifyingKey> {
        self.voucher_keys.get(node)
    }

    /// Onboard the invitee named in a voucher-issued [`VoucherConsent`],
    /// locking quadratic collateral for the consented power units.
    ///
    /// Collateral moves only after the consent signature verifies against the
    /// voucher's registered key and the nonce is fresh, so a third party
    /// cannot lock a victim's stake by naming them as voucher (fixes #63).
    pub fn onboard(&mut self, consent: &VoucherConsent) -> Result<CollateralLock, WotError> {
        let voucher = consent.voucher;
        let invitee = consent.invitee;
        let power_units = consent.power_units;

        if voucher == invitee {
            return Err(WotError::InviteeIsSelf);
        }
        if !self.nodes.contains(&voucher) {
            return Err(WotError::UnknownVoucher);
        }
        let key = self
            .voucher_keys
            .get(&voucher)
            .ok_or(WotError::UnknownVoucher)?;
        if !consent.verify(key) {
            return Err(WotError::InvalidVoucherSignature);
        }
        if self.malicious.contains(&voucher) {
            return Err(WotError::MaliciousInvitee);
        }
        if self.nodes.contains(&invitee) {
            return Err(WotError::AlreadyOnboarded);
        }
        if !self.consumed_consents.insert(consent.nonce) {
            return Err(WotError::ConsentReplayed);
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

    /// Graph with a genesis root that has a registered voucher key.
    fn vouching_graph() -> (WotGraph, SigningKey, NodeId) {
        let mut g = WotGraph::new();
        let root = NodeId::from_label("genesis");
        let signing_key = SigningKey::from_bytes(&[7u8; 32]);
        g.add_genesis(root);
        g.register_voucher_key(root, signing_key.verifying_key());
        (g, signing_key, root)
    }

    fn consent(
        sk: &SigningKey,
        voucher: NodeId,
        invitee: NodeId,
        power: u64,
        nonce_byte: u8,
    ) -> VoucherConsent {
        VoucherConsent::issue(sk, voucher, invitee, power, [nonce_byte; 32])
    }

    #[test]
    fn onboard_requires_consent_and_locks_collateral() {
        let (mut g, sk, root) = vouching_graph();
        let alice = NodeId::from_label("alice");

        let lock = g.onboard(&consent(&sk, root, alice, 3, 1)).expect("onboard");
        assert_eq!(lock.collateral, quadratic_cost(3));
        assert!(g.is_onboarded(&alice));
        assert_eq!(g.staking.locked_collateral(&root), quadratic_cost(3));
    }

    #[test]
    fn onboard_without_consent_is_impossible() {
        // The old call shape `onboard(voucher, invitee, power)` no longer
        // exists: there is no API that locks collateral without a signed
        // consent. A consent signed by anyone other than the voucher fails.
        let (mut g, _sk, root) = vouching_graph();
        let alice = NodeId::from_label("alice");
        let attacker_sk = SigningKey::from_bytes(&[9u8; 32]);
        let forged = consent(&attacker_sk, root, alice, 3, 2);
        assert_eq!(
            g.onboard(&forged),
            Err(WotError::InvalidVoucherSignature)
        );
        assert!(!g.is_onboarded(&alice));
        assert_eq!(g.staking.locked_collateral(&root), 0);
    }

    #[test]
    fn tampered_consent_rejected() {
        let (mut g, sk, root) = vouching_graph();
        let bob = NodeId::from_label("bob");

        // Signature bit flip.
        let mut bad_sig = consent(&sk, root, bob, 4, 3);
        bad_sig.signature[0] ^= 0xff;
        assert_eq!(
            g.onboard(&bad_sig),
            Err(WotError::InvalidVoucherSignature)
        );

        // Power units changed after signing.
        let mut bad_power = consent(&sk, root, bob, 4, 4);
        bad_power.power_units = 40;
        assert_eq!(
            g.onboard(&bad_power),
            Err(WotError::InvalidVoucherSignature)
        );
        assert!(!g.is_onboarded(&bob));
    }

    #[test]
    fn consent_cannot_be_replayed() {
        let (mut g, sk, root) = vouching_graph();
        let carol = NodeId::from_label("carol");
        let c = consent(&sk, root, carol, 2, 5);
        g.onboard(&c).expect("first onboard");
        assert_eq!(g.onboard(&c), Err(WotError::ConsentReplayed));
    }

    #[test]
    fn unknown_voucher_rejected() {
        let (mut g, sk, root) = vouching_graph();
        let stranger = NodeId::from_label("stranger");
        let n = NodeId::from_label("n");
        // Stranger never registered a voucher key and is not onboarded.
        let c = consent(&sk, stranger, n, 1, 6);
        assert_eq!(g.onboard(&c), Err(WotError::UnknownVoucher));
        let _ = root;
    }

    #[test]
    fn slash_voucher_when_invitee_malicious() {
        let (mut g, sk, root) = vouching_graph();
        let bob = NodeId::from_label("bob");
        g.onboard(&consent(&sk, root, bob, 4, 7)).unwrap();

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
        let (mut g, sk, root) = vouching_graph();
        let bad = NodeId::from_label("bad");
        let victim = NodeId::from_label("victim");
        g.onboard(&consent(&sk, root, bad, 2, 8)).unwrap();
        g.mark_malicious_and_slash(bad).unwrap();

        // bad is malicious; even with a root-signed consent naming bad as
        // voucher, onboarding is refused. Register bad's key first so the
        // failure is MaliciousInvitee, not UnknownVoucher.
        let bad_sk = SigningKey::from_bytes(&[8u8; 32]);
        g.register_voucher_key(bad, bad_sk.verifying_key());
        let c = consent(&bad_sk, bad, victim, 1, 9);
        assert_eq!(g.onboard(&c), Err(WotError::MaliciousInvitee));
    }
}
