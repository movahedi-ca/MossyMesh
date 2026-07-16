//! Commit-and-Reveal seed logic for Verifiable Random Function (VRF) task routing.
//! Math Problem 4: Deterministic VRF Sortition Selection

/// Evaluates if a node is selected for a task based on its VRF hash output and its weight.
/// The formula for sortition is: 
/// Hash_Value < (Max_Hash * weight) / total_network_weight
/// Since we don't have U256 in the standard library, we simulate the top 64 bits of the hash.
pub fn is_selected_for_task(vrf_hash_top_64: u64, node_weight: u64, total_network_weight: u64) -> bool {
    if total_network_weight == 0 {
        return false;
    }
    
    // Calculate the threshold. 
    // We must use u128 for intermediate multiplication to prevent overflow.
    let threshold = ((u64::MAX as u128 * node_weight as u128) / total_network_weight as u128) as u64;
    
    // Node is selected if its cryptographic VRF output is below the threshold.
    vrf_hash_top_64 < threshold
}

pub fn init_vrf_assigner() {
    println!("Initializing VRF Assigner for dynamic primary/standby worker allocation.");
    let sample_hash = 0x0FFFFFFFFFFFFFFF; // A relatively low hash
    let weight = 100;
    let total_weight = 1000;
    let selected = is_selected_for_task(sample_hash, weight, total_weight);
    println!("VRF Sortition test -> Selected: {}", selected);
}
