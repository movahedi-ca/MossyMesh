pub mod lora_mac;
pub mod ble_mesh;
pub mod kademlia_routing;
pub mod stun_hole_punch;
pub mod identity_manager;
pub mod wifi_direct;
pub mod packet_translator;
pub mod encryption_layer;
pub mod thermal_aware;
pub mod vrf_assigner;
pub mod battery_tracker;
pub mod quarantine;
pub mod honeypot;
pub mod hash_chain;
pub mod vdf_sybil;
pub mod topology;

pub fn init_mesh_transport() {
    // Placeholder for reticulum-rs initialization
    println!("Mesh Transport layer initialized.");
    
    // Initialize 16-agent transport layers
    lora_mac::init_lora_mac();
    ble_mesh::init_ble_mesh();
    kademlia_routing::init_kademlia_routing();
    stun_hole_punch::init_stun_hole_punch();
    identity_manager::init_identity_manager();
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
}
