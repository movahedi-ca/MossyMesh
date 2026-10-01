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
/// Max wall-clock advance adopted from a single remote map op (issue #191).
/// Remote walls arrive over the mesh from untrusted islands; adopting an
/// unbounded wall lets one crafted op panic (`u64::MAX + 1` in debug builds)
/// or brick the replica clock (wrap to 0 in release, so every later local
/// op loses all LWW conflicts). Adoption is capped per op; the local clock
/// stays monotone and keeps advancing, so a crafted op can never freeze it.
pub const MAX_REMOTE_WALL_SKEW: u64 = 1_000_000;


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
    /// Delete targets seen before their insert (issue #31). A `Delete`
    /// integrated ahead of its target `Insert` is held here and applied
    /// when the insert arrives, so replicas converge regardless of op
    /// arrival order.
    pending_deletes: HashSet<ItemId>,
    /// Full causal op log for delta export.
    op_log: Vec<CrdtOp>,
    /// Per-agent highest integrated sequence number.
    vector: BTreeMap<AgentId, Seq>,
    /// Per-agent highest integrated LWW-map op wall time. Kept separate from
    /// `vector` because wall ticks and seq ids are independent counters.
    map_vector: BTreeMap<AgentId, u64>,
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
        self.map.get(key).and_then(|v| v.value.as_deref())
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

    /// Map-op version vector: highest map wall time seen per agent.
    pub fn map_version_vector(&self) -> &BTreeMap<AgentId, u64> {
        &self.map_vector
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
                // A delete that arrived before its target insert is held in
                // `pending_deletes`: the item is born deleted so arrival
                // order cannot fork replicas (issue #31).
                let deleted = self.pending_deletes.remove(&id);
                let item = SeqItem {
                    id,
                    parent,
                    content,
                    deleted,
                };
                self.items.insert(id, item);
                self.children.entry(parent).or_default().push(id);
                self.applied_inserts.insert(id);
                self.note_vector(id);
            }
            CrdtOp::Delete { target, op_id } => {
                if let Some(item) = self.items.get_mut(&target) {
                    item.deleted = true;
                } else {
                    // Target not inserted yet: hold the delete so a
                    // later-arriving insert is born deleted (issue #31).
                    self.pending_deletes.insert(target);
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
                self.adopt_remote_wall(time.wall);
                self.note_map_time(time);
            }
            CrdtOp::MapDelete { key, time } => {
                self.map.insert(key, LwwValue { value: None, time });
                self.adopt_remote_wall(time.wall);
                self.note_map_time(time);
            }
        }

        self.op_log.push(op);
    }

    /// Adopt a remote wall time into the local clock, bounded (issue #191).
    /// Monotonicity is preserved (`next_wall` never decreases), but a single
    /// remote op can advance it by at most `MAX_REMOTE_WALL_SKEW`. Saturating
    /// arithmetic means `u64::MAX` can neither panic nor wrap the clock.
    fn adopt_remote_wall(&mut self, wall: u64) {
        if wall >= self.next_wall {
            let adopted = wall.min(self.next_wall.saturating_add(MAX_REMOTE_WALL_SKEW));
            self.next_wall = adopted.saturating_add(1);
        }
    }

    /// Record a map op's wall time in the map version vector.
    fn note_map_time(&mut self, time: LogicalTime) {
        let entry = self.map_vector.entry(time.agent).or_insert(0);
        if time.wall > *entry {
            *entry = time.wall;
        }
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

    /// Export ops that the remote has not yet seen (based on version vectors).
    /// Map ops are filtered against the remote's map version vector, so
    /// already-synced map history is not resent on every sync.
    pub fn delta_since(
        &self,
        remote_vv: &BTreeMap<AgentId, Seq>,
        remote_map_vv: &BTreeMap<AgentId, u64>,
    ) -> Delta {
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
                CrdtOp::MapSet { time, .. } | CrdtOp::MapDelete { time, .. } => {
                    let seen = remote_map_vv.get(&time.agent).copied().unwrap_or(0);
                    if time.wall > seen {
                        ops.push(op.clone());
                    }
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
    fn delete_before_insert_converges() {
        // Issue #31: a delete integrated before its target insert must
        // still take effect. Replicas must agree no matter which op
        // arrives first.
        let mut a = Doc::new(1);
        let ins = a.insert_char(0, 'x');
        let del = a.delete_char(0).unwrap();
        assert_eq!(a.text(), "");

        // Causal order: insert then delete.
        let mut causal = Doc::new(2);
        causal.integrate(ins.clone());
        causal.integrate(del.clone());

        // Reversed arrival: delete lands before its target insert.
        let mut reversed = Doc::new(3);
        reversed.integrate(del.clone());
        reversed.integrate(ins.clone());

        assert_eq!(causal.text(), "");
        assert_eq!(reversed.text(), "");
        assert_eq!(causal.text(), reversed.text());
        // The pending tombstone is consumed once the insert arrives.
        assert!(reversed.pending_deletes.is_empty());
    }

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
    #[test]
    fn remote_wall_clock_adoption_is_bounded() {
        // Issue #191: a crafted remote op with wall = u64::MAX must not panic
        // (debug) or brick the replica clock (wrap to 0 in release).
        let mut d = Doc::new(1);
        d.map_set("k", b"v".to_vec()); // next_wall is now 2
        assert_eq!(d.next_wall, 2);

        d.integrate(CrdtOp::MapSet {
            key: "evil".into(),
            value: b"x".to_vec(),
            time: LogicalTime::new(u64::MAX, 99),
        });
        // The op itself still applies (LWW by its own time)...
        assert_eq!(d.map_get("evil"), Some(b"x".as_ref()));
        // ...but the local clock only advanced by at most MAX_REMOTE_WALL_SKEW.
        assert!(d.next_wall <= 2 + MAX_REMOTE_WALL_SKEW + 1);
        assert!(d.next_wall >= 2);

        // Local ops keep ticking normally, so the replica can still win
        // future LWW conflicts.
        let op = d.map_set("local", b"y".to_vec());
        match op {
            CrdtOp::MapSet { time, .. } => {
                assert!(time.wall <= 2 + MAX_REMOTE_WALL_SKEW + 1);
            }
            _ => panic!("expected MapSet"),
        }
    }
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

    #[test]
    fn map_ops_not_resent_after_sync() {
        let mut a = Doc::new(1);
        a.map_set("k1", b"v1".to_vec());
        a.map_set("k2", b"v2".to_vec());

        // Fresh remote sees both map ops in the delta.
        let d1 = a.delta_since(&BTreeMap::new(), &BTreeMap::new());
        assert_eq!(d1.ops.len(), 2);

        let mut b = Doc::new(2);
        b.apply_delta(&d1);
        assert_eq!(b.map_get("k1"), Some(b"v1".as_ref()));

        // After sync, the same delta query is empty: map ops entered the vector.
        let d2 = a.delta_since(b.version_vector(), b.map_version_vector());
        assert!(d2.is_empty());

        // A later map op is still exported.
        a.map_set("k3", b"v3".to_vec());
        let d3 = a.delta_since(b.version_vector(), b.map_version_vector());
        assert_eq!(d3.ops.len(), 1);
    }
}
