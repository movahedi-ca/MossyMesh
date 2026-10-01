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

/// Max epochs accepted in a single accrue call (~30 days of hourly ticks).
/// Larger self-reported values are rejected, not saturated (issue #184).
pub const MAX_EPOCHS_PER_CALL: u64 = 24 * 30;

/// Hard ceiling on lifetime offline epochs per node (~1 year of hourly
/// ticks). The effective per-node cap is the smaller of this and the
/// wall-clock hours elapsed since the node registered (issue #184).
pub const MAX_EPOCHS_PER_NODE: u64 = 24 * 365;

/// Grace hours added to the wall-clock lifetime cap. A node's registration
/// record is created when the miner first learns about it, which can lag the
/// node's actual genesis participation (restarts, re-registration, or an
/// offline node registering after the fact), so up to a day of pre-record
/// offline time stays claimable. The lifetime take stays bounded by elapsed
/// time plus this constant; it does not reopen the unbounded mint.
pub const PRE_REGISTRATION_GRACE_HOURS: u64 = 24;

/// Wall-clock time as unix seconds. Used only to bound epoch claims by
/// elapsed real time; never for consensus-critical ordering.
fn unix_now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

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
    /// Unix timestamp (seconds) when the node registered. Bounds the
    /// lifetime offline-epoch claim by elapsed wall-clock time (issue #184).
    pub registered_at_secs: u64,
}

impl MiningAccount {
    pub fn new_genesis(node_id: impl Into<String>) -> Self {
        Self {
            node_id: node_id.into(),
            is_genesis: true,
            points: 0,
            offline_epochs: 0,
            claimed_tokens: 0,
            registered_at_secs: unix_now_secs(),
        }
    }

    pub fn new_standard(node_id: impl Into<String>) -> Self {
        Self {
            node_id: node_id.into(),
            is_genesis: false,
            points: 0,
            offline_epochs: 0,
            claimed_tokens: 0,
            registered_at_secs: unix_now_secs(),
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
    /// The self-reported epoch count exceeded the per-call cap or the
    /// node's wall-clock-bounded lifetime cap (issue #184).
    EpochCapExceeded,
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
            LiquidityError::EpochCapExceeded => {
                write!(f, "offline epoch count exceeds the allowed cap")
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

    /// Register a genesis node with an explicit registration timestamp
    /// (unix seconds). Used for nodes re-registering with a known genesis
    /// time and by tests to simulate long-lived registrations.
    pub fn register_genesis_at(&mut self, node_id: impl Into<String>, registered_at_secs: u64) {
        let id = node_id.into();
        self.accounts.entry(id.clone()).or_insert_with(|| {
            let mut acct = MiningAccount::new_genesis(id);
            acct.registered_at_secs = registered_at_secs;
            acct
        });
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
    ///
    /// Epoch counts are capped (issue #184): a per-call ceiling rejects
    /// absurd self-reported values outright, and a per-node lifetime ceiling
    /// bounded by wall-clock hours since registration rejects claims for
    /// offline time that cannot have elapsed. Rejections leave all balances
    /// untouched; nothing saturates.
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
        if epochs > MAX_EPOCHS_PER_CALL {
            return Err(LiquidityError::EpochCapExceeded);
        }
        // Only offline islands earn retroactive points.
        if self.internet_reconnected {
            return Ok(0);
        }
        let elapsed_hours = unix_now_secs().saturating_sub(acct.registered_at_secs) / 3600;
        let max_node_epochs = MAX_EPOCHS_PER_NODE.min(
            elapsed_hours
                .saturating_add(1)
                .saturating_add(PRE_REGISTRATION_GRACE_HOURS),
        );
        if acct.offline_epochs.saturating_add(epochs) > max_node_epochs {
            return Err(LiquidityError::EpochCapExceeded);
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
        // Issue #187: serialize, never hand-format. A hand-built string
        // cannot escape node-controlled values safely.
        serde_json::json!({
            "internet_reconnected": self.internet_reconnected,
            "genesis_nodes": genesis,
            "total_points_issued": self.total_points_issued,
            "total_tokens_airdropped": self.total_tokens_airdropped,
            "points_per_epoch": POINTS_PER_OFFLINE_EPOCH,
            "tokens_per_point": TOKENS_PER_POINT,
        })
        .to_string()
    }

    pub fn account_json(&self, node_id: &str) -> Result<String, LiquidityError> {
        let a = self
            .accounts
            .get(node_id)
            .ok_or(LiquidityError::UnknownNode)?;
        let unclaimed = self.unclaimed_points(node_id)?;
        // Issue #187: node_id is attacker-controlled; it must be escaped by
        // the serializer, not interpolated into a format string.
        Ok(serde_json::json!({
            "node_id": a.node_id,
            "is_genesis": a.is_genesis,
            "points": a.points,
            "offline_epochs": a.offline_epochs,
            "claimed_tokens": a.claimed_tokens,
            "unclaimed_points": unclaimed,
        })
        .to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Register a genesis node as if it joined `hours_ago` hours back, so
    /// the wall-clock lifetime cap (issue #184) does not block the accruals
    /// under test.
    fn old_genesis(miner: &mut LiquidityMiner, node_id: &str, hours_ago: u64) {
        let at = unix_now_secs().saturating_sub(hours_ago.saturating_mul(3600));
        miner.register_genesis_at(node_id, at);
    }

    #[test]
    fn genesis_offline_nodes_earn_points() {
        let mut miner = LiquidityMiner::new();
        old_genesis(&mut miner, "pi-zero-1", 24);
        let gained = miner.accrue_offline_epochs("pi-zero-1", 3).unwrap();
        assert_eq!(gained, 300);
        assert_eq!(miner.get("pi-zero-1").unwrap().points, 300);
    }

    #[test]
    fn claim_requires_reconnect() {
        let mut miner = LiquidityMiner::new();
        old_genesis(&mut miner, "pi-zero-1", 24);
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
        old_genesis(&mut miner, "g1", 24);
        assert!(!miner.is_online());
        assert_eq!(miner.accrue_offline_epochs("g1", 4).unwrap(), 400);
        assert_eq!(miner.claim_airdrop("g1"), Err(LiquidityError::StillOffline));

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
        old_genesis(&mut miner, "n", 24);
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
        old_genesis(&mut miner, "a", 24);
        old_genesis(&mut miner, "b", 24);
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
        assert!(miner.total_tokens_airdropped <= TOKENS_PER_POINT * miner.total_points_issued);
    }

    // --- Issue #184 regression tests ---

    #[test]
    fn u64_max_epochs_rejected_not_saturated() {
        // The unbounded-mint attack: epochs = u64::MAX must be rejected,
        // leaving every balance untouched.
        let mut miner = LiquidityMiner::new();
        old_genesis(&mut miner, "attacker", 24 * 365);
        assert_eq!(
            miner.accrue_offline_epochs("attacker", u64::MAX),
            Err(LiquidityError::EpochCapExceeded)
        );
        let acct = miner.get("attacker").unwrap();
        assert_eq!(acct.points, 0);
        assert_eq!(acct.offline_epochs, 0);
        assert_eq!(miner.total_points_issued, 0);
    }

    #[test]
    fn per_call_cap_boundary() {
        let mut miner = LiquidityMiner::new();
        old_genesis(&mut miner, "patient", 24 * 31);
        // Exactly at the cap: accepted.
        let gained = miner
            .accrue_offline_epochs("patient", MAX_EPOCHS_PER_CALL)
            .unwrap();
        assert_eq!(gained, MAX_EPOCHS_PER_CALL * POINTS_PER_OFFLINE_EPOCH);
        // One over: rejected.
        assert_eq!(
            miner.accrue_offline_epochs("patient", MAX_EPOCHS_PER_CALL + 1),
            Err(LiquidityError::EpochCapExceeded)
        );
    }

    #[test]
    fn per_node_lifetime_cap_bounded_by_wall_clock() {
        // Fresh registration: at most 25 epochs claimable (0 elapsed + 1 +
        // 24h pre-registration grace).
        let mut miner = LiquidityMiner::new();
        miner.register_genesis("newbie");
        miner.accrue_offline_epochs("newbie", 20).unwrap();
        // 20 + 10 = 30 > 25: rejected, balances untouched.
        assert_eq!(
            miner.accrue_offline_epochs("newbie", 10),
            Err(LiquidityError::EpochCapExceeded)
        );
        assert_eq!(miner.get("newbie").unwrap().offline_epochs, 20);
        assert_eq!(miner.get("newbie").unwrap().points, 2000);
        // The remaining 5 epochs fit exactly.
        miner.accrue_offline_epochs("newbie", 5).unwrap();
        assert_eq!(miner.get("newbie").unwrap().offline_epochs, 25);
    }

    #[test]
    fn per_call_cap_rejects_even_while_online() {
        // Input validation runs before the online no-op path.
        let mut miner = LiquidityMiner::new();
        miner.register_genesis("n1");
        miner.on_internet_reconnect();
        assert_eq!(
            miner.accrue_offline_epochs("n1", u64::MAX),
            Err(LiquidityError::EpochCapExceeded)
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

    /// Issue #187: a node id containing JSON metacharacters must round-trip
    /// through account_json instead of breaking out of the string.
    #[test]
    fn account_json_escapes_hostile_node_id() {
        let mut miner = LiquidityMiner::new();
        let evil = "node-\"quoted\"-\\-back\ttab";
        miner.register_genesis(evil);
        let json = miner.account_json(evil).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).expect("must be valid JSON");
        assert_eq!(v["node_id"], evil);
    }
}
