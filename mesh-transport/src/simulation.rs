//! Offline mesh-island packet-path simulation.
//!
//! Phase-1 DoD (P1-DoD-1): a smartphone test packet translates to LoRa frames
//! and routes via Kademlia DHT discovery + multi-link topology pathfinding to
//! a second offline peer — fully deterministic, no sockets / DNS / cloud.

use std::collections::BTreeMap;

use log::info;
use tokio::time::{sleep, Duration};

use crate::kademlia_routing::{
    iterative_find_node, node_id_from_u64, FindNodeResult, LocalTableRpc, NodeContact, NodeId,
    RoutingTable,
};
use crate::lora_mac::LoraFrame;
use crate::packet_translator::{
    reassemble_mesh_packet, smartphone_to_lora_frames, ReticulumPacket, SmartphonePacket,
    TranslateError,
};
use crate::topology::{LinkType, Path, TopologyGraph};

// ---------------------------------------------------------------------------
// Legacy async helper (kept for call sites that only need a delay stub)
// ---------------------------------------------------------------------------

pub async fn simulate_lora_transmission(data: &[u8], target: &str) {
    info!(
        "Transmitting {} bytes via simulated LoRa to node: {}",
        data.len(),
        target
    );
    sleep(Duration::from_millis(150)).await;
    info!("LoRa transmission successful!");
}

// ---------------------------------------------------------------------------
// Offline island simulation
// ---------------------------------------------------------------------------

/// A named peer on a disconnected mesh island (no public DNS / upstream IP).
#[derive(Debug, Clone)]
pub struct SimNode {
    pub name: String,
    pub id: NodeId,
    pub routing: RoutingTable,
}

impl SimNode {
    pub fn new(name: impl Into<String>, id: NodeId) -> Self {
        let name = name.into();
        Self {
            name,
            id,
            routing: RoutingTable::new(id),
        }
    }
}

/// One hop taken while forwarding LoRa frames across the island.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutedHop {
    pub from: String,
    pub to: String,
    pub link: LinkType,
    pub cost: u32,
    pub frame_count: usize,
}

/// Outcome of an end-to-end offline island packet path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PacketPathResult {
    /// Named path from topology Dijkstra (src … dst).
    pub topology_path: Path,
    /// Kademlia iterative FIND_NODE result toward the destination id.
    pub kad_found_exact: bool,
    pub kad_rounds: usize,
    pub kad_closest_endpoints: Vec<String>,
    /// Hop-by-hop LoRa frame forwards.
    pub hops: Vec<RoutedHop>,
    /// Reassembled mesh packet at the destination offline peer.
    pub delivered: ReticulumPacket,
    /// Number of LoRa frames that traversed the path.
    pub frame_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SimError {
    UnknownNode(String),
    NoTopologyPath { from: String, to: String },
    KadMiss { target: String },
    Translate(TranslateError),
    EmptyIsland,
}

impl From<TranslateError> for SimError {
    fn from(e: TranslateError) -> Self {
        SimError::Translate(e)
    }
}

/// Deterministic multi-node offline island: Kademlia tables + link topology.
///
/// Nodes and edges use stable string names; Kademlia identities are fixed
/// `node_id_from_u64` values so pathfinding is bit-identical across runs.
#[derive(Debug, Clone)]
pub struct IslandSim {
    /// Name → node (BTreeMap for deterministic iteration).
    nodes: BTreeMap<String, SimNode>,
    pub topology: TopologyGraph,
}

impl IslandSim {
    pub fn new() -> Self {
        Self {
            nodes: BTreeMap::new(),
            topology: TopologyGraph::new(),
        }
    }

    /// Insert a peer with a fixed numeric id (tests / demos).
    pub fn add_peer(&mut self, name: impl Into<String>, id_num: u64) -> NodeId {
        let name = name.into();
        let id = node_id_from_u64(id_num);
        self.topology.add_node(name.clone());
        self.nodes
            .insert(name.clone(), SimNode::new(name, id));
        id
    }

    pub fn peer_id(&self, name: &str) -> Option<NodeId> {
        self.nodes.get(name).map(|n| n.id)
    }

    pub fn peer_names(&self) -> Vec<String> {
        self.nodes.keys().cloned().collect()
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Bidirectional RF/logical link on the island topology graph.
    pub fn link(
        &mut self,
        a: &str,
        b: &str,
        link: LinkType,
        quality: u8,
    ) -> Result<(), SimError> {
        if !self.nodes.contains_key(a) {
            return Err(SimError::UnknownNode(a.to_string()));
        }
        if !self.nodes.contains_key(b) {
            return Err(SimError::UnknownNode(b.to_string()));
        }
        self.topology.add_bidirectional(a, b, link, quality);
        Ok(())
    }

    /// Seed each node's Kademlia table with **direct topology neighbors only**.
    /// Multi-hop discovery must use iterative FIND_NODE (realistic island DHT).
    pub fn seed_kademlia_from_topology(&mut self) {
        // Snapshot neighbor names first to avoid borrow issues.
        let names: Vec<String> = self.nodes.keys().cloned().collect();
        let mut neighbor_map: BTreeMap<String, Vec<(String, NodeId)>> = BTreeMap::new();

        for name in &names {
            let mut neighbors = Vec::new();
            for edge in self.topology.links_from(name) {
                if let Some(peer) = self.nodes.get(&edge.to) {
                    neighbors.push((edge.to.clone(), peer.id));
                }
            }
            // Stable order for deterministic last_seen ticks.
            neighbors.sort_by(|a, b| a.0.cmp(&b.0));
            neighbors.dedup_by(|a, b| a.0 == b.0);
            neighbor_map.insert(name.clone(), neighbors);
        }

        for name in &names {
            let node = self.nodes.get_mut(name).unwrap();
            // Fresh table so re-seed is idempotent.
            node.routing = RoutingTable::new(node.id);
            for (nname, nid) in neighbor_map.get(name).into_iter().flatten() {
                node.routing.insert(NodeContact::new(
                    *nid,
                    format!("mesh://island/{nname}"),
                ));
            }
        }
    }

    /// Iterative Kademlia FIND_NODE from `src` toward `dst` peer id.
    pub fn kad_find(&self, src: &str, dst: &str) -> Result<FindNodeResult, SimError> {
        let origin = self
            .nodes
            .get(src)
            .ok_or_else(|| SimError::UnknownNode(src.to_string()))?;
        let target = self
            .nodes
            .get(dst)
            .ok_or_else(|| SimError::UnknownNode(dst.to_string()))?;

        let mut rpc = LocalTableRpc::new();
        for n in self.nodes.values() {
            rpc.register(&n.routing);
        }
        Ok(iterative_find_node(&origin.routing, &target.id, &rpc))
    }

    /// Lowest-cost multi-link path on the island topology.
    pub fn topology_path(&self, src: &str, dst: &str) -> Result<Path, SimError> {
        if !self.nodes.contains_key(src) {
            return Err(SimError::UnknownNode(src.to_string()));
        }
        if !self.nodes.contains_key(dst) {
            return Err(SimError::UnknownNode(dst.to_string()));
        }
        self.topology
            .shortest_path(src, dst)
            .ok_or_else(|| SimError::NoTopologyPath {
                from: src.to_string(),
                to: dst.to_string(),
            })
    }

    /// End-to-end: phone packet → LoRa frames → Kademlia discover offline peer
    /// → topology path → hop-by-hop frame delivery → reassembly at `dst`.
    ///
    /// Pure synchronous simulation (no wall-clock, no sockets). Deterministic
    /// given the same island graph and packet.
    pub fn route_packet(
        &self,
        src: &str,
        dst: &str,
        phone: &SmartphonePacket,
    ) -> Result<PacketPathResult, SimError> {
        if self.nodes.is_empty() {
            return Err(SimError::EmptyIsland);
        }

        // 1) Discover offline peer via Kademlia (identity-based, no DNS).
        let kad = self.kad_find(src, dst)?;
        let target_id = self.nodes.get(dst).unwrap().id;
        let found = kad.found_exact || kad.closest.iter().any(|c| c.id == target_id);
        if !found {
            return Err(SimError::KadMiss {
                target: dst.to_string(),
            });
        }

        // 2) Physical / multi-link path across the island.
        let topology_path = self.topology_path(src, dst)?;

        // 3) Translate smartphone packet → CRC'd LoRa frames.
        let frames = smartphone_to_lora_frames(phone)?;

        // 4) Forward every frame hop-by-hop along the topology path.
        let hops = forward_frames_along_path(&topology_path, frames.len());

        // 5) Destination offline peer reassembles the mesh packet.
        let delivered = reassemble_mesh_packet(&frames)?;

        Ok(PacketPathResult {
            topology_path,
            kad_found_exact: kad.found_exact,
            kad_rounds: kad.rounds,
            kad_closest_endpoints: kad
                .closest
                .iter()
                .map(|c| c.endpoint.clone())
                .collect(),
            hops,
            delivered,
            frame_count: frames.len(),
        })
    }
}

impl Default for IslandSim {
    fn default() -> Self {
        Self::new()
    }
}

/// Record simulated LoRa frame forwards for each topology hop.
fn forward_frames_along_path(path: &Path, frame_count: usize) -> Vec<RoutedHop> {
    path.hops
        .iter()
        .map(|e| RoutedHop {
            from: e.from.clone(),
            to: e.to.clone(),
            link: e.link,
            cost: e.cost,
            frame_count,
        })
        .collect()
}

/// Canonical Phase-1 offline island fixture (phone / pi / relay / offline peer).
///
/// Topology (multi-hop, no direct A↔B radio):
/// ```text
///   phone --WiFi-- pi --LoRa-- relay --BLE-- offline-b
/// ```
/// Kademlia tables only know direct neighbors; FIND_NODE walks the island.
pub fn phase1_offline_island() -> IslandSim {
    let mut island = IslandSim::new();
    // Fixed ids: A=phone, gateway=pi, relay, B=offline destination.
    island.add_peer("phone", 1);
    island.add_peer("pi", 2);
    island.add_peer("relay", 3);
    island.add_peer("offline-b", 9);

    island
        .link("phone", "pi", LinkType::Wifi, 240)
        .expect("phone-pi");
    island
        .link("pi", "relay", LinkType::LoRa, 200)
        .expect("pi-relay");
    island
        .link("relay", "offline-b", LinkType::Ble, 180)
        .expect("relay-offline-b");

    island.seed_kademlia_from_topology();
    island
}

/// Build LoRa frames for a path without requiring a full island (unit helper).
pub fn simulate_frame_delivery(frames: &[LoraFrame], hop_count: usize) -> usize {
    // Each hop "transmits" every frame once; return total simulated TX events.
    frames.len().saturating_mul(hop_count)
}

/// Daemon-style init (prints a smoke path; no network).
pub fn init_simulation() {
    println!("Initializing offline island pathfinding simulation (Phase-1 DoD).");
    let island = phase1_offline_island();
    let phone_pkt = SmartphonePacket {
        src_ip: "192.168.4.50".into(),
        dst_ip: "192.168.4.1".into(),
        payload: b"phase1-sim-boot".to_vec(),
    };
    match island.route_packet("phone", "offline-b", &phone_pkt) {
        Ok(r) => println!(
            "Sim path phone→offline-b: nodes={:?} cost={} frames={} kad_rounds={}",
            r.topology_path.nodes, r.topology_path.total_cost, r.frame_count, r.kad_rounds
        ),
        Err(e) => println!("Sim path failed: {:?}", e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_phone(payload: Vec<u8>) -> SmartphonePacket {
        SmartphonePacket {
            src_ip: "192.168.4.50".into(),
            dst_ip: "192.168.4.1".into(),
            payload,
        }
    }

    #[test]
    fn phase1_packet_routes_to_offline_peer_via_kademlia_and_topology() {
        // P1-DoD-1: test packet translates and routes via Kademlia to second
        // offline peer (simulation).
        let island = phase1_offline_island();
        let payload = b"test-packet-from-phone-to-offline-b".to_vec();
        let phone = sample_phone(payload.clone());

        let result = island
            .route_packet("phone", "offline-b", &phone)
            .expect("offline island path must succeed");

        // Topology multi-hop (no direct phone↔offline-b link).
        assert_eq!(
            result.topology_path.nodes,
            vec![
                "phone".to_string(),
                "pi".to_string(),
                "relay".to_string(),
                "offline-b".to_string()
            ]
        );
        assert_eq!(result.hops.len(), 3);
        assert_eq!(result.hops[0].link, LinkType::Wifi);
        assert_eq!(result.hops[1].link, LinkType::LoRa);
        assert_eq!(result.hops[2].link, LinkType::Ble);

        // Kademlia discovered the offline peer through neighbor walks.
        assert!(
            result.kad_found_exact
                || result
                    .kad_closest_endpoints
                    .iter()
                    .any(|e| e.contains("offline-b")),
            "Kademlia must surface offline-b; endpoints={:?}",
            result.kad_closest_endpoints
        );
        assert!(result.kad_rounds >= 1);

        // Packet translated + reassembled byte-identically at offline peer.
        assert!(result.frame_count >= 1);
        assert_eq!(result.delivered.destination_hash, [0x01; 16]);
        assert_eq!(result.delivered.data, payload);

        // Every hop forwarded the full frame set.
        for h in &result.hops {
            assert_eq!(h.frame_count, result.frame_count);
        }
    }

    #[test]
    fn offline_island_path_is_deterministic() {
        let island = phase1_offline_island();
        let phone = sample_phone(b"deterministic-payload".to_vec());

        let a = island.route_packet("phone", "offline-b", &phone).unwrap();
        let b = island.route_packet("phone", "offline-b", &phone).unwrap();

        assert_eq!(a.topology_path, b.topology_path);
        assert_eq!(a.hops, b.hops);
        assert_eq!(a.delivered, b.delivered);
        assert_eq!(a.frame_count, b.frame_count);
        assert_eq!(a.kad_rounds, b.kad_rounds);
        assert_eq!(a.kad_closest_endpoints, b.kad_closest_endpoints);
        assert_eq!(a.kad_found_exact, b.kad_found_exact);

        // Two independently built islands with the same fixture match.
        let island2 = phase1_offline_island();
        let c = island2.route_packet("phone", "offline-b", &phone).unwrap();
        assert_eq!(a, c);
    }

    #[test]
    fn kademlia_finds_offline_peer_without_direct_contact() {
        let island = phase1_offline_island();
        // Phone's local table must NOT already contain offline-b (multi-hop DHT).
        let phone = island.nodes.get("phone").unwrap();
        let offline_id = island.peer_id("offline-b").unwrap();
        assert!(
            phone.routing.get(&offline_id).is_none(),
            "phone should only know topology neighbors, not offline-b directly"
        );

        let res = island.kad_find("phone", "offline-b").unwrap();
        assert!(
            res.found_exact || res.closest.iter().any(|c| c.id == offline_id),
            "iterative FIND_NODE must reach offline-b; closest={:?}",
            res.closest
                .iter()
                .map(|c| c.endpoint.as_str())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn no_path_when_islands_partitioned() {
        let mut island = IslandSim::new();
        island.add_peer("A", 1);
        island.add_peer("B", 2);
        island.add_peer("C", 3);
        island.link("A", "B", LinkType::Wifi, 255).unwrap();
        // C is a separate component — never linked.
        island.seed_kademlia_from_topology();

        let phone = sample_phone(b"x".to_vec());
        let err = island.route_packet("A", "C", &phone).unwrap_err();
        // Either Kademlia cannot discover C (no path in DHT) or topology fails.
        match err {
            SimError::KadMiss { target } => assert_eq!(target, "C"),
            SimError::NoTopologyPath { from, to } => {
                assert_eq!(from, "A");
                assert_eq!(to, "C");
            }
            other => panic!("expected partition error, got {:?}", other),
        }
    }

    #[test]
    fn fragmented_payload_survives_multi_hop_route() {
        let island = phase1_offline_island();
        // Large enough to force multiple LoRa fragments.
        let big: Vec<u8> = (0u8..251).cycle().take(700).collect();
        let phone = sample_phone(big.clone());
        let result = island.route_packet("phone", "offline-b", &phone).unwrap();
        assert!(result.frame_count > 1, "expected multi-frame payload");
        assert_eq!(result.delivered.data, big);
        assert_eq!(result.topology_path.nodes.last().unwrap(), "offline-b");
    }

    #[test]
    fn frame_delivery_counter_is_stable() {
        let frames = smartphone_to_lora_frames(&sample_phone(b"hi".to_vec())).unwrap();
        assert_eq!(simulate_frame_delivery(&frames, 3), frames.len() * 3);
        assert_eq!(simulate_frame_delivery(&frames, 0), 0);
    }

    #[test]
    fn peer_roster_sorted_and_countable() {
        let island = phase1_offline_island();
        assert_eq!(island.node_count(), 4);
        assert_eq!(
            island.peer_names(),
            vec![
                "offline-b".to_string(),
                "phone".to_string(),
                "pi".to_string(),
                "relay".to_string()
            ]
        );
    }

    #[test]
    fn unknown_node_errors() {
        let island = phase1_offline_island();
        let phone = sample_phone(b"z".to_vec());
        assert!(matches!(
            island.route_packet("phone", "ghost", &phone),
            Err(SimError::UnknownNode(_))
        ));
        assert!(matches!(
            island.route_packet("ghost", "offline-b", &phone),
            Err(SimError::UnknownNode(_))
        ));
    }
}
