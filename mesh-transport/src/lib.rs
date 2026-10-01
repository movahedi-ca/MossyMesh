pub mod battery_tracker;
pub mod ble_auth;
pub mod ble_hal;
pub mod ble_mesh;
pub mod ble_peripheral;
pub mod encryption_layer;
pub mod fen;
pub mod hash_chain;
pub mod honeypot;
pub mod identity_manager;
pub mod kademlia_routing;
pub mod lora_mac;
pub mod network;
pub mod packet_translator;
pub mod quarantine;
pub mod simulation;
pub mod stun_hole_punch;
pub mod thermal_aware;
pub mod topology;
pub mod vdf_sybil;
pub mod vrf_assigner;
pub mod wifi_direct;

pub fn init_mesh_transport() {
    // Placeholder for reticulum-rs initialization (libp2p Kademlia used instead)
    println!("Mesh Transport layer initialized.");

    // Core identity-based routing stack (TransportAgent)
    identity_manager::init_identity_manager();
    kademlia_routing::init_kademlia_routing();
    stun_hole_punch::init_stun_hole_punch();
    network::init_network();

    // Remaining transport layers (owned by other agents)
    lora_mac::init_lora_mac();
    ble_mesh::init_ble_mesh();
    wifi_direct::init_wifi_direct();
    packet_translator::init_packet_translator();
    encryption_layer::init_encryption_layer();
    thermal_aware::init_thermal_aware();
    vrf_assigner::init_vrf_assigner();
    battery_tracker::init_battery_tracker();
    quarantine::init_quarantine();
    honeypot::init_honeypot();
    hash_chain::init_hash_chain();
    vdf_sybil::init_vdf_sybil();
    topology::init_topology();
    simulation::init_simulation();
}
