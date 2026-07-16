//! Bluetooth Low Energy mesh: link-state advertisements and neighbor table.
//!
//! Simulation-capable control plane for ultra-low-power proximity links.
//! Peers exchange compact LSAs; each node maintains a TTL-pruned neighbor table.

use std::collections::BTreeMap;

/// Default LSA TTL in simulated milliseconds.
pub const DEFAULT_LSA_TTL_MS: u64 = 30_000;

/// Default periodic advertisement interval (ms).
pub const DEFAULT_ADV_INTERVAL_MS: u64 = 2_000;

/// Maximum neighbors retained (RAM ceiling friendly).
pub const MAX_NEIGHBORS: usize = 32;

/// Compact BLE advertising beacon (discovery).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BleBeacon {
    pub node_id: String,
    pub battery_level: u8,
}

impl BleBeacon {
    /// Simulates the periodic BLE advertising loop used to discover sleeping offline nodes.
    pub fn broadcast_loop(&self) {
        println!(
            "BLE Broadcast: Advertising PeerID {} | Battery: {}%",
            self.node_id, self.battery_level
        );
    }
}

/// Link-state advertisement flooded over BLE mesh.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkStateAdvertisement {
    pub origin_id: String,
    pub sequence: u32,
    pub battery_level: u8,
    /// Reported one-hop neighbors of the origin (id → link cost).
    pub neighbors: Vec<(String, u32)>,
    /// Simulated wall-clock when this LSA was generated (ms).
    pub generated_at_ms: u64,
    pub ttl_ms: u64,
}

impl LinkStateAdvertisement {
    pub fn new(
        origin_id: impl Into<String>,
        sequence: u32,
        battery_level: u8,
        neighbors: Vec<(String, u32)>,
        generated_at_ms: u64,
    ) -> Self {
        Self {
            origin_id: origin_id.into(),
            sequence,
            battery_level,
            neighbors,
            generated_at_ms,
            ttl_ms: DEFAULT_LSA_TTL_MS,
        }
    }

    pub fn is_expired(&self, now_ms: u64) -> bool {
        now_ms.saturating_sub(self.generated_at_ms) > self.ttl_ms
    }

    /// Deterministic binary encoding for sim/tests (length-prefixed UTF-8 ids).
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        encode_str(&mut out, &self.origin_id);
        out.extend_from_slice(&self.sequence.to_be_bytes());
        out.push(self.battery_level);
        out.extend_from_slice(&self.generated_at_ms.to_be_bytes());
        out.extend_from_slice(&self.ttl_ms.to_be_bytes());
        out.extend_from_slice(&(self.neighbors.len() as u16).to_be_bytes());
        for (id, cost) in &self.neighbors {
            encode_str(&mut out, id);
            out.extend_from_slice(&cost.to_be_bytes());
        }
        out
    }

    pub fn decode(bytes: &[u8]) -> Option<Self> {
        let mut i = 0usize;
        let origin_id = decode_str(bytes, &mut i)?;
        if i + 4 + 1 + 8 + 8 + 2 > bytes.len() {
            return None;
        }
        let sequence = u32::from_be_bytes(bytes[i..i + 4].try_into().ok()?);
        i += 4;
        let battery_level = bytes[i];
        i += 1;
        let generated_at_ms = u64::from_be_bytes(bytes[i..i + 8].try_into().ok()?);
        i += 8;
        let ttl_ms = u64::from_be_bytes(bytes[i..i + 8].try_into().ok()?);
        i += 8;
        let n = u16::from_be_bytes(bytes[i..i + 2].try_into().ok()?) as usize;
        i += 2;
        let mut neighbors = Vec::with_capacity(n);
        for _ in 0..n {
            let id = decode_str(bytes, &mut i)?;
            if i + 4 > bytes.len() {
                return None;
            }
            let cost = u32::from_be_bytes(bytes[i..i + 4].try_into().ok()?);
            i += 4;
            neighbors.push((id, cost));
        }
        Some(Self {
            origin_id,
            sequence,
            battery_level,
            neighbors,
            generated_at_ms,
            ttl_ms,
        })
    }
}

fn encode_str(out: &mut Vec<u8>, s: &str) {
    let b = s.as_bytes();
    let len = (b.len() as u16).to_be_bytes();
    out.extend_from_slice(&len);
    out.extend_from_slice(b);
}

fn decode_str(bytes: &[u8], i: &mut usize) -> Option<String> {
    if *i + 2 > bytes.len() {
        return None;
    }
    let len = u16::from_be_bytes(bytes[*i..*i + 2].try_into().ok()?) as usize;
    *i += 2;
    if *i + len > bytes.len() {
        return None;
    }
    let s = std::str::from_utf8(&bytes[*i..*i + len]).ok()?.to_string();
    *i += len;
    Some(s)
}

/// One entry in the local BLE neighbor table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NeighborEntry {
    pub node_id: String,
    pub battery_level: u8,
    /// Higher is better (0–255, from RSSI mapping or sim).
    pub link_quality: u8,
    pub last_seen_ms: u64,
    pub last_seq: u32,
    /// Path cost to use this neighbor as next hop (lower is better).
    pub cost: u32,
}

impl NeighborEntry {
    pub fn is_stale(&self, now_ms: u64, ttl_ms: u64) -> bool {
        now_ms.saturating_sub(self.last_seen_ms) > ttl_ms
    }
}

/// Local BLE mesh control-plane state.
#[derive(Debug, Clone)]
pub struct BleMeshNode {
    pub node_id: String,
    pub battery_level: u8,
    pub seq: u32,
    pub now_ms: u64,
    pub neighbor_ttl_ms: u64,
    /// Ordered map for deterministic iteration.
    neighbors: BTreeMap<String, NeighborEntry>,
    /// Highest LSA sequence seen per origin (loop / replay suppression).
    seen_seq: BTreeMap<String, u32>,
}

impl BleMeshNode {
    pub fn new(node_id: impl Into<String>, battery_level: u8) -> Self {
        Self {
            node_id: node_id.into(),
            battery_level,
            seq: 0,
            now_ms: 0,
            neighbor_ttl_ms: DEFAULT_LSA_TTL_MS,
            neighbors: BTreeMap::new(),
            seen_seq: BTreeMap::new(),
        }
    }

    pub fn advance_time(&mut self, delta_ms: u64) {
        self.now_ms = self.now_ms.saturating_add(delta_ms);
        self.prune_stale();
    }

    pub fn set_time(&mut self, now_ms: u64) {
        self.now_ms = now_ms;
        self.prune_stale();
    }

    pub fn neighbor_count(&self) -> usize {
        self.neighbors.len()
    }

    pub fn get_neighbor(&self, id: &str) -> Option<&NeighborEntry> {
        self.neighbors.get(id)
    }

    pub fn neighbors(&self) -> impl Iterator<Item = &NeighborEntry> {
        self.neighbors.values()
    }

    /// Map raw RSSI (dBm, typically −100..−40) to link quality 0..255 and cost.
    pub fn rssi_to_quality_cost(rssi_dbm: i8) -> (u8, u32) {
        // Clamp to [-100, -40]
        let r = (rssi_dbm as i16).clamp(-100, -40);
        // quality: -40 → 255, -100 → 0
        let quality = (((r + 100) as u32 * 255) / 60) as u8;
        // cost: inverse of quality, min 1
        let cost = 1u32 + (255u32.saturating_sub(quality as u32)) / 8;
        (quality, cost)
    }

    /// Direct hearing of a peer (beacon or unicast).
    pub fn hear_peer(&mut self, peer_id: &str, battery_level: u8, rssi_dbm: i8) {
        let (quality, cost) = Self::rssi_to_quality_cost(rssi_dbm);
        let entry = NeighborEntry {
            node_id: peer_id.to_string(),
            battery_level,
            link_quality: quality,
            last_seen_ms: self.now_ms,
            last_seq: self
                .neighbors
                .get(peer_id)
                .map(|e| e.last_seq)
                .unwrap_or(0),
            cost,
        };
        self.insert_neighbor(entry);
    }

    fn insert_neighbor(&mut self, entry: NeighborEntry) {
        if self.neighbors.len() >= MAX_NEIGHBORS && !self.neighbors.contains_key(&entry.node_id) {
            // Evict lowest link quality (deterministic: min quality, then id).
            if let Some(evict) = self
                .neighbors
                .values()
                .min_by(|a, b| {
                    a.link_quality
                        .cmp(&b.link_quality)
                        .then_with(|| a.node_id.cmp(&b.node_id))
                })
                .map(|e| e.node_id.clone())
            {
                if entry.link_quality
                    <= self
                        .neighbors
                        .get(&evict)
                        .map(|e| e.link_quality)
                        .unwrap_or(0)
                {
                    return; // new peer worse than worst — drop
                }
                self.neighbors.remove(&evict);
            }
        }
        self.neighbors.insert(entry.node_id.clone(), entry);
    }

    /// Build the next outbound LSA for this node.
    pub fn create_lsa(&mut self) -> LinkStateAdvertisement {
        self.seq = self.seq.wrapping_add(1);
        let neighbors: Vec<(String, u32)> = self
            .neighbors
            .values()
            .map(|n| (n.node_id.clone(), n.cost))
            .collect();
        LinkStateAdvertisement::new(
            self.node_id.clone(),
            self.seq,
            self.battery_level,
            neighbors,
            self.now_ms,
        )
    }

    /// Process an inbound LSA. Returns true if the table was updated / should re-flood.
    pub fn process_lsa(&mut self, lsa: &LinkStateAdvertisement) -> bool {
        if lsa.origin_id == self.node_id {
            return false;
        }
        if lsa.is_expired(self.now_ms) {
            return false;
        }
        if let Some(&seen) = self.seen_seq.get(&lsa.origin_id) {
            if lsa.sequence <= seen {
                return false;
            }
        }
        self.seen_seq
            .insert(lsa.origin_id.clone(), lsa.sequence);

        // Treat LSA origin as a one-hop neighbor if we received it over BLE.
        // Quality is derived from battery as a weak signal proxy when RSSI unknown.
        let quality = lsa.battery_level.saturating_mul(2).min(255);
        let cost = 1u32 + (255u32.saturating_sub(quality as u32)) / 16;
        let entry = NeighborEntry {
            node_id: lsa.origin_id.clone(),
            battery_level: lsa.battery_level,
            link_quality: quality,
            last_seen_ms: self.now_ms,
            last_seq: lsa.sequence,
            cost,
        };
        self.insert_neighbor(entry);
        true
    }

    pub fn prune_stale(&mut self) {
        let now = self.now_ms;
        let ttl = self.neighbor_ttl_ms;
        self.neighbors
            .retain(|_, e| !e.is_stale(now, ttl));
    }

    /// Snapshot of neighbor ids sorted for deterministic topology export.
    pub fn neighbor_ids(&self) -> Vec<String> {
        self.neighbors.keys().cloned().collect()
    }
}

pub fn init_ble_mesh() {
    println!("Initializing BLE Mesh routing for ultra-close offline proximity.");
    let mut node = BleMeshNode::new("NodeA_BLE_MAC", 88);
    node.hear_peer("NodeB_BLE_MAC", 70, -55);
    node.hear_peer("NodeC_BLE_MAC", 40, -80);
    let lsa = node.create_lsa();
    println!(
        "BLE LSA seq={} neighbors={} encoded={}B",
        lsa.sequence,
        lsa.neighbors.len(),
        lsa.encode().len()
    );
    let beacon = BleBeacon {
        node_id: node.node_id.clone(),
        battery_level: node.battery_level,
    };
    beacon.broadcast_loop();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lsa_encode_decode_roundtrip() {
        let lsa = LinkStateAdvertisement::new(
            "peer-alpha",
            7,
            90,
            vec![("peer-beta".into(), 3), ("peer-gamma".into(), 5)],
            12_345,
        );
        let bytes = lsa.encode();
        let decoded = LinkStateAdvertisement::decode(&bytes).unwrap();
        assert_eq!(lsa, decoded);
    }

    #[test]
    fn neighbor_table_hears_and_prunes() {
        let mut node = BleMeshNode::new("A", 80);
        node.hear_peer("B", 50, -50);
        assert_eq!(node.neighbor_count(), 1);
        node.neighbor_ttl_ms = 1000;
        node.advance_time(1001);
        assert_eq!(node.neighbor_count(), 0);
    }

    #[test]
    fn process_lsa_updates_and_rejects_old_seq() {
        let mut node = BleMeshNode::new("A", 80);
        let lsa1 = LinkStateAdvertisement::new("B", 1, 60, vec![], 0);
        assert!(node.process_lsa(&lsa1));
        assert!(node.get_neighbor("B").is_some());

        let lsa_old = LinkStateAdvertisement::new("B", 1, 99, vec![], 10);
        assert!(!node.process_lsa(&lsa_old));

        let lsa2 = LinkStateAdvertisement::new("B", 2, 55, vec![("C".into(), 2)], 20);
        assert!(node.process_lsa(&lsa2));
        assert_eq!(node.get_neighbor("B").unwrap().last_seq, 2);
        assert_eq!(node.get_neighbor("B").unwrap().battery_level, 55);
    }

    #[test]
    fn create_lsa_includes_direct_neighbors() {
        let mut node = BleMeshNode::new("A", 88);
        node.hear_peer("B", 70, -55);
        node.hear_peer("C", 40, -90);
        let lsa = node.create_lsa();
        assert_eq!(lsa.sequence, 1);
        assert_eq!(lsa.neighbors.len(), 2);
        // Deterministic BTree order
        assert_eq!(lsa.neighbors[0].0, "B");
        assert_eq!(lsa.neighbors[1].0, "C");
    }

    #[test]
    fn rssi_mapping_better_signal_lower_cost() {
        let (_, cost_good) = BleMeshNode::rssi_to_quality_cost(-45);
        let (_, cost_bad) = BleMeshNode::rssi_to_quality_cost(-95);
        assert!(cost_good < cost_bad);
    }

    #[test]
    fn max_neighbors_eviction() {
        let mut node = BleMeshNode::new("A", 80);
        for i in 0..MAX_NEIGHBORS {
            // Weak peers
            node.hear_peer(&format!("n{i:02}"), 10, -95);
        }
        assert_eq!(node.neighbor_count(), MAX_NEIGHBORS);
        // Strong peer should displace a weak one
        node.hear_peer("strong", 100, -40);
        assert_eq!(node.neighbor_count(), MAX_NEIGHBORS);
        assert!(node.get_neighbor("strong").is_some());
    }
}
