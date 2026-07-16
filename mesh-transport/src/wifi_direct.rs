//! Offline Wi-Fi Direct domain discovery and connection handling.
//! DOC 1: This module handles the physical layer mesh topologies when isolated from the internet.
//!
//! Group Owner (GO) negotiation is battery-weighted with deterministic tie-breaks
//! so every peer converges on the same leader without float math or wall-clock races.

/// Hysteresis margin: a challenger must beat the current GO by this many weight points
/// before leadership flips (prevents thrashing on equal-ish batteries).
pub const GO_HYSTERESIS: u32 = 25;

/// Minimum battery weight to volunteer as GO when alone / forming a group.
pub const MIN_GO_WEIGHT: u32 = 50;

#[derive(Debug, PartialEq, Clone)]
pub enum WifiState {
    /// DOC 2: Node is actively sweeping channels for other MossyMesh peers.
    Scanning,
    /// DOC 3: Node has claimed AP leadership due to high battery/AC power.
    GroupOwner,
    /// DOC 4: Node is a battery-constrained device connected to a GroupOwner.
    Client,
    Disconnected,
}

/// Peer advertisement used during GO negotiation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WifiPeer {
    pub peer_id: String,
    pub battery_weight: u32,
    /// Optional last-known GO intent (0–15 style Wi-Fi P2P intent, derived).
    pub go_intent: u8,
}

impl WifiPeer {
    pub fn new(peer_id: impl Into<String>, battery_weight: u32) -> Self {
        let w = battery_weight;
        Self {
            peer_id: peer_id.into(),
            battery_weight: w,
            go_intent: go_intent_from_weight(w),
        }
    }
}

/// Map battery weight (0–1000) onto a Wi-Fi P2P–style GO intent (0–15).
pub fn go_intent_from_weight(weight: u32) -> u8 {
    // 1000 → 15, 0 → 0
    ((weight.min(1000) * 15) / 1000) as u8
}

/// Deterministic ranking key: higher weight wins; on ties, lexicographically
/// greater `peer_id` wins so every node elects the same GO.
pub fn rank_key(peer_id: &str, weight: u32) -> (u32, &str) {
    (weight, peer_id)
}

/// Compare two candidates; returns true if `challenger` should be preferred as GO.
pub fn challenger_wins(
    challenger_id: &str,
    challenger_w: u32,
    incumbent_id: &str,
    incumbent_w: u32,
    hysteresis: u32,
) -> bool {
    if challenger_id == incumbent_id {
        return false;
    }
    // Strict improvement by hysteresis, or equal weight with better id tie-break
    // only when weights are equal (no hysteresis on pure id order for initial elect).
    if challenger_w > incumbent_w.saturating_add(hysteresis) {
        return true;
    }
    if challenger_w == incumbent_w {
        return challenger_id > incumbent_id;
    }
    false
}

pub struct WifiDirectManager {
    pub local_id: String,
    pub state: WifiState,
    pub battery_weight: u32,
    pub peers_in_range: Vec<WifiPeer>,
    /// Peer id of the elected group owner (self or remote).
    pub group_owner_id: Option<String>,
    pub hysteresis: u32,
}

impl WifiDirectManager {
    pub fn new(battery_weight: u32) -> Self {
        Self::with_id("local", battery_weight)
    }

    pub fn with_id(local_id: impl Into<String>, battery_weight: u32) -> Self {
        WifiDirectManager {
            local_id: local_id.into(),
            state: WifiState::Disconnected,
            battery_weight,
            peers_in_range: Vec::new(),
            group_owner_id: None,
            hysteresis: GO_HYSTERESIS,
        }
    }

    pub fn local_go_intent(&self) -> u8 {
        go_intent_from_weight(self.battery_weight)
    }

    pub fn add_peer(&mut self, peer_id: impl Into<String>, battery_weight: u32) {
        let peer = WifiPeer::new(peer_id, battery_weight);
        if let Some(existing) = self
            .peers_in_range
            .iter_mut()
            .find(|p| p.peer_id == peer.peer_id)
        {
            *existing = peer;
        } else {
            self.peers_in_range.push(peer);
        }
        // Keep peers sorted for deterministic scans.
        self.peers_in_range
            .sort_by(|a, b| a.peer_id.cmp(&b.peer_id));
    }

    pub fn remove_peer(&mut self, peer_id: &str) {
        self.peers_in_range.retain(|p| p.peer_id != peer_id);
    }

    pub fn set_battery_weight(&mut self, weight: u32) {
        self.battery_weight = weight;
    }

    /// Elect the GO among local node + peers using battery weights and id tie-break.
    /// Returns the elected peer id.
    pub fn elect_group_owner(&self) -> String {
        let mut best_id = self.local_id.as_str();
        let mut best_w = self.battery_weight;

        for peer in &self.peers_in_range {
            if peer.battery_weight > best_w
                || (peer.battery_weight == best_w && peer.peer_id.as_str() > best_id)
            {
                best_w = peer.battery_weight;
                best_id = peer.peer_id.as_str();
            }
        }
        best_id.to_string()
    }

    /// Autonomous negotiation to become the Group Owner (Access Point)
    /// based on the deterministic battery-curve weighting.
    /// DOC 5: The algorithm enforces a strict hierarchy where the highest capacity node MUST become the routing bottleneck.
    pub fn negotiate_group_owner(&mut self) {
        if self.peers_in_range.is_empty() {
            // Alone: become GO only if we have enough energy to host.
            if self.battery_weight >= MIN_GO_WEIGHT {
                self.state = WifiState::GroupOwner;
                self.group_owner_id = Some(self.local_id.clone());
            } else {
                self.state = WifiState::Scanning;
                self.group_owner_id = None;
            }
            return;
        }

        let elected = self.elect_group_owner();

        // Hysteresis against thrashing when an incumbent GO already exists.
        if let Some(ref current_go) = self.group_owner_id {
            if current_go != &elected {
                let (chal_id, chal_w) = if elected == self.local_id {
                    (self.local_id.as_str(), self.battery_weight)
                } else {
                    let p = self
                        .peers_in_range
                        .iter()
                        .find(|p| p.peer_id == elected)
                        .expect("elected peer must be in range");
                    (p.peer_id.as_str(), p.battery_weight)
                };
                let (inc_id, inc_w) = if current_go == &self.local_id {
                    (self.local_id.as_str(), self.battery_weight)
                } else if let Some(p) = self.peers_in_range.iter().find(|p| &p.peer_id == current_go)
                {
                    (p.peer_id.as_str(), p.battery_weight)
                } else {
                    // Incumbent left range — accept new election.
                    self.apply_election(elected);
                    return;
                };

                if !challenger_wins(chal_id, chal_w, inc_id, inc_w, self.hysteresis) {
                    // Keep incumbent.
                    self.apply_election(current_go.clone());
                    return;
                }
            }
        }

        self.apply_election(elected);
    }

    fn apply_election(&mut self, elected: String) {
        self.group_owner_id = Some(elected.clone());
        if elected == self.local_id {
            self.state = WifiState::GroupOwner;
        } else {
            self.state = WifiState::Client;
        }
    }

    /// Whether this node is currently the group owner.
    pub fn is_group_owner(&self) -> bool {
        self.state == WifiState::GroupOwner
    }

    /// Snapshot of all candidates sorted by GO preference (best first).
    pub fn ranked_candidates(&self) -> Vec<(String, u32, u8)> {
        let mut all: Vec<(String, u32, u8)> = self
            .peers_in_range
            .iter()
            .map(|p| (p.peer_id.clone(), p.battery_weight, p.go_intent))
            .collect();
        all.push((
            self.local_id.clone(),
            self.battery_weight,
            self.local_go_intent(),
        ));
        all.sort_by(|a, b| {
            b.1.cmp(&a.1) // weight desc
                .then_with(|| b.0.cmp(&a.0)) // id desc on tie
        });
        all
    }
}

pub fn init_wifi_direct() {
    println!("Initializing Offline Wi-Fi Direct local domain mesh topology.");
    let mut manager = WifiDirectManager::with_id("node_high", 850);
    manager.add_peer("peer_low_batt", 100);
    manager.add_peer("peer_mid_batt", 400);
    manager.negotiate_group_owner();
    println!(
        "Negotiated State: {:?} GO={:?}",
        manager.state, manager.group_owner_id
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn highest_battery_becomes_go() {
        let mut m = WifiDirectManager::with_id("A", 200);
        m.add_peer("B", 900);
        m.add_peer("C", 100);
        m.negotiate_group_owner();
        assert_eq!(m.state, WifiState::Client);
        assert_eq!(m.group_owner_id.as_deref(), Some("B"));
    }

    #[test]
    fn local_wins_when_highest() {
        let mut m = WifiDirectManager::with_id("A", 850);
        m.add_peer("peer_low_batt", 100);
        m.negotiate_group_owner();
        assert_eq!(m.state, WifiState::GroupOwner);
        assert_eq!(m.group_owner_id.as_deref(), Some("A"));
    }

    #[test]
    fn equal_weight_tie_break_by_peer_id() {
        let mut a = WifiDirectManager::with_id("alice", 500);
        a.add_peer("bob", 500);
        a.negotiate_group_owner();

        let mut b = WifiDirectManager::with_id("bob", 500);
        b.add_peer("alice", 500);
        b.negotiate_group_owner();

        // Both must elect the same GO ("bob" > "alice" lexicographically).
        assert_eq!(a.group_owner_id, b.group_owner_id);
        assert_eq!(a.group_owner_id.as_deref(), Some("bob"));
        assert_eq!(a.state, WifiState::Client);
        assert_eq!(b.state, WifiState::GroupOwner);
    }

    #[test]
    fn alone_becomes_go_if_weight_ok() {
        let mut m = WifiDirectManager::with_id("solo", 100);
        m.negotiate_group_owner();
        assert_eq!(m.state, WifiState::GroupOwner);
    }

    #[test]
    fn alone_scans_if_weight_too_low() {
        let mut m = WifiDirectManager::with_id("dying", 10);
        m.negotiate_group_owner();
        assert_eq!(m.state, WifiState::Scanning);
    }

    #[test]
    fn hysteresis_prevents_thrash() {
        let mut m = WifiDirectManager::with_id("A", 500);
        m.add_peer("B", 520); // only +20, below GO_HYSTERESIS=25
        m.group_owner_id = Some("A".into());
        m.state = WifiState::GroupOwner;
        m.negotiate_group_owner();
        // B does not beat A by hysteresis → A stays GO
        assert_eq!(m.group_owner_id.as_deref(), Some("A"));
        assert_eq!(m.state, WifiState::GroupOwner);

        m.add_peer("B", 600); // +100 > hysteresis
        m.negotiate_group_owner();
        assert_eq!(m.group_owner_id.as_deref(), Some("B"));
        assert_eq!(m.state, WifiState::Client);
    }

    #[test]
    fn go_intent_scales_with_weight() {
        assert_eq!(go_intent_from_weight(0), 0);
        assert_eq!(go_intent_from_weight(1000), 15);
        assert_eq!(go_intent_from_weight(500), 7);
    }

    #[test]
    fn ranked_candidates_deterministic() {
        let mut m = WifiDirectManager::with_id("m", 300);
        m.add_peer("z", 300);
        m.add_peer("a", 900);
        let ranked = m.ranked_candidates();
        assert_eq!(ranked[0].0, "a");
        assert_eq!(ranked[0].1, 900);
    }

    #[test]
    fn remove_peer_triggers_re_elect() {
        let mut m = WifiDirectManager::with_id("A", 200);
        m.add_peer("B", 900);
        m.negotiate_group_owner();
        assert_eq!(m.group_owner_id.as_deref(), Some("B"));
        m.remove_peer("B");
        m.negotiate_group_owner();
        assert_eq!(m.group_owner_id.as_deref(), Some("A"));
        assert!(m.is_group_owner());
    }
}
