//! Retroactive AMM liquidity mining for genesis offline nodes.
//!
//! Nodes that operate entirely offline during the genesis period accrue mining
//! points. On internet reconnection those points convert into airdropped
//! governance-token claims against the global AMM mock.

use std::collections::HashMap;

/// Points awarded per full offline epoch (hour-equivalent tick) for a genesis node.
pub const POINTS_PER_OFFLINE_EPOCH: u64 = 100;

/// Conversion rate: governance tokens per mining point (scale 1e6 token units).
pub const TOKENS_PER_POINT: u64 = 1_000;

/// Metadata for a genesis (or later) node participating in liquidity mining.
#[derive(Debug, Clone)]
pub struct MiningAccount {
    pub node_id: String,
    /// True if this node was part of the offline genesis cohort.
    pub is_genesis: bool,
    /// Accumulated retroactive AMM liquidity mining points.
    pub points: u64,
    /// Offline epochs observed while disconnected from upstream internet.
    pub offline_epochs: u64,
    /// Governance tokens already claimed after reconnect (scale 1e6).
    pub claimed_tokens: u64,
}

impl MiningAccount {
    pub fn new_genesis(node_id: impl Into<String>) -> Self {
        Self {
            node_id: node_id.into(),
            is_genesis: true,
            points: 0,
            offline_epochs: 0,
            claimed_tokens: 0,
        }
    }

    pub fn new_standard(node_id: impl Into<String>) -> Self {
        Self {
            node_id: node_id.into(),
            is_genesis: false,
            points: 0,
            offline_epochs: 0,
            claimed_tokens: 0,
        }
    }
}

/// Errors for the liquidity mining program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LiquidityError {
    UnknownNode,
    NotGenesis,
    NothingToClaim,
    /// Claims are only allowed once the mesh has reconnected upstream.
    StillOffline,
}

impl std::fmt::Display for LiquidityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LiquidityError::UnknownNode => write!(f, "unknown mining node"),
            LiquidityError::NotGenesis => {
                write!(f, "retroactive points only accrue to genesis offline nodes")
            }
            LiquidityError::NothingToClaim => write!(f, "no unclaimed points"),
            LiquidityError::StillOffline => {
                write!(f, "airdrop claim requires internet reconnection")
            }
        }
    }
}

impl std::error::Error for LiquidityError {}

/// Tracks retroactive AMM liquidity mining points for offline genesis nodes.
#[derive(Debug, Default)]
pub struct LiquidityMiner {
    accounts: HashMap<String, MiningAccount>,
    /// Mirrors the gateway reconnect flag for claim eligibility.
    internet_reconnected: bool,
    /// Total points issued network-wide.
    pub total_points_issued: u64,
    /// Total governance tokens airdropped so far.
    pub total_tokens_airdropped: u64,
}

impl LiquidityMiner {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn on_internet_reconnect(&mut self) {
        self.internet_reconnected = true;
    }

    pub fn on_internet_disconnect(&mut self) {
        self.internet_reconnected = false;
    }

    pub fn is_online(&self) -> bool {
        self.internet_reconnected
    }

    /// Register (or fetch) a genesis offline node eligible for retroactive mining points.
    pub fn register_genesis(&mut self, node_id: impl Into<String>) {
        let id = node_id.into();
        self.accounts
            .entry(id.clone())
            .or_insert_with(|| MiningAccount::new_genesis(id));
    }

    /// Register a non-genesis participant (no retroactive offline points).
    pub fn register_standard(&mut self, node_id: impl Into<String>) {
        let id = node_id.into();
        self.accounts
            .entry(id.clone())
            .or_insert_with(|| MiningAccount::new_standard(id));
    }

    /// Ensure a genesis node exists and return a snapshot.
    pub fn ensure_genesis(&mut self, node_id: &str) -> MiningAccount {
        self.accounts
            .entry(node_id.to_string())
            .or_insert_with(|| MiningAccount::new_genesis(node_id))
            .clone()
    }

    pub fn get(&self, node_id: &str) -> Option<&MiningAccount> {
        self.accounts.get(node_id)
    }

    /// Accrue retroactive points for a genesis node that stayed offline for `epochs`.
    /// Non-genesis accounts may be tracked but earn zero retroactive points.
    pub fn accrue_offline_epochs(
        &mut self,
        node_id: &str,
        epochs: u64,
    ) -> Result<u64, LiquidityError> {
        let acct = self
            .accounts
            .get_mut(node_id)
            .ok_or(LiquidityError::UnknownNode)?;
        if !acct.is_genesis {
            return Err(LiquidityError::NotGenesis);
        }
        // Only offline islands earn retroactive points.
        if self.internet_reconnected {
            return Ok(0);
        }
        let gained = epochs.saturating_mul(POINTS_PER_OFFLINE_EPOCH);
        acct.offline_epochs = acct.offline_epochs.saturating_add(epochs);
        acct.points = acct.points.saturating_add(gained);
        self.total_points_issued = self.total_points_issued.saturating_add(gained);
        Ok(gained)
    }

    /// Unclaimed points for a node.
    pub fn unclaimed_points(&self, node_id: &str) -> Result<u64, LiquidityError> {
        let acct = self
            .accounts
            .get(node_id)
            .ok_or(LiquidityError::UnknownNode)?;
        let claimed_as_points = acct.claimed_tokens / TOKENS_PER_POINT;
        Ok(acct.points.saturating_sub(claimed_as_points))
    }

    /// Convert unclaimed mining points into governance-token airdrop after reconnect.
    pub fn claim_airdrop(&mut self, node_id: &str) -> Result<u64, LiquidityError> {
        if !self.internet_reconnected {
            return Err(LiquidityError::StillOffline);
        }
        let unclaimed = self.unclaimed_points(node_id)?;
        if unclaimed == 0 {
            return Err(LiquidityError::NothingToClaim);
        }
        let tokens = unclaimed.saturating_mul(TOKENS_PER_POINT);
        let acct = self
            .accounts
            .get_mut(node_id)
            .ok_or(LiquidityError::UnknownNode)?;
        acct.claimed_tokens = acct.claimed_tokens.saturating_add(tokens);
        self.total_tokens_airdropped = self.total_tokens_airdropped.saturating_add(tokens);
        Ok(tokens)
    }

    pub fn status_json(&self) -> String {
        let genesis = self.accounts.values().filter(|a| a.is_genesis).count();
        format!(
            "{{\"internet_reconnected\":{},\"genesis_nodes\":{},\"total_points_issued\":{},\"total_tokens_airdropped\":{},\"points_per_epoch\":{},\"tokens_per_point\":{}}}",
            self.internet_reconnected,
            genesis,
            self.total_points_issued,
            self.total_tokens_airdropped,
            POINTS_PER_OFFLINE_EPOCH,
            TOKENS_PER_POINT
        )
    }

    pub fn account_json(&self, node_id: &str) -> Result<String, LiquidityError> {
        let a = self
            .accounts
            .get(node_id)
            .ok_or(LiquidityError::UnknownNode)?;
        let unclaimed = self.unclaimed_points(node_id)?;
        Ok(format!(
            "{{\"node_id\":\"{}\",\"is_genesis\":{},\"points\":{},\"offline_epochs\":{},\"claimed_tokens\":{},\"unclaimed_points\":{}}}",
            a.node_id, a.is_genesis, a.points, a.offline_epochs, a.claimed_tokens, unclaimed
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn genesis_offline_nodes_earn_points() {
        let mut miner = LiquidityMiner::new();
        miner.register_genesis("pi-zero-1");
        let gained = miner.accrue_offline_epochs("pi-zero-1", 3).unwrap();
        assert_eq!(gained, 300);
        assert_eq!(miner.get("pi-zero-1").unwrap().points, 300);
    }

    #[test]
    fn claim_requires_reconnect() {
        let mut miner = LiquidityMiner::new();
        miner.register_genesis("pi-zero-1");
        miner.accrue_offline_epochs("pi-zero-1", 2).unwrap();
        assert_eq!(
            miner.claim_airdrop("pi-zero-1"),
            Err(LiquidityError::StillOffline)
        );
        miner.on_internet_reconnect();
        let tokens = miner.claim_airdrop("pi-zero-1").unwrap();
        assert_eq!(tokens, 200 * TOKENS_PER_POINT);
        assert_eq!(
            miner.claim_airdrop("pi-zero-1"),
            Err(LiquidityError::NothingToClaim)
        );
    }

    #[test]
    fn no_new_points_while_online() {
        let mut miner = LiquidityMiner::new();
        miner.register_genesis("n1");
        miner.on_internet_reconnect();
        let gained = miner.accrue_offline_epochs("n1", 5).unwrap();
        assert_eq!(gained, 0);
    }

    #[test]
    fn non_genesis_cannot_accrue() {
        let mut miner = LiquidityMiner::new();
        miner.register_standard("edge-1");
        assert_eq!(
            miner.accrue_offline_epochs("edge-1", 1),
            Err(LiquidityError::NotGenesis)
        );
    }

    // --- Formal invariants (docs/math-htlc-twamm.md §3) ---

    /// L1–L3: offline accrual, online claim only.
    #[test]
    fn invariant_l1_l3_offline_online_boundary() {
        let mut miner = LiquidityMiner::new();
        miner.register_genesis("g1");
        assert!(!miner.is_online());
        assert_eq!(miner.accrue_offline_epochs("g1", 4).unwrap(), 400);
        assert_eq!(
            miner.claim_airdrop("g1"),
            Err(LiquidityError::StillOffline)
        );

        miner.on_internet_reconnect();
        assert_eq!(miner.accrue_offline_epochs("g1", 2).unwrap(), 0);
        assert_eq!(miner.get("g1").unwrap().points, 400);

        let tokens = miner.claim_airdrop("g1").unwrap();
        assert_eq!(tokens, 400 * TOKENS_PER_POINT);
    }

    /// L4 + L5: per-account point conservation; no double claim.
    #[test]
    fn invariant_l4_l5_point_conservation_and_single_claim() {
        let mut miner = LiquidityMiner::new();
        miner.register_genesis("n");
        miner.accrue_offline_epochs("n", 5).unwrap(); // 500 points
        miner.on_internet_reconnect();

        let acct = miner.get("n").unwrap();
        let unclaimed = miner.unclaimed_points("n").unwrap();
        let claimed_as_points = acct.claimed_tokens / TOKENS_PER_POINT;
        assert_eq!(claimed_as_points + unclaimed, acct.points);

        miner.claim_airdrop("n").unwrap();
        let acct = miner.get("n").unwrap();
        let unclaimed = miner.unclaimed_points("n").unwrap();
        let claimed_as_points = acct.claimed_tokens / TOKENS_PER_POINT;
        assert_eq!(unclaimed, 0);
        assert_eq!(claimed_as_points + unclaimed, acct.points);
        assert_eq!(claimed_as_points, 500);
        assert_eq!(
            miner.claim_airdrop("n"),
            Err(LiquidityError::NothingToClaim)
        );
    }

    /// L6 + L7: network totals and token–point link.
    #[test]
    fn invariant_l6_l7_network_conservation() {
        let mut miner = LiquidityMiner::new();
        miner.register_genesis("a");
        miner.register_genesis("b");
        miner.accrue_offline_epochs("a", 2).unwrap(); // 200
        miner.accrue_offline_epochs("b", 3).unwrap(); // 300
        assert_eq!(miner.total_points_issued, 500);

        let sum_points: u64 = ["a", "b"]
            .iter()
            .map(|id| miner.get(id).unwrap().points)
            .sum();
        assert_eq!(sum_points, miner.total_points_issued);

        miner.on_internet_reconnect();
        miner.claim_airdrop("a").unwrap();
        // b leaves points unclaimed
        let sum_claimed: u64 = ["a", "b"]
            .iter()
            .map(|id| miner.get(id).map(|x| x.claimed_tokens).unwrap_or(0))
            .sum();
        assert_eq!(sum_claimed, miner.total_tokens_airdropped);
        assert_eq!(miner.total_tokens_airdropped, 200 * TOKENS_PER_POINT);
        assert!(
            miner.total_tokens_airdropped <= TOKENS_PER_POINT * miner.total_points_issued
        );
    }

    #[test]
    fn invariant_l2_non_genesis_blocked() {
        let mut miner = LiquidityMiner::new();
        miner.register_standard("std");
        assert_eq!(
            miner.accrue_offline_epochs("std", 10),
            Err(LiquidityError::NotGenesis)
        );
        assert_eq!(miner.total_points_issued, 0);
    }
}
