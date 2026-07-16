//! Advanced DHT pathfinding and nearest-neighbor logic.
//! Math Problem 2: Kademlia XOR Metric Distance

/// Computes the XOR logical distance between two 256-bit PeerIDs.
/// The distance metric determines the routing table buckets in Kademlia.
pub fn xor_distance(node_a: &[u8; 32], node_b: &[u8; 32]) -> [u8; 32] {
    let mut distance = [0u8; 32];
    for i in 0..32 {
        distance[i] = node_a[i] ^ node_b[i];
    }
    distance
}

/// Helper function to calculate the leading zeros of the XOR distance.
/// This determines which bucket the node belongs to.
pub fn calculate_bucket_index(distance: &[u8; 32]) -> usize {
    let mut leading_zeros = 0;
    for &byte in distance.iter() {
        if byte == 0 {
            leading_zeros += 8;
        } else {
            leading_zeros += byte.leading_zeros() as usize;
            break;
        }
    }
    // For a 256-bit key space, there are 256 possible buckets (0 to 255)
    // The bucket index is (256 - leading_zeros - 1)
    if leading_zeros == 256 {
        0 // Distance is 0, same node
    } else {
        256 - leading_zeros - 1
    }
}

pub fn init_kademlia_routing() {
    println!("Initializing Kademlia DHT for offline identity-based routing.");
    let a = [0b10101010; 32];
    let b = [0b01010101; 32];
    let dist = xor_distance(&a, &b);
    let bucket = calculate_bucket_index(&dist);
    println!("Kademlia test XOR distance bucket: {}", bucket);
}
