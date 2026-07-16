//! Offline Wi-Fi Direct domain discovery and connection handling.

#[derive(Debug, PartialEq, Clone)]
pub enum WifiState {
    Scanning,
    GroupOwner,
    Client,
    Disconnected,
}

pub struct WifiDirectManager {
    pub state: WifiState,
    pub battery_weight: u32,
    pub peers_in_range: Vec<(String, u32)>, // (PeerId, BatteryWeight)
}

impl WifiDirectManager {
    pub fn new(battery_weight: u32) -> Self {
        WifiDirectManager {
            state: WifiState::Disconnected,
            battery_weight,
            peers_in_range: Vec::new(),
        }
    }

    /// Autonomous negotiation to become the Group Owner (Access Point)
    /// based on the deterministic battery-curve weighting.
    pub fn negotiate_group_owner(&mut self) {
        if self.peers_in_range.is_empty() {
            // Alone, default to AP to catch stragglers
            self.state = WifiState::GroupOwner;
            return;
        }

        // Find peer with highest capacity
        let mut max_weight = self.battery_weight;
        let mut i_am_leader = true;

        for peer in &self.peers_in_range {
            if peer.1 > max_weight {
                max_weight = peer.1;
                i_am_leader = false;
            }
        }

        if i_am_leader {
            self.state = WifiState::GroupOwner;
        } else {
            self.state = WifiState::Client;
        }
    }
}

pub fn init_wifi_direct() {
    println!("Initializing Offline Wi-Fi Direct local domain mesh topology.");
    let mut manager = WifiDirectManager::new(850); // High battery weight
    manager.peers_in_range.push(("peer_low_batt".to_string(), 100));
    manager.negotiate_group_owner();
    println!("Negotiated State: {:?}", manager.state);
}
