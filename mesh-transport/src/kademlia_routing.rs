//! Advanced DHT pathfinding and nearest-neighbor logic.
//! Math Problem 2: Kademlia XOR Metric Distance
//! DOC 6: The DHT relies entirely on the XOR mathematical distance metric to map nodes without DNS.
//!
//! Pure-Rust routing table (k=20) with deterministic iterative `find_node`.
//! Complements the libp2p Kademlia behaviour wired in [`crate::network`].

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

/// Kademlia replication / bucket capacity parameter.
pub const K: usize = 20;

/// Parallelism factor (α) for iterative lookups.
pub const ALPHA: usize = 3;

/// 256-bit node identifier (PeerID / destination hash).
pub type NodeId = [u8; 32];

/// Contact entry stored in a k-bucket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeContact {
    pub id: NodeId,
    /// Opaque multiaddr / endpoint (e.g. `/ip4/…/udp/…` or mesh handle).
    pub endpoint: String,
    /// Monotonic last-seen tick for LRU ordering (tail = most recently seen).
    pub last_seen: u64,
}

impl NodeContact {
    pub fn new(id: NodeId, endpoint: impl Into<String>) -> Self {
        Self {
            id,
            endpoint: endpoint.into(),
            last_seen: 0,
        }
    }
}

/// Computes the XOR logical distance between two 256-bit PeerIDs.
/// DOC 7: XOR ensures symmetry: d(A,B) == d(B,A).
pub fn xor_distance(node_a: &NodeId, node_b: &NodeId) -> NodeId {
    let mut distance = [0u8; 32];
    for i in 0..32 {
        distance[i] = node_a[i] ^ node_b[i];
    }
    distance
}

/// Compare two XOR distances as big-endian unsigned integers.
/// Returns `Ordering::Less` if `a` is closer (smaller distance).
pub fn cmp_distance(a: &NodeId, b: &NodeId) -> Ordering {
    a.cmp(b)
}

/// True if `candidate` is strictly closer to `target` than `reference`.
pub fn is_closer(target: &NodeId, candidate: &NodeId, reference: &NodeId) -> bool {
    let d_cand = xor_distance(target, candidate);
    let d_ref = xor_distance(target, reference);
    cmp_distance(&d_cand, &d_ref) == Ordering::Less
}

/// Leading-zero count of the XOR distance → bucket index.
/// DOC 8: partitions the 256-bit keyspace into 256 buckets (0..=255).
pub fn calculate_bucket_index(distance: &NodeId) -> usize {
    let mut leading_zeros = 0usize;
    for &byte in distance.iter() {
        if byte == 0 {
            leading_zeros += 8;
        } else {
            leading_zeros += byte.leading_zeros() as usize;
            break;
        }
    }
    if leading_zeros >= 256 {
        0 // distance 0 → same node; treat as bucket 0
    } else {
        256 - leading_zeros - 1
    }
}

/// Bucket index for `remote` relative to `local`.
pub fn bucket_for(local_id: &NodeId, remote_id: &NodeId) -> usize {
    calculate_bucket_index(&xor_distance(local_id, remote_id))
}

/// Optional liveness probe used when a k-bucket is full (head eviction).
/// Return `true` if the oldest contact is still alive (keep it, drop newcomer).
pub trait LivenessProbe {
    fn is_alive(&self, contact: &NodeContact) -> bool;
}

/// Default probe: always treat existing head as alive (conservative drop-new).
#[derive(Debug, Default, Clone, Copy)]
pub struct AlwaysAlive;

impl LivenessProbe for AlwaysAlive {
    fn is_alive(&self, _contact: &NodeContact) -> bool {
        true
    }
}

/// Probe that always reports dead (useful in tests for eviction).
#[derive(Debug, Default, Clone, Copy)]
pub struct AlwaysDead;

impl LivenessProbe for AlwaysDead {
    fn is_alive(&self, _contact: &NodeContact) -> bool {
        false
    }
}

/// Full Kademlia routing table with k=20 buckets.
#[derive(Debug, Clone)]
pub struct RoutingTable {
    pub local_id: NodeId,
    /// 256 buckets; each holds at most `K` contacts (head=oldest, tail=newest).
    pub buckets: Vec<Vec<NodeContact>>,
    /// Clock for `last_seen` stamps.
    pub tick: u64,
    /// Max contacts per bucket (k-parameter).
    pub k: usize,
}

impl RoutingTable {
    pub fn new(local_id: NodeId) -> Self {
        Self::with_k(local_id, K)
    }

    pub fn with_k(local_id: NodeId, k: usize) -> Self {
        Self {
            local_id,
            buckets: (0..256).map(|_| Vec::with_capacity(k.min(K))).collect(),
            tick: 0,
            k,
        }
    }

    fn next_tick(&mut self) -> u64 {
        self.tick = self.tick.wrapping_add(1);
        self.tick
    }

    /// Insert or refresh a node contact. Returns whether the table changed.
    pub fn insert(&mut self, contact: NodeContact) -> bool {
        self.insert_with_probe(contact, &AlwaysAlive)
    }

    /// Insert with a custom liveness probe for full-bucket eviction.
    pub fn insert_with_probe<P: LivenessProbe>(
        &mut self,
        mut contact: NodeContact,
        probe: &P,
    ) -> bool {
        if contact.id == self.local_id {
            return false;
        }

        let idx = bucket_for(&self.local_id, &contact.id);
        contact.last_seen = self.next_tick();
        let bucket = &mut self.buckets[idx];

        if let Some(pos) = bucket.iter().position(|c| c.id == contact.id) {
            // Move to tail (most recently seen).
            let mut existing = bucket.remove(pos);
            existing.endpoint = contact.endpoint;
            existing.last_seen = contact.last_seen;
            bucket.push(existing);
            return true;
        }

        if bucket.len() < self.k {
            bucket.push(contact);
            return true;
        }

        // Bucket full: ping head (oldest). If dead → evict and append newcomer.
        // If alive → drop newcomer (replacement-cache omitted in Phase-1).
        let head_alive = probe.is_alive(&bucket[0]);
        if !head_alive {
            bucket.remove(0);
            bucket.push(contact);
            true
        } else {
            false
        }
    }

    /// Backward-compatible insert by raw id (endpoint empty).
    pub fn insert_node(&mut self, local_id: &NodeId, remote_id: &NodeId) {
        // If caller passes a different local_id, still bucket relative to our table root.
        let _ = local_id;
        self.insert(NodeContact::new(*remote_id, String::new()));
    }

    /// Number of known contacts.
    pub fn len(&self) -> usize {
        self.buckets.iter().map(|b| b.len()).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Lookup a contact by id.
    pub fn get(&self, id: &NodeId) -> Option<&NodeContact> {
        let idx = bucket_for(&self.local_id, id);
        self.buckets[idx].iter().find(|c| c.id == *id)
    }

    /// Return up to `count` contacts closest to `target` (XOR metric).
    pub fn closest(&self, target: &NodeId, count: usize) -> Vec<NodeContact> {
        let mut all: Vec<NodeContact> = self
            .buckets
            .iter()
            .flat_map(|b| b.iter().cloned())
            .collect();
        all.sort_by(|a, b| {
            let da = xor_distance(target, &a.id);
            let db = xor_distance(target, &b.id);
            cmp_distance(&da, &db)
        });
        all.truncate(count);
        all
    }

    /// All contacts (arbitrary order).
    pub fn all_contacts(&self) -> Vec<NodeContact> {
        self.buckets.iter().flat_map(|b| b.iter().cloned()).collect()
    }
}

impl Default for RoutingTable {
    fn default() -> Self {
        Self::new([0u8; 32])
    }
}

/// Trait for answering FIND_NODE RPCs during iterative lookup.
/// Production code queries remote peers; tests inject a fixed map.
pub trait FindNodeRpc {
    /// Peer's response: up to k contacts closest to `target` that it knows.
    fn find_node(&self, peer: &NodeId, target: &NodeId) -> Vec<NodeContact>;
}

/// Local-only RPC: answers from a single shared table (deterministic offline sim).
#[derive(Debug)]
pub struct LocalTableRpc<'a> {
    pub tables: HashMap<NodeId, &'a RoutingTable>,
}

impl<'a> LocalTableRpc<'a> {
    pub fn new() -> Self {
        Self {
            tables: HashMap::new(),
        }
    }

    pub fn register(&mut self, table: &'a RoutingTable) {
        self.tables.insert(table.local_id, table);
    }
}

impl<'a> Default for LocalTableRpc<'a> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> FindNodeRpc for LocalTableRpc<'a> {
    fn find_node(&self, peer: &NodeId, target: &NodeId) -> Vec<NodeContact> {
        match self.tables.get(peer) {
            Some(table) => table.closest(target, table.k),
            None => Vec::new(),
        }
    }
}

/// Result of an iterative FIND_NODE lookup.
#[derive(Debug, Clone)]
pub struct FindNodeResult {
    /// Up to k closest contacts discovered.
    pub closest: Vec<NodeContact>,
    /// True if `target` itself was found among responses.
    pub found_exact: bool,
    /// Number of FIND_NODE RPCs issued.
    pub rounds: usize,
}

/// Deterministic iterative `find_node` (Kademlia §2.3).
///
/// Starts from the caller's routing table, queries α closest unqueried peers
/// each round, and terminates when a full round yields no closer nodes.
pub fn iterative_find_node<R: FindNodeRpc>(
    local: &RoutingTable,
    target: &NodeId,
    rpc: &R,
) -> FindNodeResult {
    let k = local.k;
    let mut shortlist: HashMap<NodeId, NodeContact> = HashMap::new();
    for c in local.closest(target, k) {
        shortlist.insert(c.id, c);
    }

    // If we already know the target, include it.
    if let Some(c) = local.get(target) {
        shortlist.insert(c.id, c.clone());
    }

    let mut queried: HashSet<NodeId> = HashSet::new();
    queried.insert(local.local_id);

    let mut rounds = 0usize;
    let mut found_exact = shortlist.contains_key(target);

    loop {
        rounds += 1;

        // Select α closest unqueried contacts.
        let mut candidates: Vec<NodeContact> = shortlist
            .values()
            .filter(|c| !queried.contains(&c.id))
            .cloned()
            .collect();
        candidates.sort_by(|a, b| {
            let da = xor_distance(target, &a.id);
            let db = xor_distance(target, &b.id);
            cmp_distance(&da, &db)
        });
        candidates.truncate(ALPHA);

        if candidates.is_empty() {
            break;
        }

        let prev_best = shortlist
            .values()
            .map(|c| xor_distance(target, &c.id))
            .min()
            .unwrap_or([0xff; 32]);

        let mut improved = false;
        for peer in &candidates {
            queried.insert(peer.id);
            let replies = rpc.find_node(&peer.id, target);
            for contact in replies {
                if contact.id == local.local_id {
                    continue;
                }
                if contact.id == *target {
                    found_exact = true;
                }
                shortlist.entry(contact.id).or_insert(contact);
            }
        }

        // Keep only k closest in shortlist.
        let mut ranked: Vec<NodeContact> = shortlist.values().cloned().collect();
        ranked.sort_by(|a, b| {
            let da = xor_distance(target, &a.id);
            let db = xor_distance(target, &b.id);
            cmp_distance(&da, &db)
        });
        ranked.truncate(k);
        shortlist = ranked.iter().map(|c| (c.id, c.clone())).collect();

        let new_best = shortlist
            .values()
            .map(|c| xor_distance(target, &c.id))
            .min()
            .unwrap_or([0xff; 32]);

        if cmp_distance(&new_best, &prev_best) == Ordering::Less {
            improved = true;
        }

        // Terminate when a full α-query round did not improve the closest distance
        // and every remaining shortlist member has been queried, or no improvement.
        let unqueried_left = shortlist.keys().any(|id| !queried.contains(id));
        if !improved && !unqueried_left {
            break;
        }
        // Safety cap for adversarial / incomplete graphs.
        if rounds > 256 {
            break;
        }
        // If no improvement and we already queried this batch with no better shortlist,
        // continue only while unqueried shortlist members remain (standard Kademlia).
        if !improved && unqueried_left {
            continue;
        }
        if !improved {
            break;
        }
    }

    let mut closest: Vec<NodeContact> = shortlist.into_values().collect();
    closest.sort_by(|a, b| {
        let da = xor_distance(target, &a.id);
        let db = xor_distance(target, &b.id);
        cmp_distance(&da, &db)
    });
    closest.truncate(k);

    FindNodeResult {
        closest,
        found_exact,
        rounds,
    }
}

/// Convenience stub used by older call sites: lookup target in a single table.
pub fn find_node(target: &NodeId) -> Option<NodeId> {
    let _ = target;
    None
}

/// Lookup `target` against one local routing table (no multi-hop RPC).
pub fn find_node_local(table: &RoutingTable, target: &NodeId) -> Option<NodeContact> {
    table.get(target).cloned().or_else(|| {
        let closest = table.closest(target, 1);
        closest.into_iter().next()
    })
}

/// Initialize module (daemon boot path).
pub fn init_kademlia_routing() {
    println!("Initializing Kademlia DHT for offline identity-based routing (k={}).", K);
    let local = node_id_from_u8(0xAA);
    let mut table = RoutingTable::new(local);
    for i in 1u8..=25 {
        let id = node_id_from_u8(i);
        table.insert(NodeContact::new(id, format!("mesh://node/{i}")));
    }
    let target = node_id_from_u8(7);
    let hit = find_node_local(&table, &target);
    println!(
        "Kademlia local table: {} contacts; find_node(7) => {:?}",
        table.len(),
        hit.as_ref().map(|c| c.endpoint.as_str())
    );
    let a = [0b10101010; 32];
    let b = [0b01010101; 32];
    let dist = xor_distance(&a, &b);
    let bucket = calculate_bucket_index(&dist);
    println!("Kademlia XOR self-check bucket: {}", bucket);
}

/// Deterministic helper for demos/tests: fill first byte, rest zero.
pub fn node_id_from_u8(v: u8) -> NodeId {
    let mut id = [0u8; 32];
    id[0] = v;
    id
}

/// Deterministic helper: encode a u64 into the last 8 bytes (big-endian).
pub fn node_id_from_u64(v: u64) -> NodeId {
    let mut id = [0u8; 32];
    id[24..32].copy_from_slice(&v.to_be_bytes());
    id
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

        assert_eq!(dist_ab, dist_ba);
        assert_eq!(xor_distance(&a, &a), [0u8; 32]);
    }

    #[test]
    fn test_xor_distance_bytes() {
        let mut a = [0u8; 32];
        let mut b = [0u8; 32];
        a[0] = 0xF0;
        b[0] = 0x0F;
        let d = xor_distance(&a, &b);
        assert_eq!(d[0], 0xFF);
        for i in 1..32 {
            assert_eq!(d[i], 0);
        }
    }

    #[test]
    fn test_bucket_index() {
        let mut dist = [0u8; 32];
        dist[31] = 1; // LSB set → distance 1
        assert_eq!(calculate_bucket_index(&dist), 0);

        let mut dist2 = [0u8; 32];
        dist2[0] = 0b10000000; // MSB set
        assert_eq!(calculate_bucket_index(&dist2), 255);

        // Distance 0 → bucket 0
        assert_eq!(calculate_bucket_index(&[0u8; 32]), 0);
    }

    #[test]
    fn test_is_closer() {
        let target = node_id_from_u8(0x10);
        let near = node_id_from_u8(0x11); // xor = 0x01
        let far = node_id_from_u8(0xF0); // xor = 0xE0
        assert!(is_closer(&target, &near, &far));
        assert!(!is_closer(&target, &far, &near));
    }

    #[test]
    fn test_k_bucket_capacity() {
        let local = node_id_from_u8(0);
        let mut table = RoutingTable::with_k(local, K);

        // Force many nodes into the same high-distance bucket by flipping MSB.
        let mut inserted = 0usize;
        for i in 1u16..=40 {
            let mut id = [0u8; 32];
            id[0] = 0x80;
            id[1] = (i & 0xFF) as u8;
            id[2] = (i >> 8) as u8;
            if table.insert(NodeContact::new(id, format!("ep-{i}"))) {
                inserted += 1;
            }
        }
        // At most k contacts in that single bucket.
        let idx = bucket_for(&local, &[0x80, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        assert!(table.buckets[idx].len() <= K);
        assert_eq!(table.buckets[idx].len(), K);
        assert!(inserted >= K);
    }

    #[test]
    fn test_eviction_when_head_dead() {
        let local = node_id_from_u8(0);
        let mut table = RoutingTable::with_k(local, 2);

        let mut a = [0u8; 32];
        a[0] = 0x80;
        a[1] = 1;
        let mut b = [0u8; 32];
        b[0] = 0x80;
        b[1] = 2;
        let mut c = [0u8; 32];
        c[0] = 0x80;
        c[1] = 3;

        assert!(table.insert(NodeContact::new(a, "a")));
        assert!(table.insert(NodeContact::new(b, "b")));
        // Full; AlwaysDead probe should evict head (a) and accept c.
        assert!(table.insert_with_probe(NodeContact::new(c, "c"), &AlwaysDead));
        assert!(table.get(&a).is_none());
        assert!(table.get(&c).is_some());
    }

    #[test]
    fn test_closest_ordering() {
        let local = node_id_from_u64(100);
        let mut table = RoutingTable::new(local);
        for v in [1u64, 50, 90, 95, 200, 300] {
            table.insert(NodeContact::new(node_id_from_u64(v), format!("n{v}")));
        }
        let target = node_id_from_u64(100);
        let closest = table.closest(&target, 3);
        assert_eq!(closest.len(), 3);
        // Distances must be non-decreasing.
        for w in closest.windows(2) {
            let d0 = xor_distance(&target, &w[0].id);
            let d1 = xor_distance(&target, &w[1].id);
            assert!(cmp_distance(&d0, &d1) != Ordering::Greater);
        }
    }

    #[test]
    fn test_iterative_find_node_deterministic() {
        // Build a small mesh of routing tables that know their neighbors.
        let ids: Vec<NodeId> = (0..16u64).map(node_id_from_u64).collect();
        let mut tables: Vec<RoutingTable> = ids
            .iter()
            .map(|id| RoutingTable::new(*id))
            .collect();

        // Ring + skip links so FIND_NODE can walk toward the target.
        for i in 0..tables.len() {
            for offset in [1usize, 2, 4] {
                let j = (i + offset) % tables.len();
                let contact = NodeContact::new(ids[j], format!("node-{j}"));
                tables[i].insert(contact);
            }
            // Also seed a few reverse links.
            let j = (i + tables.len() - 1) % tables.len();
            tables[i].insert(NodeContact::new(ids[j], format!("node-{j}")));
        }

        // Need a second phase of references for LocalTableRpc lifetimes:
        // rebuild a owned map of tables and register.
        let tables = tables; // own
        let origin = &tables[0];
        let target = ids[9];

        let mut rpc = LocalTableRpc::new();
        for t in &tables {
            rpc.register(t);
        }

        let result = iterative_find_node(origin, &target, &rpc);
        assert!(
            result.found_exact || result.closest.iter().any(|c| c.id == target),
            "expected to discover target via iterative lookup; closest={:?}",
            result.closest.iter().map(|c| c.endpoint.as_str()).collect::<Vec<_>>()
        );
        assert!(!result.closest.is_empty());
        assert!(result.rounds >= 1);

        // Determinism: second run yields identical closest ids.
        let result2 = iterative_find_node(origin, &target, &rpc);
        let ids1: Vec<_> = result.closest.iter().map(|c| c.id).collect();
        let ids2: Vec<_> = result2.closest.iter().map(|c| c.id).collect();
        assert_eq!(ids1, ids2);
        assert_eq!(result.rounds, result2.rounds);
    }

    #[test]
    fn test_refresh_moves_to_tail() {
        let local = node_id_from_u8(0);
        let mut table = RoutingTable::new(local);
        let id = node_id_from_u8(0x80);
        table.insert(NodeContact::new(id, "v1"));
        table.insert(NodeContact::new(node_id_from_u8(0x81), "other"));
        table.insert(NodeContact::new(id, "v2"));
        let bucket = table
            .buckets
            .iter()
            .find(|b| b.iter().any(|c| c.id == id))
            .unwrap();
        let pos = bucket.iter().position(|c| c.id == id).unwrap();
        assert_eq!(pos, bucket.len() - 1);
        assert_eq!(bucket[pos].endpoint, "v2");
    }
}
