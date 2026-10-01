//! Bluetooth Low Energy mesh: link-state advertisements and neighbor table.
//!
//! Simulation-capable control plane for ultra-low-power proximity links.
//! Peers exchange compact LSAs; each node maintains a TTL-pruned neighbor table.
//!
//! ## LSA authenticity (fixes #205)
//!
//! Every [`LinkStateAdvertisement`] is signed with the origin node's ed25519
//! key, and verification happens **before** any neighbor-table or
//! sequence-tracker state is touched. The origin id is **self-certifying**: it
//! is the lowercase hex encoding of the origin's 32-byte ed25519 verifying
//! key, so the receiver needs no key registry and there is no key-distribution
//! step for an attacker to subvert. Forging or ratcheting another node's LSA
//! (bumping `sequence`, rewriting `neighbors`, back-dating `generated_at_ms`)
//! requires that node's private key.
//!
//! Trust assumption, stated plainly: identity == key. A peer presenting a
//! valid signature under origin id `X` *is* the holder of `X`'s private key.
//! Minting a *new* identity (Sybil) is still possible and is out of scope
//! here; Sybil-cost machinery lives in `vdf_sybil.rs`. BLE beacons
//! ([`BleBeacon`]) remain unsigned discovery hints; only LSAs carry trusted
//! link-state.
//!
//! Signed payload (everything the receiver trusts), big-endian:
//! ```text
//! u16 len || origin_id bytes || u32 sequence || u8 battery_level ||
//! u64 generated_at_ms || u64 ttl_ms || u16 neighbor_count ||
//! per neighbor: u16 len || id bytes || u32 cost
//! ```
//! Wire encoding = signed payload || 64-byte ed25519 signature. Signed LSAs
//! are larger than the legacy 20-byte relay MTU; use [`fragment_ble_payload`]
//! to move them across legacy relays.

use std::collections::BTreeMap;

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use sha2::{Digest, Sha256};

/// Default LSA TTL in simulated milliseconds.
pub const DEFAULT_LSA_TTL_MS: u64 = 30_000;

/// Default periodic advertisement interval (ms).
pub const DEFAULT_ADV_INTERVAL_MS: u64 = 2_000;

/// Maximum neighbors retained (RAM ceiling friendly).
pub const MAX_NEIGHBORS: usize = 32;

/// Compact BLE advertising beacon (discovery).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BleBeacon {
    pub node_id: String,
    pub battery_level: u8,
}

impl BleBeacon {
    /// Simulates the periodic BLE advertising loop used to discover sleeping offline nodes.
    pub fn broadcast_loop(&self) {
        println!(
            "BLE Broadcast: Advertising PeerID {} | Battery: {}%",
            self.node_id, self.battery_level
        );
    }
}

/// Link-state advertisement flooded over BLE mesh.
///
/// `origin_id` is the lowercase hex of the origin's ed25519 verifying key
/// (self-certifying identity, see module docs). `signature` is the origin's
/// ed25519 signature over [`Self::signed_payload_bytes`]; it covers every
/// field the receiver trusts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkStateAdvertisement {
    pub origin_id: String,
    pub sequence: u32,
    pub battery_level: u8,
    /// Reported one-hop neighbors of the origin (id → link cost).
    pub neighbors: Vec<(String, u32)>,
    /// Simulated wall-clock when this LSA was generated (ms).
    pub generated_at_ms: u64,
    pub ttl_ms: u64,
    pub signature: [u8; 64],
}

impl LinkStateAdvertisement {
    pub fn new(
        origin_id: impl Into<String>,
        sequence: u32,
        battery_level: u8,
        neighbors: Vec<(String, u32)>,
        generated_at_ms: u64,
    ) -> Self {
        Self {
            origin_id: origin_id.into(),
            sequence,
            battery_level,
            neighbors,
            generated_at_ms,
            ttl_ms: DEFAULT_LSA_TTL_MS,
            // Unsigned until `sign` is called (e.g. via `BleMeshNode::create_lsa`).
            signature: [0u8; 64],
        }
    }

    pub fn is_expired(&self, now_ms: u64) -> bool {
        now_ms.saturating_sub(self.generated_at_ms) > self.ttl_ms
    }

    /// Canonical bytes covered by the signature: every trusted field,
    /// big-endian, length-prefixed UTF-8 ids (see module docs for layout).
    pub fn signed_payload_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        encode_str(&mut out, &self.origin_id);
        out.extend_from_slice(&self.sequence.to_be_bytes());
        out.push(self.battery_level);
        out.extend_from_slice(&self.generated_at_ms.to_be_bytes());
        out.extend_from_slice(&self.ttl_ms.to_be_bytes());
        out.extend_from_slice(&(self.neighbors.len() as u16).to_be_bytes());
        for (id, cost) in &self.neighbors {
            encode_str(&mut out, id);
            out.extend_from_slice(&cost.to_be_bytes());
        }
        out
    }

    /// Sign this LSA in place with the origin's key.
    pub fn sign(&mut self, signing_key: &SigningKey) {
        let sig: Signature = signing_key.sign(&self.signed_payload_bytes());
        self.signature = sig.to_bytes();
    }

    /// The origin's verifying key, decoded from the self-certifying origin id.
    pub fn origin_verifying_key(&self) -> Option<VerifyingKey> {
        let raw = hex_decode_32(&self.origin_id)?;
        VerifyingKey::from_bytes(&raw).ok()
    }

    /// True iff the signature is valid under the key named by `origin_id`.
    /// Pure cryptography: no table or sequence state is consulted.
    pub fn verify(&self) -> bool {
        let Some(key) = self.origin_verifying_key() else {
            return false;
        };
        let Ok(sig_bytes) = <[u8; 64]>::try_from(&self.signature[..]) else {
            return false;
        };
        let sig = Signature::from_bytes(&sig_bytes);
        key.verify(&self.signed_payload_bytes(), &sig).is_ok()
    }

    /// Deterministic binary encoding for sim/tests: signed payload followed
    /// by the 64-byte signature.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = self.signed_payload_bytes();
        out.extend_from_slice(&self.signature);
        out
    }

    pub fn decode(bytes: &[u8]) -> Option<Self> {
        let mut i = 0usize;
        let origin_id = decode_str(bytes, &mut i)?;
        if i + 4 + 1 + 8 + 8 + 2 > bytes.len() {
            return None;
        }
        let sequence = u32::from_be_bytes(bytes[i..i + 4].try_into().ok()?);
        i += 4;
        let battery_level = bytes[i];
        i += 1;
        let generated_at_ms = u64::from_be_bytes(bytes[i..i + 8].try_into().ok()?);
        i += 8;
        let ttl_ms = u64::from_be_bytes(bytes[i..i + 8].try_into().ok()?);
        i += 8;
        let n = u16::from_be_bytes(bytes[i..i + 2].try_into().ok()?) as usize;
        i += 2;
        let mut neighbors = Vec::with_capacity(n);
        for _ in 0..n {
            let id = decode_str(bytes, &mut i)?;
            if i + 4 > bytes.len() {
                return None;
            }
            let cost = u32::from_be_bytes(bytes[i..i + 4].try_into().ok()?);
            i += 4;
            neighbors.push((id, cost));
        }
        // Exactly one 64-byte signature must follow; truncated or padded
        // wire bytes are rejected.
        if bytes.len() - i != 64 {
            return None;
        }
        let mut signature = [0u8; 64];
        signature.copy_from_slice(&bytes[i..i + 64]);
        Some(Self {
            origin_id,
            sequence,
            battery_level,
            neighbors,
            generated_at_ms,
            ttl_ms,
            signature,
        })
    }
}

/// Lowercase hex of a 32-byte key (self-certifying node ids).
fn hex_encode_32(bytes: &[u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(64);
    for b in bytes {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0x0f) as usize] as char);
    }
    s
}

fn hex_val(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        _ => None,
    }
}

/// Decode a 64-char lowercase hex node id back to 32 key bytes.
fn hex_decode_32(s: &str) -> Option<[u8; 32]> {
    let b = s.as_bytes();
    if b.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, chunk) in b.chunks(2).enumerate() {
        out[i] = (hex_val(chunk[0])? << 4) | hex_val(chunk[1])?;
    }
    Some(out)
}

fn encode_str(out: &mut Vec<u8>, s: &str) {
    let b = s.as_bytes();
    let len = (b.len() as u16).to_be_bytes();
    out.extend_from_slice(&len);
    out.extend_from_slice(b);
}

fn decode_str(bytes: &[u8], i: &mut usize) -> Option<String> {
    if *i + 2 > bytes.len() {
        return None;
    }
    let len = u16::from_be_bytes(bytes[*i..*i + 2].try_into().ok()?) as usize;
    *i += 2;
    if *i + len > bytes.len() {
        return None;
    }
    let s = std::str::from_utf8(&bytes[*i..*i + len]).ok()?.to_string();
    *i += len;
    Some(s)
}

/// One entry in the local BLE neighbor table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NeighborEntry {
    pub node_id: String,
    pub battery_level: u8,
    /// Higher is better (0–255, from RSSI mapping or sim).
    pub link_quality: u8,
    pub last_seen_ms: u64,
    pub last_seq: u32,
    /// Path cost to use this neighbor as next hop (lower is better).
    pub cost: u32,
}

impl NeighborEntry {
    pub fn is_stale(&self, now_ms: u64, ttl_ms: u64) -> bool {
        now_ms.saturating_sub(self.last_seen_ms) > ttl_ms
    }
}

/// Local BLE mesh control-plane state.
///
/// The node owns an ed25519 signing key; `node_id` is the lowercase hex of
/// the corresponding verifying key (self-certifying identity). LSAs this
/// node emits are always signed; LSAs it ingests are verified before any
/// table state changes.
#[derive(Clone)]
pub struct BleMeshNode {
    pub node_id: String,
    /// LSA signing key (secret; redacted in `Debug`).
    signing_key: SigningKey,
    pub battery_level: u8,
    pub seq: u32,
    pub now_ms: u64,
    pub neighbor_ttl_ms: u64,
    /// Ordered map for deterministic iteration.
    neighbors: BTreeMap<String, NeighborEntry>,
    /// Highest LSA sequence seen per origin (loop / replay suppression).
    seen_seq: BTreeMap<String, u32>,
}

impl std::fmt::Debug for BleMeshNode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BleMeshNode")
            .field("node_id", &self.node_id)
            .field("signing_key", &"[REDACTED]")
            .field("battery_level", &self.battery_level)
            .field("seq", &self.seq)
            .field("now_ms", &self.now_ms)
            .field("neighbor_ttl_ms", &self.neighbor_ttl_ms)
            .field("neighbors", &self.neighbors)
            .field("seen_seq", &self.seen_seq)
            .finish()
    }
}

impl BleMeshNode {
    fn from_signing_key(signing_key: SigningKey, battery_level: u8) -> Self {
        let node_id = hex_encode_32(signing_key.verifying_key().as_bytes());
        Self {
            node_id,
            signing_key,
            battery_level,
            seq: 0,
            now_ms: 0,
            neighbor_ttl_ms: DEFAULT_LSA_TTL_MS,
            neighbors: BTreeMap::new(),
            seen_seq: BTreeMap::new(),
        }
    }

    /// Fresh random identity (production path).
    pub fn generate(battery_level: u8) -> Self {
        let mut bytes = [0u8; 32];
        getrandom::getrandom(&mut bytes).expect("RNG failure generating BLE node key");
        Self::from_signing_key(SigningKey::from_bytes(&bytes), battery_level)
    }

    /// Deterministic identity from a seed (tests and simulations).
    pub fn from_seed(seed: &[u8], battery_level: u8) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(b"mossymesh/ble-mesh/lsa-key/v1");
        hasher.update(seed);
        let digest = hasher.finalize();
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(&digest);
        Self::from_signing_key(SigningKey::from_bytes(&bytes), battery_level)
    }

    pub fn advance_time(&mut self, delta_ms: u64) {
        self.now_ms = self.now_ms.saturating_add(delta_ms);
        self.prune_stale();
    }

    pub fn set_time(&mut self, now_ms: u64) {
        self.now_ms = now_ms;
        self.prune_stale();
    }

    pub fn neighbor_count(&self) -> usize {
        self.neighbors.len()
    }

    pub fn get_neighbor(&self, id: &str) -> Option<&NeighborEntry> {
        self.neighbors.get(id)
    }

    pub fn neighbors(&self) -> impl Iterator<Item = &NeighborEntry> {
        self.neighbors.values()
    }

    /// Map raw RSSI (dBm, typically −100..−40) to link quality 0..255 and cost.
    pub fn rssi_to_quality_cost(rssi_dbm: i8) -> (u8, u32) {
        // Clamp to [-100, -40]
        let r = (rssi_dbm as i16).clamp(-100, -40);
        // quality: -40 → 255, -100 → 0
        let quality = (((r + 100) as u32 * 255) / 60) as u8;
        // cost: inverse of quality, min 1
        let cost = 1u32 + (255u32.saturating_sub(quality as u32)) / 8;
        (quality, cost)
    }

    /// Direct hearing of a peer (beacon or unicast).
    pub fn hear_peer(&mut self, peer_id: &str, battery_level: u8, rssi_dbm: i8) {
        let (quality, cost) = Self::rssi_to_quality_cost(rssi_dbm);
        let entry = NeighborEntry {
            node_id: peer_id.to_string(),
            battery_level,
            link_quality: quality,
            last_seen_ms: self.now_ms,
            last_seq: self.neighbors.get(peer_id).map(|e| e.last_seq).unwrap_or(0),
            cost,
        };
        self.insert_neighbor(entry);
    }

    fn insert_neighbor(&mut self, entry: NeighborEntry) {
        if self.neighbors.len() >= MAX_NEIGHBORS && !self.neighbors.contains_key(&entry.node_id) {
            // Evict lowest link quality (deterministic: min quality, then id).
            if let Some(evict) = self
                .neighbors
                .values()
                .min_by(|a, b| {
                    a.link_quality
                        .cmp(&b.link_quality)
                        .then_with(|| a.node_id.cmp(&b.node_id))
                })
                .map(|e| e.node_id.clone())
            {
                if entry.link_quality
                    <= self
                        .neighbors
                        .get(&evict)
                        .map(|e| e.link_quality)
                        .unwrap_or(0)
                {
                    return; // new peer worse than worst — drop
                }
                self.neighbors.remove(&evict);
            }
        }
        self.neighbors.insert(entry.node_id.clone(), entry);
    }

    /// Build the next outbound LSA for this node, signed with the node's key.
    pub fn create_lsa(&mut self) -> LinkStateAdvertisement {
        self.seq = self.seq.wrapping_add(1);
        let neighbors: Vec<(String, u32)> = self
            .neighbors
            .values()
            .map(|n| (n.node_id.clone(), n.cost))
            .collect();
        let mut lsa = LinkStateAdvertisement::new(
            self.node_id.clone(),
            self.seq,
            self.battery_level,
            neighbors,
            self.now_ms,
        );
        lsa.sign(&self.signing_key);
        lsa
    }

    /// Process an inbound LSA. Returns true if the table was updated / should re-flood.
    ///
    /// The signature is verified **before** any sequence-tracker or neighbor
    /// state is updated: a forged LSA cannot poison `seen_seq`, ratchet the
    /// sequence window, or plant a neighbor entry. Rejection order: self,
    /// expired, bad signature, stale sequence.
    pub fn process_lsa(&mut self, lsa: &LinkStateAdvertisement) -> bool {
        if lsa.origin_id == self.node_id {
            return false;
        }
        if lsa.is_expired(self.now_ms) {
            return false;
        }
        if !lsa.verify() {
            return false;
        }
        if let Some(&seen) = self.seen_seq.get(&lsa.origin_id) {
            if lsa.sequence <= seen {
                return false;
            }
        }
        self.seen_seq.insert(lsa.origin_id.clone(), lsa.sequence);

        // Treat LSA origin as a one-hop neighbor if we received it over BLE.
        // Quality is derived from battery as a weak signal proxy when RSSI unknown.
        let quality = lsa.battery_level.saturating_mul(2);
        let cost = 1u32 + (255u32.saturating_sub(quality as u32)) / 16;
        let entry = NeighborEntry {
            node_id: lsa.origin_id.clone(),
            battery_level: lsa.battery_level,
            link_quality: quality,
            last_seen_ms: self.now_ms,
            last_seq: lsa.sequence,
            cost,
        };
        self.insert_neighbor(entry);
        true
    }

    pub fn prune_stale(&mut self) {
        let now = self.now_ms;
        let ttl = self.neighbor_ttl_ms;
        self.neighbors.retain(|_, e| !e.is_stale(now, ttl));
    }

    /// Snapshot of neighbor ids sorted for deterministic topology export.
    pub fn neighbor_ids(&self) -> Vec<String> {
        self.neighbors.keys().cloned().collect()
    }
}

pub fn init_ble_mesh() {
    println!("Initializing BLE Mesh routing for ultra-close offline proximity.");
    let mut node = BleMeshNode::generate(88);
    node.hear_peer("NodeB_BLE_MAC", 70, -55);
    node.hear_peer("NodeC_BLE_MAC", 40, -80);
    let lsa = node.create_lsa();
    println!(
        "BLE LSA id={}.. seq={} neighbors={} encoded={}B signed={}",
        &node.node_id[..16],
        lsa.sequence,
        lsa.neighbors.len(),
        lsa.encode().len(),
        lsa.verify(),
    );
    let beacon = BleBeacon {
        node_id: node.node_id.clone(),
        battery_level: node.battery_level,
    };
    beacon.broadcast_loop();
}

/// Legacy BLE ATT payload ceiling: some Android devices drop anything larger
/// than 20 bytes when acting as relays. Fixes #25.
pub const BLE_LEGACY_MTU: usize = 20;

/// Relay fragment header: message_id (u16 BE) || index (u8) || count (u8).
pub const BLE_RELAY_HEADER_LEN: usize = 4;

/// Payload bytes per relay fragment, so total on-air size stays within 20 B.
pub const BLE_RELAY_PAYLOAD_LEN: usize = BLE_LEGACY_MTU - BLE_RELAY_HEADER_LEN;

/// One BLE relay fragment small enough for legacy 20-byte relays.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BleRelayFragment {
    pub message_id: u16,
    pub index: u8,
    pub count: u8,
    pub payload: Vec<u8>,
}

impl BleRelayFragment {
    /// Encode to the exact on-air bytes (always <= 20).
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(BLE_RELAY_HEADER_LEN + self.payload.len());
        out.extend_from_slice(&self.message_id.to_be_bytes());
        out.push(self.index);
        out.push(self.count);
        out.extend_from_slice(&self.payload);
        out
    }

    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < BLE_RELAY_HEADER_LEN || bytes.len() > BLE_LEGACY_MTU {
            return None;
        }
        let mut id = [0u8; 2];
        id.copy_from_slice(&bytes[..2]);
        Some(Self {
            message_id: u16::from_be_bytes(id),
            index: bytes[2],
            count: bytes[3],
            payload: bytes[BLE_RELAY_HEADER_LEN..].to_vec(),
        })
    }
}

/// Split a payload into relay fragments, each fitting in 20 bytes on air.
pub fn fragment_ble_payload(message_id: u16, payload: &[u8]) -> Vec<BleRelayFragment> {
    if payload.is_empty() {
        return Vec::new();
    }
    let mtu = BLE_RELAY_PAYLOAD_LEN;
    let count = payload.len().div_ceil(mtu).min(u8::MAX as usize) as u8;
    (0..count)
        .map(|index| {
            let start = (index as usize) * mtu;
            let end = start.saturating_add(mtu).min(payload.len());
            BleRelayFragment {
                message_id,
                index,
                count,
                payload: payload[start..end].to_vec(),
            }
        })
        .collect()
}

/// Reassembles one BLE relay message from its fragments (any arrival order).
#[derive(Debug, Default)]
pub struct BleRelayReassembler {
    message_id: Option<u16>,
    count: u8,
    parts: Vec<Option<Vec<u8>>>,
}

impl BleRelayReassembler {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed one fragment. Returns the full payload once every part arrived.
    pub fn accept(&mut self, frag: &BleRelayFragment) -> Option<Vec<u8>> {
        if self.message_id.is_none() {
            self.message_id = Some(frag.message_id);
            self.count = frag.count;
            self.parts = vec![None; frag.count as usize];
        }
        if self.message_id != Some(frag.message_id) || frag.count != self.count {
            return None;
        }
        let idx = frag.index as usize;
        if idx >= self.parts.len() {
            return None;
        }
        self.parts[idx] = Some(frag.payload.clone());
        if self.parts.iter().all(|p| p.is_some()) {
            let mut out = Vec::new();
            for part in self.parts.drain(..).flatten() {
                out.extend_from_slice(&part);
            }
            self.message_id = None;
            Some(out)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node_a() -> BleMeshNode {
        BleMeshNode::from_seed(b"test-vector-node-a", 80)
    }

    fn node_b() -> BleMeshNode {
        BleMeshNode::from_seed(b"test-vector-node-b", 60)
    }

    #[test]
    fn lsa_sign_verify_roundtrip() {
        let mut node = node_a();
        let lsa = node.create_lsa();
        assert!(lsa.verify(), "freshly signed LSA must verify");
        let bytes = lsa.encode();
        let decoded = LinkStateAdvertisement::decode(&bytes).unwrap();
        assert_eq!(lsa, decoded);
        assert!(decoded.verify(), "decoded LSA must still verify");
    }

    #[test]
    fn lsa_encode_decode_roundtrip() {
        let mut node = node_a();
        node.hear_peer("peer-beta", 70, -55);
        let lsa = node.create_lsa();
        let decoded = LinkStateAdvertisement::decode(&lsa.encode()).unwrap();
        assert_eq!(lsa, decoded);
    }

    #[test]
    fn lsa_decode_rejects_truncated_or_padded_wire() {
        let mut node = node_a();
        let bytes = node.create_lsa().encode();
        assert!(LinkStateAdvertisement::decode(&bytes[..bytes.len() - 1]).is_none());
        let mut padded = bytes.clone();
        padded.push(0x00);
        assert!(LinkStateAdvertisement::decode(&padded).is_none());
        assert!(LinkStateAdvertisement::decode(&[]).is_none());
    }

    /// Every trusted field is covered by the signature: flipping any one of
    /// them (or the signature itself) must fail verification.
    #[test]
    fn lsa_signature_rejects_tampered_fields() {
        let mut node = node_b();
        node.hear_peer("peer-gamma", 40, -80);
        let base = node.create_lsa();
        assert!(base.verify());

        let mut tampered = base.clone();
        tampered.sequence = tampered.sequence.wrapping_add(1);
        assert!(!tampered.verify(), "bumped sequence must not verify");

        let mut tampered = base.clone();
        tampered.battery_level ^= 0xff;
        assert!(!tampered.verify(), "battery must be covered");

        let mut tampered = base.clone();
        tampered.generated_at_ms = tampered.generated_at_ms.wrapping_add(1);
        assert!(!tampered.verify(), "timestamp must be covered");

        let mut tampered = base.clone();
        tampered.ttl_ms = tampered.ttl_ms.wrapping_add(1);
        assert!(!tampered.verify(), "ttl must be covered");

        let mut tampered = base.clone();
        tampered.neighbors.push(("evil-peer".into(), 1));
        assert!(!tampered.verify(), "added neighbor must not verify");

        let mut tampered = base.clone();
        tampered.neighbors[0].1 = tampered.neighbors[0].1.wrapping_add(1);
        assert!(!tampered.verify(), "neighbor cost must be covered");

        let mut tampered = base.clone();
        tampered.neighbors.clear();
        assert!(!tampered.verify(), "removed neighbors must not verify");

        // Flip the last hex digit of the self-certifying origin id.
        let mut tampered = base.clone();
        let mut id = tampered.origin_id.clone().into_bytes();
        let last = id.len() - 1;
        id[last] = if id[last] == b'0' { b'1' } else { b'0' };
        tampered.origin_id = String::from_utf8(id).unwrap();
        assert!(!tampered.verify(), "origin id swap must not verify");

        let mut tampered = base.clone();
        tampered.signature[0] ^= 0x01;
        assert!(!tampered.verify(), "flipped signature byte must not verify");

        let mut tampered = base.clone();
        tampered.signature = [0u8; 64];
        assert!(!tampered.verify(), "zeroed signature must not verify");
    }

    /// A signature made by a *different* key does not authenticate as the
    /// claimed origin: the classic cross-key forgery.
    #[test]
    fn lsa_wrong_key_signature_rejected() {
        let a = node_a();
        let b = node_b();
        let mut forged = LinkStateAdvertisement::new(b.node_id.clone(), 42, 60, vec![], 0);
        forged.sign(&a.signing_key); // attacker signs victim's origin id with own key
        assert!(!forged.verify());

        let mut receiver = node_a();
        assert!(!receiver.process_lsa(&forged));
        assert!(receiver.get_neighbor(&b.node_id).is_none());
    }

    /// The core #205 attack: attacker bumps the sequence number on a forged
    /// LSA for a victim origin. Without the victim's key it must be rejected,
    /// and it must not touch any receiver state.
    #[test]
    fn ingest_rejects_forged_ratchet_sequence() {
        let mut victim = node_b();
        let mut receiver = node_a();

        // Attacker crafts an LSA claiming the victim's origin id with a huge
        // sequence number but no valid signature.
        let forged = LinkStateAdvertisement::new(victim.node_id.clone(), 9999, 100, vec![], 0);
        assert!(!forged.verify());
        assert!(!receiver.process_lsa(&forged));
        // Verify-before-state: the forgery poisoned nothing.
        assert!(receiver.get_neighbor(&victim.node_id).is_none());

        // Attacker replays a *valid* signature from the victim's old LSA
        // under a bumped sequence number: the signature covers the sequence,
        // so it must still fail.
        let old = victim.create_lsa(); // seq 1, validly signed
        let mut ratchet = old.clone();
        ratchet.sequence = 5000;
        assert!(!ratchet.verify());
        assert!(!receiver.process_lsa(&ratchet));
        assert!(receiver.get_neighbor(&victim.node_id).is_none());

        // The real victim LSA is still accepted afterwards.
        let legit = victim.create_lsa(); // seq 2
        assert!(receiver.process_lsa(&legit));
        assert_eq!(receiver.get_neighbor(&victim.node_id).unwrap().last_seq, 2);
    }

    /// Captured valid LSA, neighbors rewritten, old signature kept: rejected.
    #[test]
    fn ingest_rejects_replayed_lsa_with_modified_neighbors() {
        let mut victim = node_b();
        let mut receiver = node_a();
        victim.hear_peer("real-neighbor", 50, -60);
        let captured = victim.create_lsa();

        let mut modified = captured.clone();
        modified.neighbors = vec![("attacker-node".into(), 1)];
        assert!(!modified.verify());
        assert!(!receiver.process_lsa(&modified));
        assert!(receiver.get_neighbor(&victim.node_id).is_none());

        // Untouched capture is accepted.
        assert!(receiver.process_lsa(&captured));
    }

    /// Malformed origin ids (not a 32-byte key) are rejected at ingest.
    #[test]
    fn ingest_rejects_malformed_origin_id() {
        let mut receiver = node_a();
        let mut lsa = LinkStateAdvertisement::new("not-a-key", 1, 60, vec![], 0);
        // Even signed by a real key, the origin id does not name that key.
        lsa.sign(&receiver.signing_key);
        assert!(!lsa.verify());
        assert!(!receiver.process_lsa(&lsa));
    }

    #[test]
    fn ingest_accepts_legitimate_advance_rejects_replay() {
        let mut origin = node_b();
        let mut receiver = node_a();

        let lsa1 = origin.create_lsa();
        assert!(receiver.process_lsa(&lsa1));
        assert!(receiver.get_neighbor(&origin.node_id).is_some());

        // Exact replay of seq 1: rejected.
        assert!(!receiver.process_lsa(&lsa1));

        let lsa2 = origin.create_lsa();
        assert!(receiver.process_lsa(&lsa2));
        assert_eq!(receiver.get_neighbor(&origin.node_id).unwrap().last_seq, 2);
        assert_eq!(
            receiver
                .get_neighbor(&origin.node_id)
                .unwrap()
                .battery_level,
            60
        );

        // Stale sequence after advance: rejected.
        assert!(!receiver.process_lsa(&lsa1));
    }

    /// Two honest nodes exchange signed LSAs; both neighbor tables update.
    #[test]
    fn two_honest_nodes_exchange_lsas() {
        let mut a = node_a();
        let mut b = node_b();
        a.hear_peer("direct-of-a", 70, -55);

        let lsa_b = b.create_lsa();
        let lsa_a = a.create_lsa();

        assert!(a.process_lsa(&lsa_b));
        assert!(b.process_lsa(&lsa_a));

        let nb = a.get_neighbor(&b.node_id).expect("A should learn B");
        assert_eq!(nb.last_seq, 1);
        assert_eq!(nb.battery_level, 60);
        let na = b.get_neighbor(&a.node_id).expect("B should learn A");
        assert_eq!(na.last_seq, 1);
        assert_eq!(na.battery_level, 80);
        // A's own direct-hearing entry is untouched by B's LSA.
        assert!(a.get_neighbor("direct-of-a").is_some());
    }

    #[test]
    fn self_lsa_is_ignored() {
        let mut node = node_a();
        let lsa = node.create_lsa();
        assert!(!node.process_lsa(&lsa));
    }

    #[test]
    fn node_id_is_self_certifying_key_hex() {
        let node = node_b();
        assert_eq!(node.node_id.len(), 64);
        let key = node.signing_key.verifying_key();
        assert_eq!(hex_encode_32(key.as_bytes()), node.node_id);
    }

    #[test]
    fn neighbor_table_hears_and_prunes() {
        let mut node = node_a();
        node.hear_peer("B", 50, -50);
        assert_eq!(node.neighbor_count(), 1);
        node.neighbor_ttl_ms = 1000;
        node.advance_time(1001);
        assert_eq!(node.neighbor_count(), 0);
    }

    #[test]
    fn create_lsa_includes_direct_neighbors() {
        let mut node = node_a();
        node.hear_peer("B", 70, -55);
        node.hear_peer("C", 40, -90);
        let lsa = node.create_lsa();
        assert_eq!(lsa.sequence, 1);
        assert_eq!(lsa.neighbors.len(), 2);
        // Deterministic BTree order
        assert_eq!(lsa.neighbors[0].0, "B");
        assert_eq!(lsa.neighbors[1].0, "C");
        assert!(lsa.verify());
    }

    #[test]
    fn rssi_mapping_better_signal_lower_cost() {
        let (_, cost_good) = BleMeshNode::rssi_to_quality_cost(-45);
        let (_, cost_bad) = BleMeshNode::rssi_to_quality_cost(-95);
        assert!(cost_good < cost_bad);
    }

    #[test]
    fn max_neighbors_eviction() {
        let mut node = node_a();
        for i in 0..MAX_NEIGHBORS {
            // Weak peers
            node.hear_peer(&format!("n{i:02}"), 10, -95);
        }
        assert_eq!(node.neighbor_count(), MAX_NEIGHBORS);
        // Strong peer should displace a weak one
        node.hear_peer("strong", 100, -40);
        assert_eq!(node.neighbor_count(), MAX_NEIGHBORS);
        assert!(node.get_neighbor("strong").is_some());
    }

    #[test]
    fn relay_fragments_fit_legacy_mtu() {
        let payload: Vec<u8> = (0..100).map(|i| i as u8).collect();
        let frags = fragment_ble_payload(0x1234, &payload);
        assert!(frags.len() > 1);
        for f in &frags {
            let wire = f.encode();
            assert!(
                wire.len() <= BLE_LEGACY_MTU,
                "fragment must fit 20-byte relay MTU, got {}",
                wire.len()
            );
            let dec = BleRelayFragment::decode(&wire).unwrap();
            assert_eq!(dec, *f);
        }
    }

    #[test]
    fn relay_reassembler_accepts_any_order() {
        let payload: Vec<u8> = (0..64).map(|i| (i * 3) as u8).collect();
        let mut frags = fragment_ble_payload(0xBEEF, &payload);
        frags.reverse();
        let mut reasm = BleRelayReassembler::new();
        let mut done = None;
        for f in &frags {
            if let Some(out) = reasm.accept(f) {
                done = Some(out);
            }
        }
        assert_eq!(done, Some(payload));
    }

    #[test]
    fn empty_payload_produces_no_fragments() {
        assert!(fragment_ble_payload(1, &[]).is_empty());
    }
}
