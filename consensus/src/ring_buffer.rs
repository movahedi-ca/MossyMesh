//! RAM-disk ring buffer with LRU eviction for Ephemeral Data Availability.
//!
//! Edge nodes keep a strict capacity bound (bytes and/or entry count) so the
//! active ledger + ephemeral DA stay within the 10 MB RAM overhead budget.
//! Regional SSD hubs are out of scope here; this structure is the hot path only.

use std::collections::HashMap;
use std::hash::Hash;

/// Default soft cap aligned with MossyMesh 10 MB ledger overhead budget.
pub const DEFAULT_MAX_BYTES: usize = 10_000_000;

/// Entry stored in the ephemeral DA ring.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DaEntry {
    pub key: Vec<u8>,
    pub value: Vec<u8>,
    /// Logical generation / insertion stamp (monotone).
    pub stamp: u64,
}

impl DaEntry {
    pub fn byte_size(&self) -> usize {
        self.key.len() + self.value.len() + std::mem::size_of::<u64>()
    }
}

/// Capacity-bounded LRU ring buffer for ephemeral data availability.
///
/// - `put` inserts or updates; promotes key to most-recently-used.
/// - `get` / `touch` promote on access (true LRU).
/// - Eviction removes least-recently-used entries until under capacity.
#[derive(Clone, Debug)]
pub struct LruRingBuffer {
    /// Max number of entries (0 = unlimited by count).
    max_entries: usize,
    /// Max total payload bytes (0 = unlimited by bytes).
    max_bytes: usize,
    /// key → value
    map: HashMap<Vec<u8>, Vec<u8>>,
    /// LRU order: front = least recently used, back = most recently used.
    order: Vec<Vec<u8>>,
    /// Approximate stored payload bytes (keys + values).
    bytes: usize,
    /// Monotone stamp for debugging / DA proofs.
    next_stamp: u64,
    /// Number of entries evicted over the lifetime of this buffer.
    evictions: u64,
}

impl LruRingBuffer {
    pub fn new(max_entries: usize, max_bytes: usize) -> Self {
        Self {
            max_entries,
            max_bytes,
            map: HashMap::new(),
            order: Vec::new(),
            bytes: 0,
            next_stamp: 1,
            evictions: 0,
        }
    }

    /// Ring sized only by entry count.
    pub fn with_capacity(max_entries: usize) -> Self {
        Self::new(max_entries, 0)
    }

    /// Ring sized by a byte budget (e.g. 10 MB).
    pub fn with_byte_budget(max_bytes: usize) -> Self {
        Self::new(0, max_bytes)
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn bytes_used(&self) -> usize {
        self.bytes
    }

    pub fn max_entries(&self) -> usize {
        self.max_entries
    }

    pub fn max_bytes(&self) -> usize {
        self.max_bytes
    }

    pub fn evictions(&self) -> u64 {
        self.evictions
    }

    pub fn contains(&self, key: &[u8]) -> bool {
        self.map.contains_key(key)
    }

    /// Insert or update. Returns list of keys that were LRU-evicted.
    pub fn put(&mut self, key: impl Into<Vec<u8>>, value: impl Into<Vec<u8>>) -> Vec<Vec<u8>> {
        let key = key.into();
        let value = value.into();
        let new_size = key.len() + value.len();

        if let Some(old) = self.map.get(&key) {
            self.bytes = self.bytes.saturating_sub(key.len() + old.len());
            self.map.insert(key.clone(), value);
            self.bytes = self.bytes.saturating_add(new_size);
            self.promote(&key);
        } else {
            self.map.insert(key.clone(), value);
            self.order.push(key);
            self.bytes = self.bytes.saturating_add(new_size);
        }

        self.next_stamp = self.next_stamp.saturating_add(1);
        self.evict_if_needed()
    }

    /// Get value and promote to MRU. Returns None if missing.
    pub fn get(&mut self, key: &[u8]) -> Option<&[u8]> {
        if self.map.contains_key(key) {
            self.promote(key);
            self.map.get(key).map(|v| v.as_slice())
        } else {
            None
        }
    }

    /// Peek without promoting (no LRU update).
    pub fn peek(&self, key: &[u8]) -> Option<&[u8]> {
        self.map.get(key).map(|v| v.as_slice())
    }

    /// Remove a key if present.
    pub fn remove(&mut self, key: &[u8]) -> Option<Vec<u8>> {
        if let Some(val) = self.map.remove(key) {
            self.bytes = self.bytes.saturating_sub(key.len() + val.len());
            self.order.retain(|k| k.as_slice() != key);
            Some(val)
        } else {
            None
        }
    }

    /// Keys from LRU (front) to MRU (back).
    pub fn keys_lru_to_mru(&self) -> impl Iterator<Item = &[u8]> {
        self.order.iter().map(|k| k.as_slice())
    }

    /// Least-recently-used key, if any.
    pub fn lru_key(&self) -> Option<&[u8]> {
        self.order.first().map(|k| k.as_slice())
    }

    /// Most-recently-used key, if any.
    pub fn mru_key(&self) -> Option<&[u8]> {
        self.order.last().map(|k| k.as_slice())
    }

    fn promote(&mut self, key: &[u8]) {
        if let Some(pos) = self.order.iter().position(|k| k.as_slice() == key) {
            let k = self.order.remove(pos);
            self.order.push(k);
        }
    }

    fn over_capacity(&self) -> bool {
        let over_count = self.max_entries > 0 && self.map.len() > self.max_entries;
        let over_bytes = self.max_bytes > 0 && self.bytes > self.max_bytes;
        over_count || over_bytes
    }

    fn evict_if_needed(&mut self) -> Vec<Vec<u8>> {
        let mut evicted = Vec::new();
        while self.over_capacity() && !self.order.is_empty() {
            let key = self.order.remove(0);
            if let Some(val) = self.map.remove(&key) {
                self.bytes = self.bytes.saturating_sub(key.len() + val.len());
                self.evictions += 1;
                evicted.push(key);
            }
        }
        evicted
    }
}

/// Typed wrapper when keys are hashable owned types (e.g. content-addressed CIDs as strings).
#[derive(Clone, Debug)]
pub struct TypedLruRing<K, V>
where
    K: Clone + Eq + Hash,
{
    max_entries: usize,
    map: HashMap<K, V>,
    order: Vec<K>,
    evictions: u64,
}

impl<K, V> TypedLruRing<K, V>
where
    K: Clone + Eq + Hash,
{
    pub fn new(max_entries: usize) -> Self {
        Self {
            max_entries,
            map: HashMap::new(),
            order: Vec::new(),
            evictions: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn evictions(&self) -> u64 {
        self.evictions
    }

    pub fn put(&mut self, key: K, value: V) -> Option<K> {
        if self.map.contains_key(&key) {
            self.map.insert(key.clone(), value);
            self.promote(&key);
            return None;
        }
        self.map.insert(key.clone(), value);
        self.order.push(key);
        self.evict_one()
    }

    pub fn get(&mut self, key: &K) -> Option<&V> {
        if self.map.contains_key(key) {
            self.promote(key);
            self.map.get(key)
        } else {
            None
        }
    }

    pub fn peek(&self, key: &K) -> Option<&V> {
        self.map.get(key)
    }

    pub fn contains(&self, key: &K) -> bool {
        self.map.contains_key(key)
    }

    fn promote(&mut self, key: &K) {
        if let Some(pos) = self.order.iter().position(|k| k == key) {
            let k = self.order.remove(pos);
            self.order.push(k);
        }
    }

    fn evict_one(&mut self) -> Option<K> {
        if self.max_entries > 0 && self.map.len() > self.max_entries {
            let key = self.order.remove(0);
            self.map.remove(&key);
            self.evictions += 1;
            Some(key)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_evicts_lru_on_overflow() {
        let mut ring = LruRingBuffer::with_capacity(3);
        ring.put(b"a", b"1");
        ring.put(b"b", b"2");
        ring.put(b"c", b"3");
        assert_eq!(ring.len(), 3);

        // Access "a" so it becomes MRU; "b" is now LRU.
        assert_eq!(ring.get(b"a"), Some(&b"1"[..]));
        assert_eq!(ring.lru_key(), Some(&b"b"[..]));

        let evicted = ring.put(b"d", b"4");
        assert_eq!(evicted, vec![b"b".to_vec()]);
        assert!(!ring.contains(b"b"));
        assert!(ring.contains(b"a"));
        assert!(ring.contains(b"c"));
        assert!(ring.contains(b"d"));
        assert_eq!(ring.len(), 3);
        assert_eq!(ring.evictions(), 1);
    }

    #[test]
    fn ring_byte_budget_eviction() {
        let mut ring = LruRingBuffer::with_byte_budget(20);
        // each entry: key 1 + value 8 = 9 bytes
        ring.put(b"1", b"aaaaaaaa");
        ring.put(b"2", b"bbbbbbbb");
        assert!(ring.bytes_used() <= 20);
        ring.put(b"3", b"cccccccc");
        // must have evicted at least one to stay under budget
        assert!(ring.bytes_used() <= 20);
        assert!(ring.evictions() >= 1);
        assert!(ring.len() < 3 || ring.bytes_used() <= 20);
    }

    #[test]
    fn update_does_not_grow_count() {
        let mut ring = LruRingBuffer::with_capacity(2);
        ring.put(b"k", b"v1");
        ring.put(b"k", b"v2");
        assert_eq!(ring.len(), 1);
        assert_eq!(ring.peek(b"k"), Some(&b"v2"[..]));
        assert_eq!(ring.evictions(), 0);
    }

    #[test]
    fn typed_lru_eviction() {
        let mut ring: TypedLruRing<String, u32> = TypedLruRing::new(2);
        assert!(ring.put("a".into(), 1).is_none());
        assert!(ring.put("b".into(), 2).is_none());
        let evicted = ring.put("c".into(), 3);
        assert_eq!(evicted.as_deref(), Some("a"));
        assert!(!ring.contains(&"a".into()));
        assert_eq!(ring.get(&"b".into()), Some(&2));
    }
}
