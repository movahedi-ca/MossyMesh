//! libp2p swarm wiring for identity-based mesh routing.
//!
//! Uses the workspace `libp2p` Kademlia behaviour (reticulum-rs is not on
//! crates.io). Complements the pure-Rust [`crate::kademlia_routing`] table
//! used for deterministic offline / unit-test pathfinding.

use std::error::Error;
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use libp2p::{
    identity, kad,
    multiaddr::Protocol,
    ping,
    swarm::{NetworkBehaviour, Swarm, SwarmEvent},
    Multiaddr, PeerId, SwarmBuilder,
};

use crate::identity_manager::{IdentityManager, LocalIdentity, PeerIdBytes};
use crate::kademlia_routing::{
    iterative_find_node, FindNodeResult, LocalTableRpc, NodeContact, NodeId, RoutingTable, K,
};

/// Composite NetworkBehaviour: Kademlia DHT + active ping probing.
///
/// libp2p 0.53+ auto-generates `MeshBehaviourEvent` from this derive.
/// Ping is the one real network path wired end to end: every connection
/// gets probed on an interval and each successful round trip is a real
/// packet sent and received over a real socket.
#[derive(NetworkBehaviour)]
pub struct MeshBehaviour {
    pub kademlia: kad::Behaviour<kad::store::MemoryStore>,
    pub ping: ping::Behaviour,
}

/// Build a server-mode Kademlia + ping behaviour for `local_key`.
pub fn build_mesh_behaviour(local_key: &identity::Keypair) -> MeshBehaviour {
    build_mesh_behaviour_with_ping_config(local_key, ping::Config::default())
}

/// Build the behaviour with an explicit ping config (tests use a short interval).
pub fn build_mesh_behaviour_with_ping_config(
    local_key: &identity::Keypair,
    ping_config: ping::Config,
) -> MeshBehaviour {
    let peer_id = PeerId::from(local_key.public());
    let store = kad::store::MemoryStore::new(peer_id);
    let mut kademlia = kad::Behaviour::new(peer_id, store);
    // Server mode: answer DHT queries for the local island.
    kademlia.set_mode(Some(kad::Mode::Server));
    MeshBehaviour {
        kademlia,
        ping: ping::Behaviour::new(ping_config),
    }
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

/// Construct a Tokio swarm with TCP + Noise + Yamux, Kademlia, and ping.
pub async fn build_swarm(
    local_key: identity::Keypair,
) -> Result<Swarm<MeshBehaviour>, Box<dyn Error + Send + Sync>> {
    build_swarm_with_ping_config(local_key, ping::Config::default()).await
}

/// Swarm constructor with an explicit ping config (tests use a short interval).
pub async fn build_swarm_with_ping_config(
    local_key: identity::Keypair,
    ping_config: ping::Config,
) -> Result<Swarm<MeshBehaviour>, Box<dyn Error + Send + Sync>> {
    let behaviour = build_mesh_behaviour_with_ping_config(&local_key, ping_config);
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
pub fn listen_on_tcp(
    swarm: &mut Swarm<MeshBehaviour>,
    port: u16,
) -> Result<Multiaddr, Box<dyn Error>> {
    let addr: Multiaddr = format!("/ip4/0.0.0.0/tcp/{port}").parse()?;
    swarm.listen_on(addr.clone())?;
    Ok(addr)
}

/// Dial a peer multiaddr and inject it into the Kademlia routing table.
pub fn dial_and_add_address(
    swarm: &mut Swarm<MeshBehaviour>,
    peer: PeerId,
    addr: Multiaddr,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    swarm
        .behaviour_mut()
        .kademlia
        .add_address(&peer, addr.clone());
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

/// Split a full multiaddr with a `/p2p/<peer-id>` component into
/// the peer id and the dialable address.
pub fn split_peer_multiaddr(addr: &Multiaddr) -> Option<(PeerId, Multiaddr)> {
    let peer = addr.iter().find_map(|proto| match proto {
        Protocol::P2p(id) => Some(id),
        _ => None,
    })?;
    Some((peer, addr.clone()))
}

/// Decode a 32-byte node seed from hex (the `MESH_NODE_SEED` env format).
pub fn parse_node_seed_hex(hex: &str) -> Option<[u8; 32]> {
    let hex = hex.trim();
    if hex.len() != 64 {
        return None;
    }
    let mut seed = [0u8; 32];
    for (i, chunk) in seed.iter_mut().enumerate() {
        *chunk = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(seed)
}

// ---------------------------------------------------------------------------
// Real p2p node: a live libp2p swarm with active ping, driven forever.
// This is the one real network path: every ping round trip logged here is
// a real packet exchanged over a real TCP socket between two real processes.
// ---------------------------------------------------------------------------

/// Options for [`run_p2p_node`].
pub struct P2pNodeOptions {
    pub listen_ip: Ipv4Addr,
    pub listen_port: u16,
    /// Deterministic identity when set; ephemeral keypair otherwise.
    pub seed: Option<[u8; 32]>,
    /// Bootstrap peer to dial on startup (peer id + full multiaddr).
    pub dial_peer: Option<(PeerId, Multiaddr)>,
    /// JSON-lines status file; every real network event is appended here.
    pub status_file: PathBuf,
    /// Ping probe interval.
    pub ping_interval: Duration,
}

fn append_status_line(path: &Path, line: &str) {
    use std::fs::OpenOptions;
    use std::io::Write;
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{line}");
    }
}

/// Run a live libp2p node forever.
///
/// Listens on the configured TCP address, dials the bootstrap peer when
/// configured, and appends JSON lines for `listening`, `peer_connected`,
/// and `ping` (with real round-trip times) events to the status file.
/// Only returns on unrecoverable error.
pub async fn run_p2p_node(opts: P2pNodeOptions) -> Result<(), Box<dyn Error + Send + Sync>> {
    let local_key = match opts.seed {
        Some(seed) => libp2p_identity_from_seed(seed)?,
        None => generate_libp2p_identity(),
    };
    let local_peer_id = PeerId::from(local_key.public());
    let ping_config = ping::Config::new().with_interval(opts.ping_interval);
    let mut swarm = build_swarm_with_ping_config(local_key, ping_config).await?;

    let listen: Multiaddr = format!("/ip4/{}/tcp/{}", opts.listen_ip, opts.listen_port)
        .parse()
        .map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("bad listen address: {e}"),
            )
        })?;
    swarm.listen_on(listen)?;

    if let Some((peer, addr)) = opts.dial_peer {
        dial_and_add_address(&mut swarm, peer, addr)?;
    }

    loop {
        use libp2p::futures::StreamExt;
        match swarm.select_next_some().await {
            SwarmEvent::NewListenAddr { address, .. } => {
                println!("[p2p] listening on {address} (peer {local_peer_id})");
                append_status_line(
                    &opts.status_file,
                    &format!(
                        "{{\"event\":\"listening\",\"peer_id\":\"{local_peer_id}\",\"addr\":\"{address}\"}}"
                    ),
                );
            }
            SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                println!("[p2p] connected to {peer_id}");
                append_status_line(
                    &opts.status_file,
                    &format!(
                        "{{\"event\":\"peer_connected\",\"peer_id\":\"{local_peer_id}\",\"peer\":\"{peer_id}\"}}"
                    ),
                );
            }
            SwarmEvent::Behaviour(MeshBehaviourEvent::Ping(ping::Event {
                peer,
                result: Ok(rtt),
                ..
            })) => {
                let rtt_ms = rtt.as_secs_f64() * 1000.0;
                println!("[p2p] ping {peer}: {rtt_ms:.2} ms");
                append_status_line(
                    &opts.status_file,
                    &format!(
                        "{{\"event\":\"ping\",\"peer_id\":\"{local_peer_id}\",\"peer\":\"{peer}\",\"rtt_ms\":{rtt_ms:.3}}}"
                    ),
                );
            }
            SwarmEvent::Behaviour(MeshBehaviourEvent::Ping(ping::Event {
                peer,
                result: Err(_),
                ..
            })) => {
                println!("[p2p] ping to {peer} failed");
            }
            _ => {}
        }
    }
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
        self.routing.insert(NodeContact::new(id, endpoint.into()))
    }

    pub fn closest_peers(&self, target: &NodeId, count: usize) -> Vec<NodeContact> {
        self.routing.closest(target, count.min(K))
    }

    /// Deterministic multi-table iterative lookup using only local knowledge maps.
    pub fn find_node_iterative(&self, target: &NodeId, rpc: &LocalTableRpc<'_>) -> FindNodeResult {
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
        dest.map(|h| hex_short(&h)).unwrap_or_else(|| "none".into())
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kademlia_routing::node_id_from_u64;
    use libp2p::swarm::SwarmEvent;

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
        // Behaviour is constructible; mode() is private on this libp2p version.
        let _ = behaviour;
    }

    #[test]
    fn libp2p_seed_identity_deterministic() {
        let seed = [42u8; 32];
        let a = libp2p_identity_from_seed(seed).expect("seed key a");
        let b = libp2p_identity_from_seed(seed).expect("seed key b");
        assert_eq!(PeerId::from(a.public()), PeerId::from(b.public()));
    }

    fn concrete_tcp_addr(ev: SwarmEvent<MeshBehaviourEvent>) -> Option<Multiaddr> {
        if let SwarmEvent::NewListenAddr { address, .. } = ev {
            let s = address.to_string();
            if s.starts_with("/ip4/127.0.0.1/tcp/") && !s.ends_with("/tcp/0") {
                return Some(address);
            }
        }
        None
    }

    /// Real socket test: two live swarms on ephemeral loopback ports, B dials
    /// A, both sides must see ConnectionEstablished and at least one side must
    /// observe a successful ping round trip. No mocks anywhere in this path.
    #[tokio::test]
    async fn two_real_swarms_connect_and_ping() {
        use libp2p::futures::StreamExt;
        use libp2p::swarm::SwarmEvent;

        let ping_cfg = || ping::Config::new().with_interval(Duration::from_secs(1));
        let mut a = build_swarm_with_ping_config(generate_libp2p_identity(), ping_cfg())
            .await
            .expect("build swarm a");
        let mut b = build_swarm_with_ping_config(generate_libp2p_identity(), ping_cfg())
            .await
            .expect("build swarm b");

        let a_id = *a.local_peer_id();
        let b_id = *b.local_peer_id();
        a.listen_on("/ip4/127.0.0.1/tcp/0".parse::<Multiaddr>().expect("addr"))
            .expect("listen a");
        b.listen_on("/ip4/127.0.0.1/tcp/0".parse::<Multiaddr>().expect("addr"))
            .expect("listen b");

        // Drive both swarms until each reports its concrete bound address.
        let (a_addr, _b_addr) = tokio::time::timeout(Duration::from_secs(10), async {
            let mut a_addr = None;
            let mut b_addr = None;
            while a_addr.is_none() || b_addr.is_none() {
                tokio::select! {
                    ev = a.select_next_some() => {
                        if a_addr.is_none() {
                            a_addr = concrete_tcp_addr(ev);
                        }
                    }
                    ev = b.select_next_some() => {
                        if b_addr.is_none() {
                            b_addr = concrete_tcp_addr(ev);
                        }
                    }
                }
            }
            (a_addr.unwrap(), b_addr.unwrap())
        })
        .await
        .expect("both swarms bound within 10s");

        // B dials A's real address (with /p2p component, like a real bootstrap).
        let mut dial_addr = a_addr;
        dial_addr.push(Protocol::P2p(a_id));
        dial_and_add_address(&mut b, a_id, dial_addr).expect("dial a");

        let (a_conn, b_conn, ping_ok) = tokio::time::timeout(Duration::from_secs(30), async {
            let mut a_conn = false;
            let mut b_conn = false;
            let mut ping_ok = false;
            while !(a_conn && b_conn && ping_ok) {
                tokio::select! {
                    ev = a.select_next_some() => match ev {
                        SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                            assert_eq!(peer_id, b_id);
                            a_conn = true;
                        }
                        SwarmEvent::Behaviour(MeshBehaviourEvent::Ping(ping::Event {
                            result: Ok(_),
                            ..
                        })) => {
                            ping_ok = true;
                        }
                        _ => {}
                    },
                    ev = b.select_next_some() => match ev {
                        SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                            assert_eq!(peer_id, a_id);
                            b_conn = true;
                        }
                        SwarmEvent::Behaviour(MeshBehaviourEvent::Ping(ping::Event {
                            result: Ok(_),
                            ..
                        })) => {
                            ping_ok = true;
                        }
                        _ => {}
                    },
                }
            }
            (a_conn, b_conn, ping_ok)
        })
        .await
        .expect("connection + ping within 30s");

        assert!(a_conn, "swarm A never saw ConnectionEstablished");
        assert!(b_conn, "swarm B never saw ConnectionEstablished");
        assert!(ping_ok, "no successful ping round trip observed");
    }
}
