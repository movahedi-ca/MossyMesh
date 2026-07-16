//! YATA/RGA-inspired CRDT for deterministic merge of disconnected mesh islands.
//!
//! Implements:
//! - Sequence (text) CRDT using an RGA tree (parent = left origin at insert)
//!   with concurrent siblings ordered by `ItemId` descending — converges
//!   regardless of integrate order
//! - LWW Map CRDT with `(logical_time, agent_id)` total order
//! - Binary op-log deltas for sync across islands
//!
//! Guarantees strong eventual consistency: concurrent divergent edits converge
//! to the same state after exchanging op logs (commutative merge).

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};

/// Stable identifier for a mesh agent / island replica.
pub type AgentId = u64;

/// Monotonic per-agent operation sequence number.
pub type Seq = u64;

/// Globally unique item / operation identifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ItemId {
    pub agent: AgentId,
    pub seq: Seq,
}

impl ItemId {
    pub fn new(agent: AgentId, seq: Seq) -> Self {
        Self { agent, seq }
    }
}

/// Logical clock tick used for LWW map conflict resolution.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct LogicalTime {
    pub wall: u64,
    pub agent: AgentId,
}

impl LogicalTime {
    pub fn new(wall: u64, agent: AgentId) -> Self {
        Self { wall, agent }
    }
}

/// A single character (or atom) in the RGA sequence tree.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeqItem {
    pub id: ItemId,
    /// Parent = left neighbor at insertion time (None = document root / start).
    pub parent: Option<ItemId>,
    pub content: char,
    pub deleted: bool,
}

/// Operations that can be exchanged as binary deltas between islands.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CrdtOp {
    /// Insert `content` after `parent` (RGA).
    Insert {
        id: ItemId,
        parent: Option<ItemId>,
        content: char,
    },
    /// Tombstone-delete an existing sequence item (`op_id` is unique; `target` is the insert).
    Delete { target: ItemId, op_id: ItemId },
    /// Last-writer-wins map put.
    MapSet {
        key: String,
        value: Vec<u8>,
        time: LogicalTime,
    },
    /// Last-writer-wins map remove (stores tombstone with time).
    MapDelete { key: String, time: LogicalTime },
}

/// Binary delta: ordered op log for island-to-island sync.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Delta {
    pub ops: Vec<CrdtOp>,
}

impl Delta {
    pub fn new(ops: Vec<CrdtOp>) -> Self {
        Self { ops }
    }

    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    /// Encode delta to a compact binary blob.
    pub fn encode(&self) -> Result<Vec<u8>, CrdtError> {
        bincode::serialize(self).map_err(|e| CrdtError::Encode(e.to_string()))
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, CrdtError> {
        bincode::deserialize(bytes).map_err(|e| CrdtError::Decode(e.to_string()))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CrdtError {
    Decode(String),
    Encode(String),
    UnknownItem(ItemId),
}

impl std::fmt::Display for CrdtError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CrdtError::Decode(m) => write!(f, "decode error: {m}"),
            CrdtError::Encode(m) => write!(f, "encode error: {m}"),
            CrdtError::UnknownItem(id) => write!(f, "unknown item {:?}", id),
        }
    }
}

impl std::error::Error for CrdtError {}

/// LWW register value stored in the map CRDT.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct LwwValue {
    value: Option<Vec<u8>>,
    time: LogicalTime,
}

/// Document CRDT: RGA sequence + LWW map, with full op log for deltas.
#[derive(Clone, Debug, Default)]
pub struct Doc {
    /// All sequence items keyed by insert id.
    items: HashMap<ItemId, SeqItem>,
    /// parent → children (unordered; sorted deterministically on materialize).
    children: HashMap<Option<ItemId>, Vec<ItemId>>,
    /// LWW map state.
    map: HashMap<String, LwwValue>,
    /// Applied insert ids (idempotent).
    applied_inserts: HashSet<ItemId>,
    /// Applied delete op ids (idempotent).
    applied_deletes: HashSet<ItemId>,
    /// Full causal op log for delta export.
    op_log: Vec<CrdtOp>,
    /// Per-agent highest integrated sequence number.
    vector: BTreeMap<AgentId, Seq>,
    /// Local agent identity.
    agent: AgentId,
    /// Next local sequence number.
    next_seq: Seq,
    /// Local wall-clock counter for LWW (monotone).
    next_wall: u64,
}

impl Doc {
    pub fn new(agent: AgentId) -> Self {
        Self {
            agent,
            next_seq: 1,
            next_wall: 1,
            ..Default::default()
        }
    }

    pub fn agent(&self) -> AgentId {
        self.agent
    }

    /// Visible text content (skips tombstones). Order is deterministic RGA traversal.
    pub fn text(&self) -> String {
        let mut out = String::new();
        self.walk(None, &mut |item| {
            if !item.deleted {
                out.push(item.content);
            }
        });
        out
    }

    /// Length of visible text.
    pub fn text_len(&self) -> usize {
        self.items.values().filter(|i| !i.deleted).count()
    }

    pub fn map_get(&self, key: &str) -> Option<&[u8]> {
        self.map
            .get(key)
            .and_then(|v| v.value.as_ref().map(|b| b.as_slice()))
    }

    pub fn map_keys(&self) -> impl Iterator<Item = &String> {
        self.map
            .iter()
            .filter(|(_, v)| v.value.is_some())
            .map(|(k, _)| k)
    }

    /// Version vector: highest seq seen per agent.
    pub fn version_vector(&self) -> &BTreeMap<AgentId, Seq> {
        &self.vector
    }

    pub fn op_log_len(&self) -> usize {
        self.op_log.len()
    }

    fn alloc_id(&mut self) -> ItemId {
        let id = ItemId::new(self.agent, self.next_seq);
        self.next_seq += 1;
        id
    }

    fn tick_wall(&mut self) -> LogicalTime {
        let t = LogicalTime::new(self.next_wall, self.agent);
        self.next_wall += 1;
        t
    }

    /// Depth-first RGA walk: children of each parent sorted by ItemId **descending**.
    /// Higher (agent, seq) appears closer to the parent (classic RGA: newer concurrent
    /// inserts sit immediately after the left neighbor). Deterministic → converges.
    fn walk(&self, parent: Option<ItemId>, f: &mut dyn FnMut(&SeqItem)) {
        let mut kids = self.children.get(&parent).cloned().unwrap_or_default();
        kids.sort_by(|a, b| b.cmp(a)); // descending ItemId
        for id in kids {
            if let Some(item) = self.items.get(&id) {
                f(item);
                self.walk(Some(id), f);
            }
        }
    }

    /// Visible items in document order.
    fn visible_ids(&self) -> Vec<ItemId> {
        let mut ids = Vec::new();
        self.walk(None, &mut |item| {
            if !item.deleted {
                ids.push(item.id);
            }
        });
        ids
    }

    /// Insert a character at a visible character index (0..=len).
    pub fn insert_char(&mut self, visible_index: usize, content: char) -> CrdtOp {
        let parent = if visible_index == 0 {
            None
        } else {
            let ids = self.visible_ids();
            ids.get(visible_index - 1).copied()
        };
        let id = self.alloc_id();
        let op = CrdtOp::Insert {
            id,
            parent,
            content,
        };
        self.integrate(op.clone());
        op
    }

    /// Insert a full string at a visible index; returns one op per char.
    pub fn insert_str(&mut self, mut visible_index: usize, s: &str) -> Vec<CrdtOp> {
        let mut ops = Vec::with_capacity(s.chars().count());
        for ch in s.chars() {
            ops.push(self.insert_char(visible_index, ch));
            visible_index += 1;
        }
        ops
    }

    /// Delete the visible character at index.
    pub fn delete_char(&mut self, visible_index: usize) -> Option<CrdtOp> {
        let ids = self.visible_ids();
        let target = *ids.get(visible_index)?;
        let op_id = self.alloc_id();
        let op = CrdtOp::Delete { target, op_id };
        self.integrate(op.clone());
        Some(op)
    }

    pub fn map_set(&mut self, key: impl Into<String>, value: impl Into<Vec<u8>>) -> CrdtOp {
        let time = self.tick_wall();
        let op = CrdtOp::MapSet {
            key: key.into(),
            value: value.into(),
            time,
        };
        self.integrate(op.clone());
        op
    }

    pub fn map_delete(&mut self, key: impl Into<String>) -> CrdtOp {
        let time = self.tick_wall();
        let op = CrdtOp::MapDelete {
            key: key.into(),
            time,
        };
        self.integrate(op.clone());
        op
    }

    /// Integrate a remote or local operation (idempotent).
    pub fn integrate(&mut self, op: CrdtOp) {
        match &op {
            CrdtOp::Insert { id, .. } => {
                if self.applied_inserts.contains(id) {
                    return;
                }
            }
            CrdtOp::Delete { op_id, .. } => {
                if self.applied_deletes.contains(op_id) {
                    return;
                }
            }
            CrdtOp::MapSet { key, time, .. } | CrdtOp::MapDelete { key, time } => {
                if let Some(existing) = self.map.get(key) {
                    if existing.time >= *time {
                        return;
                    }
                }
            }
        }

        match op.clone() {
            CrdtOp::Insert {
                id,
                parent,
                content,
            } => {
                let item = SeqItem {
                    id,
                    parent,
                    content,
                    deleted: false,
                };
                self.items.insert(id, item);
                self.children.entry(parent).or_default().push(id);
                self.applied_inserts.insert(id);
                self.note_vector(id);
            }
            CrdtOp::Delete { target, op_id } => {
                if let Some(item) = self.items.get_mut(&target) {
                    item.deleted = true;
                }
                self.applied_deletes.insert(op_id);
                self.note_vector(op_id);
            }
            CrdtOp::MapSet { key, value, time } => {
                self.map.insert(
                    key,
                    LwwValue {
                        value: Some(value),
                        time,
                    },
                );
                if time.wall >= self.next_wall {
                    self.next_wall = time.wall + 1;
                }
            }
            CrdtOp::MapDelete { key, time } => {
                self.map.insert(
                    key,
                    LwwValue {
                        value: None,
                        time,
                    },
                );
                if time.wall >= self.next_wall {
                    self.next_wall = time.wall + 1;
                }
            }
        }

        self.op_log.push(op);
    }

    fn note_vector(&mut self, id: ItemId) {
        let entry = self.vector.entry(id.agent).or_insert(0);
        if id.seq > *entry {
            *entry = id.seq;
        }
        if id.agent == self.agent && id.seq >= self.next_seq {
            self.next_seq = id.seq + 1;
        }
    }

    /// Merge remote document by integrating all remote ops not yet applied.
    /// Commutative and idempotent → concurrent islands converge.
    pub fn merge(&mut self, remote: &Doc) {
        for op in &remote.op_log {
            self.integrate(op.clone());
        }
    }

    /// Export ops that the remote has not yet seen (based on version vector).
    pub fn delta_since(&self, remote_vv: &BTreeMap<AgentId, Seq>) -> Delta {
        let mut ops = Vec::new();
        for op in &self.op_log {
            match op {
                CrdtOp::Insert { id, .. } => {
                    let seen = remote_vv.get(&id.agent).copied().unwrap_or(0);
                    if id.seq > seen {
                        ops.push(op.clone());
                    }
                }
                CrdtOp::Delete { op_id, .. } => {
                    let seen = remote_vv.get(&op_id.agent).copied().unwrap_or(0);
                    if op_id.seq > seen {
                        ops.push(op.clone());
                    }
                }
                CrdtOp::MapSet { .. } | CrdtOp::MapDelete { .. } => {
                    ops.push(op.clone());
                }
            }
        }
        Delta::new(compact_map_ops(ops))
    }

    /// Full state as a delta (all ops) for cold sync.
    pub fn full_delta(&self) -> Delta {
        Delta::new(self.op_log.clone())
    }

    /// Apply a binary delta from another island.
    pub fn apply_delta(&mut self, delta: &Delta) {
        for op in &delta.ops {
            self.integrate(op.clone());
        }
    }

    /// Apply binary-encoded delta bytes.
    pub fn apply_delta_bytes(&mut self, bytes: &[u8]) -> Result<(), CrdtError> {
        let delta = Delta::decode(bytes)?;
        self.apply_delta(&delta);
        Ok(())
    }
}

/// Compact map ops: later ops for same key supersede earlier ones in the delta list.
fn compact_map_ops(ops: Vec<CrdtOp>) -> Vec<CrdtOp> {
    let mut latest_map: HashMap<String, CrdtOp> = HashMap::new();
    let mut seq_ops = Vec::new();
    for op in ops {
        match &op {
            CrdtOp::MapSet { key, .. } | CrdtOp::MapDelete { key, .. } => {
                let replace = match latest_map.get(key) {
                    None => true,
                    Some(prev) => map_op_time(prev) < map_op_time(&op),
                };
                if replace {
                    latest_map.insert(key.clone(), op);
                }
            }
            _ => seq_ops.push(op),
        }
    }
    seq_ops.extend(latest_map.into_values());
    seq_ops
}

fn map_op_time(op: &CrdtOp) -> LogicalTime {
    match op {
        CrdtOp::MapSet { time, .. } | CrdtOp::MapDelete { time, .. } => *time,
        _ => LogicalTime::new(0, 0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concurrent_divergent_edits_converge() {
        let mut a = Doc::new(1);
        let mut b = Doc::new(2);

        a.insert_str(0, "Hello");
        b.insert_str(0, "World");

        a.map_set("turn", b"A");
        b.map_set("turn", b"B");
        a.map_set("game", b"chess");
        b.map_set("mode", b"blitz");

        let mut a2 = a.clone();
        let mut b2 = b.clone();
        a2.merge(&b);
        b2.merge(&a);

        assert_eq!(
            a2.text(),
            b2.text(),
            "text must converge: a={:?} b={:?}",
            a2.text(),
            b2.text()
        );
        assert_eq!(a2.map_get("game"), b2.map_get("game"));
        assert_eq!(a2.map_get("mode"), b2.map_get("mode"));
        assert_eq!(a2.map_get("turn"), b2.map_get("turn"));

        let mut c = Doc::new(3);
        let bytes = a2.full_delta().encode().expect("encode");
        c.apply_delta_bytes(&bytes).expect("decode");
        assert_eq!(c.text(), a2.text());
        assert_eq!(c.map_get("turn"), a2.map_get("turn"));
    }

    #[test]
    fn three_way_merge_converges() {
        let mut a = Doc::new(10);
        let mut b = Doc::new(20);
        let mut c = Doc::new(30);

        a.insert_str(0, "X");
        b.insert_str(0, "Y");
        c.insert_str(0, "Z");

        a.merge(&b);
        a.merge(&c);
        b.merge(&a);
        c.merge(&a);

        assert_eq!(a.text(), b.text());
        assert_eq!(b.text(), c.text());
        assert_eq!(a.text().chars().count(), 3);
    }

    #[test]
    fn delete_and_insert_converge() {
        let mut base = Doc::new(1);
        base.insert_str(0, "ab");
        let bytes = base.full_delta().encode().unwrap();

        let mut a = Doc::new(1);
        a.apply_delta_bytes(&bytes).unwrap();

        let mut b = Doc::new(2);
        b.apply_delta_bytes(&bytes).unwrap();

        a.delete_char(0); // delete 'a'
        b.insert_char(2, 'c'); // append 'c'

        a.merge(&b);
        b.merge(&a);

        assert_eq!(a.text(), b.text());
        assert_eq!(a.text(), "bc");
    }

    #[test]
    fn map_lww_deterministic() {
        let mut a = Doc::new(1);
        let mut b = Doc::new(2);

        a.integrate(CrdtOp::MapSet {
            key: "k".into(),
            value: b"a".to_vec(),
            time: LogicalTime::new(5, 1),
        });
        b.integrate(CrdtOp::MapSet {
            key: "k".into(),
            value: b"b".to_vec(),
            time: LogicalTime::new(5, 2),
        });

        a.merge(&b);
        b.merge(&a);
        assert_eq!(a.map_get("k"), Some(&b"b"[..]));
        assert_eq!(b.map_get("k"), Some(&b"b"[..]));
    }

    #[test]
    fn delta_idempotent() {
        let mut a = Doc::new(1);
        a.insert_str(0, "hi");
        let d = a.full_delta();
        let mut b = Doc::new(2);
        b.apply_delta(&d);
        b.apply_delta(&d);
        assert_eq!(b.text(), "hi");
        assert_eq!(b.text_len(), 2);
    }

    #[test]
    fn sequential_insert_reads_back() {
        let mut d = Doc::new(1);
        d.insert_str(0, "Mossy");
        assert_eq!(d.text(), "Mossy");
        d.insert_char(5, '!');
        assert_eq!(d.text(), "Mossy!");
        d.delete_char(5);
        assert_eq!(d.text(), "Mossy");
    }
}
