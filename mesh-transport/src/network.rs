//! libp2p swarm wiring for identity-based mesh routing.
//!
//! Uses the workspace `libp2p` Kademlia behaviour (reticulum-rs is not on
//! crates.io). Complements the pure-Rust [`crate::kademlia_routing`] table
//! used for deterministic offline / unit-test pathfinding.

use std::error::Error;
use std::time::Duration;

use libp2p::{
    identity, kad,
    multiaddr::Protocol,
    swarm::{NetworkBehaviour, Swarm},
    Multiaddr, PeerId, SwarmBuilder,
};

use crate::identity_manager::{IdentityManager, LocalIdentity, PeerIdBytes};
use crate::kademlia_routing::{
    iterative_find_node, FindNodeResult, LocalTableRpc, NodeContact, NodeId, RoutingTable, K,
};

/// Composite NetworkBehaviour: Kademlia DHT only (Phase-1 transport).
///
/// libp2p 0.53+ auto-generates `MeshBehaviourEvent` from this derive.
#[derive(NetworkBehaviour)]
pub struct MeshBehaviour {
    pub kademlia: kad::Behaviour<kad::store::MemoryStore>,
}

/// Build a server-mode Kademlia behaviour for `local_key`.
pub fn build_mesh_behaviour(local_key: &identity::Keypair) -> MeshBehaviour {
    let peer_id = PeerId::from(local_key.public());
    let store = kad::store::MemoryStore::new(peer_id);
    let mut kademlia = kad::Behaviour::new(peer_id, store);
    // Server mode: answer DHT queries for the local island.
    kademlia.set_mode(Some(kad::Mode::Server));
    MeshBehaviour { kademlia }
}

/// Generate an ephemeral libp2p Ed25519 keypair.
pub fn generate_libp2p_identity() -> identity::Keypair {
    identity::Keypair::generate_ed25519()
}

/// Deterministic libp2p keypair from a 32-byte seed (test / bootstrap nodes).
/// Note: `ed25519_from_bytes` zeroizes the provided buffer.
pub fn libp2p_identity_from_seed(
    mut seed: [u8; 32],
) -> Result<identity::Keypair, libp2p::identity::DecodingError> {
    identity::Keypair::ed25519_from_bytes(&mut seed)
}

/// Construct a Tokio swarm with TCP + Noise + Yamux and Kademlia.
pub async fn build_swarm(
    local_key: identity::Keypair,
) -> Result<Swarm<MeshBehaviour>, Box<dyn Error + Send + Sync>> {
    let behaviour = build_mesh_behaviour(&local_key);
    let swarm = SwarmBuilder::with_existing_identity(local_key)
        .with_tokio()
        .with_tcp(
            libp2p::tcp::Config::default(),
            libp2p::noise::Config::new,
            libp2p::yamux::Config::default,
        )?
        .with_behaviour(move |_key| Ok(behaviour))?
        .with_swarm_config(|cfg| cfg.with_idle_connection_timeout(Duration::from_secs(60)))
        .build();
    Ok(swarm)
}

/// Listen on an ephemeral TCP port; returns the bound multiaddr if available.
pub fn listen_on_tcp(swarm: &mut Swarm<MeshBehaviour>, port: u16) -> Result<Multiaddr, Box<dyn Error>> {
    let addr: Multiaddr = format!("/ip4/0.0.0.0/tcp/{port}").parse()?;
    swarm.listen_on(addr.clone())?;
    Ok(addr)
}

/// Dial a peer multiaddr and inject it into the Kademlia routing table.
pub fn dial_and_add_address(
    swarm: &mut Swarm<MeshBehaviour>,
    peer: PeerId,
    addr: Multiaddr,
) -> Result<(), Box<dyn Error>> {
    swarm.behaviour_mut().kademlia.add_address(&peer, addr.clone());
    swarm.dial(addr)?;
    Ok(())
}

/// Issue a Kademlia GET_CLOSEST_PEERS query for `target`.
pub fn bootstrap_query(swarm: &mut Swarm<MeshBehaviour>, target: PeerId) -> kad::QueryId {
    swarm.behaviour_mut().kademlia.get_closest_peers(target)
}

/// Map a MossyMesh peer into a multiaddr with /p2p component.
pub fn mesh_peer_multiaddr(ip: &str, port: u16, peer: &PeerId) -> Multiaddr {
    let mut addr = Multiaddr::empty();
    if let Ok(v4) = ip.parse::<std::net::Ipv4Addr>() {
        addr.push(Protocol::Ip4(v4));
    } else {
        addr.push(Protocol::Ip4(std::net::Ipv4Addr::UNSPECIFIED));
    }
    addr.push(Protocol::Tcp(port));
    addr.push(Protocol::P2p(*peer));
    addr
}

// ---------------------------------------------------------------------------
// Offline / pure-Rust mesh node façade (no live sockets required)
// ---------------------------------------------------------------------------

/// High-level transport node combining identity + pure Kademlia table.
/// Used by the daemon boot path when a full async swarm is not yet running.
#[derive(Debug)]
pub struct MeshNode {
    pub identity: IdentityManager,
    pub routing: RoutingTable,
}

impl MeshNode {
    /// Bootstrap a node from a seed string (deterministic PeerID + empty table).
    pub fn bootstrap(seed: &[u8]) -> Self {
        let mut identity = IdentityManager::new();
        let peer = identity.bootstrap_from_seed(seed).clone();
        let routing = RoutingTable::new(peer.id);
        Self { identity, routing }
    }

    pub fn from_local_identity(local: LocalIdentity) -> Self {
        let mut identity = IdentityManager::new();
        let peer = identity.set_local(local).clone();
        let routing = RoutingTable::new(peer.id);
        Self { identity, routing }
    }

    pub fn local_id(&self) -> Option<PeerIdBytes> {
        self.identity.local_id_bytes()
    }

    pub fn insert_peer(&mut self, id: NodeId, endpoint: impl Into<String>) -> bool {
        self.routing
            .insert(NodeContact::new(id, endpoint.into()))
    }

    pub fn closest_peers(&self, target: &NodeId, count: usize) -> Vec<NodeContact> {
        self.routing.closest(target, count.min(K))
    }

    /// Deterministic multi-table iterative lookup using only local knowledge maps.
    pub fn find_node_iterative(
        &self,
        target: &NodeId,
        rpc: &LocalTableRpc<'_>,
    ) -> FindNodeResult {
        iterative_find_node(&self.routing, target, rpc)
    }

    pub fn announce_app(&mut self, app: &str, aspects: &[&str]) -> Option<[u8; 32]> {
        self.identity
            .announce_destination(app, aspects)
            .map(|d| d.hash)
    }
}

/// Seed a pure-Rust routing view from the local identity manager peer list.
pub fn sync_peers_into_table(node: &mut MeshNode) {
    let local = match node.local_id() {
        Some(id) => id,
        None => return,
    };
    if node.routing.local_id != local {
        node.routing = RoutingTable::new(local);
    }
    for peer in node.identity.peers().to_vec() {
        node.routing.insert(NodeContact::new(
            peer.id,
            format!("peer:{}", hex_short(&peer.id)),
        ));
    }
}

fn hex_short(id: &NodeId) -> String {
    id.iter()
        .take(4)
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join("")
}

/// Synchronous init path for the daemon (no executor required).
pub fn init_network() {
    println!("Initializing mesh network façade (libp2p Kademlia + pure DHT table).");
    let mut node = MeshNode::bootstrap(b"mossymesh-daemon-node");
    let local = node.local_id().expect("bootstrapped");
    for i in 1u8..=5 {
        let mut id = local;
        id[31] ^= i;
        node.insert_peer(id, format!("mesh://island/peer/{i}"));
    }
    let dest = node.announce_app("mesh", &["lxmf", "delivery"]);
    println!(
        "MeshNode online: peer={}… contacts={} dest={}…",
        hex_short(&local),
        node.routing.len(),
        dest
            .map(|h| hex_short(&h))
            .unwrap_or_else(|| "none".into())
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kademlia_routing::node_id_from_u64;

    #[test]
    fn mesh_node_bootstrap_deterministic() {
        let a = MeshNode::bootstrap(b"same-seed");
        let b = MeshNode::bootstrap(b"same-seed");
        assert_eq!(a.local_id(), b.local_id());
    }

    #[test]
    fn mesh_node_insert_and_closest() {
        let mut node = MeshNode::bootstrap(b"n0");
        for v in 1..10u64 {
            node.insert_peer(node_id_from_u64(v), format!("e{v}"));
        }
        let target = node_id_from_u64(3);
        let closest = node.closest_peers(&target, 3);
        assert_eq!(closest.len(), 3);
        assert_eq!(closest[0].id, target);
    }

    #[test]
    fn build_mesh_behaviour_smoke() {
        let key = generate_libp2p_identity();
        let peer = PeerId::from(key.public());
        let behaviour = build_mesh_behaviour(&key);
        assert!(!peer.to_string().is_empty());
        let _ = format!("{:?}", behaviour.kademlia.mode());
    }

    #[test]
    fn libp2p_seed_identity_deterministic() {
        let seed = [42u8; 32];
        let a = libp2p_identity_from_seed(seed).expect("seed key a");
        let b = libp2p_identity_from_seed(seed).expect("seed key b");
        assert_eq!(PeerId::from(a.public()), PeerId::from(b.public()));
    }
}
