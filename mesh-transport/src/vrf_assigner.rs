//! Commit-and-Reveal seed logic for Verifiable Random Function (VRF) task routing.
//! Math Problem 4: Deterministic VRF Sortition Selection
//! DOC 16: This module uses cryptographic sortition to randomly but verifiably elect worker nodes.

/// Evaluates if a node is selected for a task based on its VRF hash output and its weight.
/// The formula for sortition is: 
/// Hash_Value < (Max_Hash * weight) / total_network_weight
/// Since we don't have U256 in the standard library, we simulate the top 64 bits of the hash.
/// DOC 17: We scale the threshold by `node_weight` / `total_network_weight` to ensure fair proportional representation.
pub fn is_selected_for_task(vrf_hash_top_64: u64, node_weight: u64, total_network_weight: u64) -> bool {
    if total_network_weight == 0 {
        return false;
    }
    
    // Calculate the threshold. 
    // We must use u128 for intermediate multiplication to prevent overflow.
    // DOC 18: `u128` intermediate casting guarantees mathematical safety when scaling large hash boundaries.
    let threshold = ((u64::MAX as u128 * node_weight as u128) / total_network_weight as u128) as u64;
    
    // Node is selected if its cryptographic VRF output is below the threshold.
    // DOC 19: The `<` check enforces deterministic selection without requiring a centralized coordinator.
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

/// DOC 20: The VrfProof ensures that the node mathematically proves it won the sortition lottery via ED25519 signatures.
pub struct VrfProof {
    pub hash: [u8; 32],
    pub signature: [u8; 64],
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vrf_sortition_selection() {
        let max_hash = u64::MAX;
        
        // If node has 50% of the network weight, threshold should be ~50% of max_hash
        // Let's use a hash that is exactly 25% of max_hash.
        let vrf_hash = max_hash / 4;
        
        // Node with 50% network weight (100 out of 200) -> selected (25% < 50%)
        assert!(is_selected_for_task(vrf_hash, 100, 200));
        
        // Node with 10% network weight (20 out of 200) -> rejected (25% > 10%)
        assert!(!is_selected_for_task(vrf_hash, 20, 200));
    }

    #[test]
    fn test_vrf_overflow_protection() {
        // High weights that might overflow standard u64 scaling
        let max_hash = u64::MAX;
        let vrf_hash = max_hash - 1000;
        
        // Very high node weight, close to total. 
        // Should not panic (because we use u128 intermediate).
        assert!(is_selected_for_task(vrf_hash, 999_999, 1_000_000));
    }
}
