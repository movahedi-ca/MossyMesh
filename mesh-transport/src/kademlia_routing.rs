//! Advanced DHT pathfinding and nearest-neighbor logic.
//! Math Problem 2: Kademlia XOR Metric Distance
//! DOC 6: The DHT relies entirely on the XOR mathematical distance metric to map nodes without DNS.

/// Computes the XOR logical distance between two 256-bit PeerIDs.
/// The distance metric determines the routing table buckets in Kademlia.
/// DOC 7: XOR ensures symmetry: d(A,B) == d(B,A).
pub fn xor_distance(node_a: &[u8; 32], node_b: &[u8; 32]) -> [u8; 32] {
    let mut distance = [0u8; 32];
    for i in 0..32 {
        distance[i] = node_a[i] ^ node_b[i];
    }
    distance
}

/// Helper function to calculate the leading zeros of the XOR distance.
/// This determines which bucket the node belongs to.
/// DOC 8: By counting leading zeros, we mathematically partition the 256-bit keyspace into exactly 256 buckets.
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

pub struct RoutingTable {
    pub buckets: Vec<Vec<[u8; 32]>>,
}

impl RoutingTable {
    pub fn new() -> Self {
        // 256 buckets for 256-bit space
        RoutingTable {
            buckets: vec![Vec::new(); 256],
        }
    }

    /// Insert a node into the appropriate k-bucket.
    /// Kademlia constant k = 20.
    /// DOC 9: The k=20 limit ensures maximum network resilience while preventing unbounded RAM growth per bucket.
    pub fn insert_node(&mut self, local_id: &[u8; 32], remote_id: &[u8; 32]) {
        let dist = xor_distance(local_id, remote_id);
        let bucket_idx = calculate_bucket_index(&dist);
        
        let bucket = &mut self.buckets[bucket_idx];
        
        // Eviction / update logic
        if let Some(pos) = bucket.iter().position(|id| id == remote_id) {
            // Node exists, move to tail (most recently seen)
            let node = bucket.remove(pos);
            bucket.push(node);
        } else {
            if bucket.len() < 20 {
                // Space available, insert at tail
                bucket.push(remote_id.clone());
            } else {
                // Bucket full (k=20 limit reached). 
                // In full Kademlia, we'd ping the head (oldest). If it responds, drop new.
                // If it fails, drop head and push new. 
                // For Phase 1 constraint mapping, we simulate dropping the new node.
                println!("Bucket {} full, eviction policy triggered.", bucket_idx);
            }
        }
    }
}

/// DOC 10: Recursive node lookup stub for $\mathcal{O}(\log N)$ resolution.
pub fn find_node(target: &[u8; 32]) -> Option<[u8; 32]> {
    // Stub
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_xor_symmetry() {
        let a = [0b10101010; 32];
        let b = [0b01010101; 32];
        let dist_ab = xor_distance(&a, &b);
        let dist_ba = xor_distance(&b, &a);
        
        // Proof: Distance is symmetric.
        assert_eq!(dist_ab, dist_ba);
        
        // Proof: Distance to self is 0.
        let dist_aa = xor_distance(&a, &a);
        assert_eq!(dist_aa, [0u8; 32]);
    }

    #[test]
    fn test_bucket_index() {
        let mut dist = [0u8; 32];
        dist[31] = 1; // Least significant bit set (distance 1)
        
        let bucket = calculate_bucket_index(&dist);
        assert_eq!(bucket, 0); // Should be in bucket 0

        let mut dist2 = [0u8; 32];
        dist2[0] = 0b10000000; // Most significant bit set
        let bucket2 = calculate_bucket_index(&dist2);
        assert_eq!(bucket2, 255); // Should be in highest bucket (255)
    }
}
