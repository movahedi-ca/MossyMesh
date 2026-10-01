//! MossyMesh Daemon Entrypoint
//! DOC 54: This binary wires the isolated crates together into a single, cohesive offline routing node.

use consensus::init_consensus;
use engine::init_engine;
use interop::init_interop;
use libp2p::Multiaddr;
use mesh_transport::ble_mesh::init_ble_mesh;
use mesh_transport::identity_manager::init_identity_manager;
use mesh_transport::kademlia_routing::{
    find_node_local, init_kademlia_routing, node_id_from_u8, NodeContact, RoutingTable,
};
use mesh_transport::network::{
    init_network, parse_node_seed_hex, split_peer_multiaddr, MeshNode, P2pNodeOptions,
};
use mesh_transport::stun_hole_punch::init_stun_hole_punch;
use mesh_transport::wifi_direct::{WifiDirectManager, WifiPeer, WifiState};
use sandbox::init_sandbox;
use std::time::Duration;

/// Parse `MESH_LISTEN_ADDR` (`127.0.0.1:PORT`) into ip + port.
fn parse_listen_addr(raw: &str) -> (std::net::Ipv4Addr, u16) {
    let (ip, port) = raw.rsplit_once(':').unwrap_or(("", ""));
    match (ip.parse::<std::net::Ipv4Addr>(), port.parse::<u16>()) {
        (Ok(ip), Ok(port)) => (ip, port),
        _ => {
            eprintln!("[Network] bad MESH_LISTEN_ADDR {raw:?}: expected 127.0.0.1:PORT");
            std::process::exit(1);
        }
    }
}

#[tokio::main]
async fn main() {
    println!("==================================================");
    println!("=           MOSSYMESH DAEMON BOOTING             =");
    println!("==================================================");

    // 1. Initialize Sub-Crates
    init_consensus();
    init_engine();
    init_sandbox();
    init_interop();

    // 2. Boot identity-based routing stack (TransportAgent)
    println!("\n[Transport] Booting identity + Kademlia + hole-punch…");
    init_identity_manager();
    init_kademlia_routing();
    init_stun_hole_punch();
    init_network();
    init_ble_mesh();

    // 2b. Minimal live API exercise (pure-Rust, no async executor required)
    let mut node = MeshNode::bootstrap(b"mossymesh-daemon-node");
    for i in 1u8..=8 {
        let id = node_id_from_u8(i);
        node.insert_peer(id, format!("mesh://boot/{i}"));
    }
    let target = node_id_from_u8(3);
    if let Some(hit) = find_node_local(&node.routing, &target) {
        println!(
            "[Transport] find_node_local => endpoint={} contacts={}",
            hit.endpoint,
            node.routing.len()
        );
    }
    let mut table = RoutingTable::new(node.local_id().unwrap_or([0u8; 32]));
    table.insert(NodeContact::new(target, "mesh://boot/3"));
    println!(
        "[Transport] RoutingTable k-bucket demo: {} contact(s)",
        table.len()
    );

    // 3. Negotiate Swarm Leadership
    println!("\n[Network] Negotiating offline Access Point leadership...");
    let mut wifi_manager = WifiDirectManager::new(950); // High simulated battery weight
    wifi_manager
        .peers_in_range
        .push(WifiPeer::new("low_power_peer", 150));
    wifi_manager.negotiate_group_owner();

    match wifi_manager.state {
        WifiState::GroupOwner => println!(
            "[Network] Successfully claimed Group Owner status. Broadcasting SSID: MossyMesh_Local"
        ),
        WifiState::Client => println!("[Network] Yielded to stronger peer. Connecting as Client."),
        _ => println!("[Network] Isolated state."),
    }

    // 4. Mount Interop Bridging
    println!("\n[Interop] Mounting HTTP API endpoints...");

    let server_handle = tokio::spawn(async {
        interop::run_http_server().await;
    });

    // Simulate persistent Websocket sync thread if external internet is available
    match std::env::var("MESH_LISTEN_ADDR") {
        Ok(listen_raw) => {
            // REAL p2p mode: drive a live libp2p swarm (with active ping)
            // alongside the HTTP gateway. Every `ping` line in the status
            // file is a real packet over a real socket.
            println!("\n[Network] REAL p2p mode active: spinning up live libp2p swarm...");
            let (ip, port) = parse_listen_addr(&listen_raw);
            let dial_peer = std::env::var("MESH_PEER_ADDR").ok().map(|raw| {
                raw.parse::<Multiaddr>()
                    .ok()
                    .and_then(|m| split_peer_multiaddr(&m))
                    .unwrap_or_else(|| {
                        eprintln!("[Network] bad MESH_PEER_ADDR {raw:?}: expected full multiaddr with /p2p/<peer-id>");
                        std::process::exit(1);
                    })
            });
            let seed = std::env::var("MESH_NODE_SEED").ok().map(|raw| {
                parse_node_seed_hex(&raw).unwrap_or_else(|| {
                    eprintln!("[Network] bad MESH_NODE_SEED: expected 64 hex chars");
                    std::process::exit(1);
                })
            });
            let status_file = std::env::var("MESH_STATUS_FILE")
                .unwrap_or_else(|_| "/tmp/mesh-p2p-status.jsonl".to_string());
            println!(
                "[Network] p2p listen={listen_raw} dial={} status={status_file}",
                dial_peer
                    .as_ref()
                    .map(|(_, a)| a.to_string())
                    .unwrap_or_else(|| "none".into())
            );
            let p2p_handle = tokio::spawn(mesh_transport::network::run_p2p_node(P2pNodeOptions {
                listen_ip: ip,
                listen_port: port,
                seed,
                dial_peer,
                status_file: status_file.into(),
                ping_interval: Duration::from_secs(15),
            }));

            println!("\n[Daemon] Entering event loop (HTTP gateway + real p2p swarm)...");
            // select, not join: run_p2p_node loops forever, so join would hang
            // if the HTTP task failed first and never report the failure.
            tokio::select! {
                res = server_handle => {
                    if let Err(e) = res {
                        eprintln!("[Daemon] HTTP server task failed: {e}");
                        std::process::exit(1);
                    }
                }
                res = p2p_handle => {
                    if let Err(e) = res {
                        eprintln!("[Daemon] p2p task failed: {e}");
                        std::process::exit(1);
                    }
                }
            }
        }
        Err(_) => {
            println!("\n[Daemon] Entering event loop...");
            // We let the HTTP server run indefinitely
            server_handle.await.unwrap();
        }
    }

    println!("==================================================");
    println!("=          MOSSYMESH DAEMON TERMINATED           =");
    println!("==================================================");
}
