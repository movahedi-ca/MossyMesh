//! MossyMesh Daemon Entrypoint
//! DOC 54: This binary wires the isolated crates together into a single, cohesive offline routing node.

use consensus::init_consensus;
use engine::init_engine;
use sandbox::init_sandbox;
use interop::{init_interop, AsyncApiRequest, handle_rest_call, handle_websocket};
use mesh_transport::wifi_direct::{WifiDirectManager, WifiState};
use mesh_transport::ble_mesh::init_ble_mesh;
use mesh_transport::identity_manager::init_identity_manager;
use mesh_transport::kademlia_routing::{
    find_node_local, init_kademlia_routing, node_id_from_u8, NodeContact, RoutingTable,
};
use mesh_transport::network::{init_network, MeshNode};
use mesh_transport::stun_hole_punch::init_stun_hole_punch;

fn main() {
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
    wifi_manager.peers_in_range.push(("low_power_peer".to_string(), 150));
    wifi_manager.negotiate_group_owner();

    match wifi_manager.state {
        WifiState::GroupOwner => println!("[Network] Successfully claimed Group Owner status. Broadcasting SSID: MossyMesh_Local"),
        WifiState::Client => println!("[Network] Yielded to stronger peer. Connecting as Client."),
        _ => println!("[Network] Isolated state."),
    }

    // 4. Mount Interop Bridging
    println!("\n[Interop] Mounting mock API endpoints...");
    let mock_req = AsyncApiRequest {
        endpoint: "/api/v1/submit_job".to_string(),
        payload: "{\"action\":\"move\",\"from\":[1,4],\"to\":[3,4]}".to_string(),
    };

    match handle_rest_call(&mock_req) {
        Ok(msg) => println!("[Interop] API Response: {}", msg),
        Err(_) => println!("[Interop] API Failed."),
    }

    // Simulate persistent Websocket sync thread if external internet is available
    println!("\n[Daemon] Entering event loop...");
    handle_websocket(true);

    println!("==================================================");
    println!("=          MOSSYMESH DAEMON TERMINATED           =");
    println!("==================================================");
}
