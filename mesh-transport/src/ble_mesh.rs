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

/// Maximum neighbor entries an LSA may carry on the wire. Decodes with a
/// larger count are rejected before any allocation (fixes #201: a
/// wire-controlled `u16` count of 65535 must not drive `Vec::with_capacity`).
pub const MAX_LSA_NEIGHBORS: usize = 64;

/// Maximum plausible sequence jump between two accepted LSAs from one origin.
///
/// `process_lsa` drops any LSA whose sequence jumps further than this past
/// the last seen sequence, *without* updating the seen watermark. A single
/// spoofed packet with `sequence = u32::MAX` can therefore never permanently
/// suppress a victim's real LSAs (fixes #205). Genuine jumps larger than this
/// only occur after a node restart, which clears the in-memory watermark, so
/// the window does not break legitimate rejoins.
///
/// Per-LSA signatures would remove the remaining spoofing window (an attacker
/// inside the window can still ratchet the watermark with repeated packets);
/// that is documented follow-up work.
pub const MAX_LSA_SEQUENCE_JUMP: u32 = 4096;

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
    /// Hops this advertisement has already traversed. The origin emits 0;
    /// every re-flooding relay increments it (see `flood_relay_lsa`).
    /// Only a 0-hop arrival proves the origin is in radio range (fixes #198).
    pub hops_traversed: u8,
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
            hops_traversed: 0,
        }
    }

    pub fn is_expired(&self, now_ms: u64) -> bool {
        now_ms.saturating_sub(self.generated_at_ms) > self.ttl_ms
    }

    /// Deterministic binary encoding for sim/tests (length-prefixed UTF-8 ids).
    ///
    /// Wire layout: origin_id || sequence || battery || generated_at_ms ||
    /// ttl_ms || hops_traversed || neighbor_count(u16) || neighbors…
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        encode_str(&mut out, &self.origin_id);
        out.extend_from_slice(&self.sequence.to_be_bytes());
        out.push(self.battery_level);
        out.extend_from_slice(&self.generated_at_ms.to_be_bytes());
        out.extend_from_slice(&self.ttl_ms.to_be_bytes());
        out.push(self.hops_traversed);
        out.extend_from_slice(&(self.neighbors.len().min(MAX_LSA_NEIGHBORS) as u16).to_be_bytes());
        for (id, cost) in self.neighbors.iter().take(MAX_LSA_NEIGHBORS) {
            encode_str(&mut out, id);
            out.extend_from_slice(&cost.to_be_bytes());
        }
        out
    }

    pub fn decode(bytes: &[u8]) -> Option<Self> {
        let mut i = 0usize;
        let origin_id = decode_str(bytes, &mut i)?;
        if i + 4 + 1 + 8 + 8 + 1 + 2 > bytes.len() {
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
        let hops_traversed = bytes[i];
        i += 1;
        let n = u16::from_be_bytes(bytes[i..i + 2].try_into().ok()?) as usize;
        i += 2;
        // Fix #201: never pre-allocate from an unbounded wire count.
        if n > MAX_LSA_NEIGHBORS {
            return None;
        }
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
            hops_traversed,
        })
    }
}

/// Prepare an LSA for re-flooding by a relay: bump the hop count (saturating).
///
/// Receiving nodes use `hops_traversed` to tell a directly-heard origin
/// (`0`) from a multi-hop flood (`> 0`); see `BleMeshNode::process_lsa`.
pub fn flood_relay_lsa(lsa: &LinkStateAdvertisement) -> LinkStateAdvertisement {
    let mut out = lsa.clone();
    out.hops_traversed = out.hops_traversed.saturating_add(1);
    out
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
    /// Wall-clock of the newest accepted LSA per origin (wrap tie-break).
    seen_lsa_time: BTreeMap<String, u64>,
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
            seen_lsa_time: BTreeMap::new(),
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
            last_seq: self.neighbors.get(peer_id).map(|e| e.last_seq).unwrap_or(0),
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

    /// Process an inbound LSA. Returns true if the LSA was new and should re-flood.
    ///
    /// Sequence handling (fixes #205):
    /// - Jumps beyond [`MAX_LSA_SEQUENCE_JUMP`] are dropped *without* updating
    ///   the watermark, so a spoofed maxed-out sequence cannot permanently
    ///   suppress the victim's real LSAs.
    /// - `wrapping_add` self-suppression after 2^32 LSAs is handled by a
    ///   wall-clock tie-break: a sequence near 0 is accepted after a watermark
    ///   near u32::MAX only when the LSA is not older than the newest accepted
    ///   one from that origin.
    ///
    /// Neighbor handling (fixes #198): only an LSA that arrived directly from
    /// its origin (`hops_traversed == 0`) proves radio adjacency. Flooded
    /// copies (`hops_traversed > 0`) update the link-state view but never enter
    /// the one-hop neighbor table, keeping Dijkstra honest.
    pub fn process_lsa(&mut self, lsa: &LinkStateAdvertisement) -> bool {
        if lsa.origin_id == self.node_id {
            return false;
        }
        if lsa.is_expired(self.now_ms) {
            return false;
        }
        if let Some(&seen) = self.seen_seq.get(&lsa.origin_id) {
            if lsa.sequence > seen {
                if lsa.sequence - seen > MAX_LSA_SEQUENCE_JUMP {
                    return false;
                }
            } else {
                // sequence <= seen: replay, duplicate, or the origin's own
                // counter wrapped past 2^32. Accept only the wrap case.
                let near_max = u32::MAX - seen <= MAX_LSA_SEQUENCE_JUMP;
                let near_zero = lsa.sequence <= MAX_LSA_SEQUENCE_JUMP;
                let last_gen = self.seen_lsa_time.get(&lsa.origin_id).copied().unwrap_or(0);
                let not_older = lsa.generated_at_ms >= last_gen;
                if !(near_max && near_zero && not_older) {
                    return false;
                }
            }
        }
        self.seen_seq.insert(lsa.origin_id.clone(), lsa.sequence);
        self.seen_lsa_time
            .insert(lsa.origin_id.clone(), lsa.generated_at_ms);

        // Only a directly-heard origin becomes a one-hop neighbor.
        if lsa.hops_traversed == 0 {
            // Quality is derived from battery as a weak signal proxy when RSSI unknown.
            let quality = lsa.battery_level.saturating_mul(2);
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
        }
        true
    }

    pub fn prune_stale(&mut self) {
        let now = self.now_ms;
        let ttl = self.neighbor_ttl_ms;
        self.neighbors.retain(|_, e| !e.is_stale(now, ttl));
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

/// Legacy BLE ATT payload ceiling: some Android devices drop anything larger
/// than 20 bytes when acting as relays. Fixes #25.
pub const BLE_LEGACY_MTU: usize = 20;

/// Relay fragment header: message_id (u16 BE) || index (u8) || count (u8).
pub const BLE_RELAY_HEADER_LEN: usize = 4;

/// Payload bytes per relay fragment, so total on-air size stays within 20 B.
pub const BLE_RELAY_PAYLOAD_LEN: usize = BLE_LEGACY_MTU - BLE_RELAY_HEADER_LEN;

/// One BLE relay fragment small enough for legacy 20-byte relays.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BleRelayFragment {
    pub message_id: u16,
    pub index: u8,
    pub count: u8,
    pub payload: Vec<u8>,
}

impl BleRelayFragment {
    /// Encode to the exact on-air bytes (always <= 20).
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(BLE_RELAY_HEADER_LEN + self.payload.len());
        out.extend_from_slice(&self.message_id.to_be_bytes());
        out.push(self.index);
        out.push(self.count);
        out.extend_from_slice(&self.payload);
        out
    }

    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < BLE_RELAY_HEADER_LEN || bytes.len() > BLE_LEGACY_MTU {
            return None;
        }
        let mut id = [0u8; 2];
        id.copy_from_slice(&bytes[..2]);
        Some(Self {
            message_id: u16::from_be_bytes(id),
            index: bytes[2],
            count: bytes[3],
            payload: bytes[BLE_RELAY_HEADER_LEN..].to_vec(),
        })
    }
}

/// Split a payload into relay fragments, each fitting in 20 bytes on air.
pub fn fragment_ble_payload(message_id: u16, payload: &[u8]) -> Vec<BleRelayFragment> {
    if payload.is_empty() {
        return Vec::new();
    }
    let mtu = BLE_RELAY_PAYLOAD_LEN;
    let count = payload.len().div_ceil(mtu).min(u8::MAX as usize) as u8;
    (0..count)
        .map(|index| {
            let start = (index as usize) * mtu;
            let end = start.saturating_add(mtu).min(payload.len());
            BleRelayFragment {
                message_id,
                index,
                count,
                payload: payload[start..end].to_vec(),
            }
        })
        .collect()
}

/// Reassembles one BLE relay message from its fragments (any arrival order).
#[derive(Debug, Default)]
pub struct BleRelayReassembler {
    message_id: Option<u16>,
    count: u8,
    parts: Vec<Option<Vec<u8>>>,
}

impl BleRelayReassembler {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed one fragment. Returns the full payload once every part arrived.
    pub fn accept(&mut self, frag: &BleRelayFragment) -> Option<Vec<u8>> {
        if self.message_id.is_none() {
            self.message_id = Some(frag.message_id);
            self.count = frag.count;
            self.parts = vec![None; frag.count as usize];
        }
        if self.message_id != Some(frag.message_id) || frag.count != self.count {
            return None;
        }
        let idx = frag.index as usize;
        if idx >= self.parts.len() {
            return None;
        }
        self.parts[idx] = Some(frag.payload.clone());
        if self.parts.iter().all(|p| p.is_some()) {
            let mut out = Vec::new();
            for part in self.parts.drain(..).flatten() {
                out.extend_from_slice(&part);
            }
            self.message_id = None;
            Some(out)
        } else {
            None
        }
    }
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

    #[test]
    fn relay_fragments_fit_legacy_mtu() {
        let payload: Vec<u8> = (0..100).map(|i| i as u8).collect();
        let frags = fragment_ble_payload(0x1234, &payload);
        assert!(frags.len() > 1);
        for f in &frags {
            let wire = f.encode();
            assert!(
                wire.len() <= BLE_LEGACY_MTU,
                "fragment must fit 20-byte relay MTU, got {}",
                wire.len()
            );
            let dec = BleRelayFragment::decode(&wire).unwrap();
            assert_eq!(dec, *f);
        }
    }

    #[test]
    fn relay_reassembler_accepts_any_order() {
        let payload: Vec<u8> = (0..64).map(|i| (i * 3) as u8).collect();
        let mut frags = fragment_ble_payload(0xBEEF, &payload);
        frags.reverse();
        let mut reasm = BleRelayReassembler::new();
        let mut done = None;
        for f in &frags {
            if let Some(out) = reasm.accept(f) {
                done = Some(out);
            }
        }
        assert_eq!(done, Some(payload));
    }

    #[test]
    fn empty_payload_produces_no_fragments() {
        assert!(fragment_ble_payload(1, &[]).is_empty());
    }

    /// Regression test for #198: an LSA that arrived via flood relay
    /// (hops_traversed > 0) must not poison the one-hop neighbor table.
    #[test]
    fn flooded_lsa_does_not_become_neighbor() {
        let mut node = BleMeshNode::new("A", 80);

        // Directly-heard origin (0 hops): neighbor entry created.
        let direct = LinkStateAdvertisement::new("B", 1, 60, vec![], 0);
        assert_eq!(direct.hops_traversed, 0);
        assert!(node.process_lsa(&direct));
        assert!(node.get_neighbor("B").is_some());

        // Same origin re-flooded through a relay: accepted for re-flood, but
        // the origin is three relays away and must not be a direct neighbor.
        let mut relayed = LinkStateAdvertisement::new("C", 1, 60, vec![], 10);
        relayed.hops_traversed = 3;
        assert!(node.process_lsa(&relayed));
        assert!(
            node.get_neighbor("C").is_none(),
            "flooded LSA origin must not enter the neighbor table"
        );

        // 1-hop flood: still not a direct neighbor.
        let relayed_once = flood_relay_lsa(&direct);
        assert_eq!(relayed_once.hops_traversed, 1);
        let mut node2 = BleMeshNode::new("X", 80);
        assert!(node2.process_lsa(&relayed_once));
        assert!(node2.get_neighbor("B").is_none());
    }

    /// Regression test for #205: a spoofed maxed-out sequence must not
    /// permanently suppress the victim's real LSAs.
    #[test]
    fn spoofed_max_sequence_does_not_suppress_victim() {
        let mut node = BleMeshNode::new("A", 80);
        let legit = LinkStateAdvertisement::new("B", 10, 60, vec![], 0);
        assert!(node.process_lsa(&legit));

        // Attacker forges victim's LSA with sequence = u32::MAX.
        let spoof = LinkStateAdvertisement::new("B", u32::MAX, 60, vec![], 5);
        assert!(
            !node.process_lsa(&spoof),
            "jump beyond the plausible window must be dropped"
        );
        // The watermark must be untouched: the victim's next real LSA lands.
        let next = LinkStateAdvertisement::new("B", 11, 61, vec![], 10);
        assert!(node.process_lsa(&next));
        assert_eq!(node.get_neighbor("B").unwrap().battery_level, 61);

        // Small genuine jumps still pass.
        let jumpy = LinkStateAdvertisement::new("B", 11 + MAX_LSA_SEQUENCE_JUMP, 62, vec![], 20);
        assert!(node.process_lsa(&jumpy));
        // One step past the window is rejected.
        let too_far = LinkStateAdvertisement::new(
            "B",
            11 + MAX_LSA_SEQUENCE_JUMP + MAX_LSA_SEQUENCE_JUMP + 1,
            62,
            vec![],
            30,
        );
        assert!(!node.process_lsa(&too_far));
    }

    /// The origin's own counter wraps after 2^32 LSAs (wrapping_add); peers
    /// must not self-suppress the wrapped advertisements.
    #[test]
    fn sequence_wrap_accepted_via_wall_clock_tie_break() {
        let mut node = BleMeshNode::new("A", 80);
        // Simulate a long-lived mesh: watermark near u32::MAX.
        node.seen_seq.insert("B".into(), u32::MAX - 5);
        node.seen_lsa_time.insert("B".into(), 1000);

        // Wrapped sequence with newer wall-clock: accepted (not self-suppressed).
        let wrapped = LinkStateAdvertisement::new("B", 3, 60, vec![], 2000);
        assert!(node.process_lsa(&wrapped));

        // Same wrapped epoch, but an older sequence and older wall-clock: replay.
        let stale = LinkStateAdvertisement::new("B", 2, 60, vec![], 500);
        assert!(!node.process_lsa(&stale));

        // Plain replay of an old sequence without a wrap in progress: rejected.
        let mut node2 = BleMeshNode::new("A", 80);
        node2.seen_seq.insert("B".into(), 100);
        node2.seen_lsa_time.insert("B".into(), 1000);
        let replay = LinkStateAdvertisement::new("B", 50, 60, vec![], 2000);
        assert!(!node2.process_lsa(&replay));
    }

    /// Regression test for #201: a wire-controlled neighbor count of 65535
    /// must be rejected before any allocation.
    #[test]
    fn decode_rejects_huge_neighbor_count() {
        let lsa = LinkStateAdvertisement::new("peer-x", 1, 90, vec![], 0);
        let mut bytes = lsa.encode();
        // Patch the u16 neighbor count to 65535 (last two bytes of the header).
        let n = bytes.len() - 2;
        bytes[n] = 0xFF;
        bytes[n + 1] = 0xFF;
        assert!(
            LinkStateAdvertisement::decode(&bytes).is_none(),
            "count=65535 must be rejected, not pre-allocated"
        );
    }

    /// The hop count survives an encode/decode round trip.
    #[test]
    fn lsa_hop_count_roundtrip() {
        let mut lsa = LinkStateAdvertisement::new("peer-alpha", 7, 90, vec![], 12_345);
        lsa = flood_relay_lsa(&lsa);
        lsa = flood_relay_lsa(&lsa);
        assert_eq!(lsa.hops_traversed, 2);
        let decoded = LinkStateAdvertisement::decode(&lsa.encode()).unwrap();
        assert_eq!(decoded, lsa);
    }
}
