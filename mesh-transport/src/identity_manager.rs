//! Identity generation and PeerID / destination-name management.
//!
//! Reticulum/LXMF-style destination names are app+aspect hashes bound to a
//! signing public key. Key material is stored in zeroize-friendly wrappers so
//! secrets are scrubbed on drop when the `zeroize` crate is available.
//!
//! Cryptographic signing is stubbed: seed → deterministic 32-byte keypair
//! material suitable for later wiring to ed25519 / libp2p identity.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zeroize::{Zeroize, ZeroizeOnDrop};

/// 32-byte peer identifier (public key digest / DHT key).
pub type PeerIdBytes = [u8; 32];

/// Raw ed25519-sized key material (stub; not a live crypto implementation).
pub const KEY_LEN: usize = 32;

/// Secret key material — zeroized on drop.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct SecretKey {
    bytes: [u8; KEY_LEN],
}

impl SecretKey {
    pub fn from_bytes(bytes: [u8; KEY_LEN]) -> Self {
        Self { bytes }
    }

    /// Deterministic key from arbitrary seed bytes (HKDF-style single hash expand).
    pub fn from_seed(seed: &[u8]) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(b"mossymesh/identity/secret/v1");
        hasher.update(seed);
        let digest = hasher.finalize();
        let mut bytes = [0u8; KEY_LEN];
        bytes.copy_from_slice(&digest);
        Self { bytes }
    }

    pub fn as_bytes(&self) -> &[u8; KEY_LEN] {
        &self.bytes
    }

    /// Expose bytes for signing stubs without copying into long-lived buffers.
    pub fn expose(&self) -> [u8; KEY_LEN] {
        self.bytes
    }
}

impl std::fmt::Debug for SecretKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretKey([REDACTED])")
    }
}

/// Public verification key material.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PublicKey {
    pub bytes: [u8; KEY_LEN],
}

impl PublicKey {
    pub fn from_bytes(bytes: [u8; KEY_LEN]) -> Self {
        Self { bytes }
    }

    /// Derive a public key stub from secret material (hash domain-separated).
    /// Replace with real ed25519 derive when signing is wired.
    pub fn derive_from_secret(secret: &SecretKey) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(b"mossymesh/identity/public/v1");
        hasher.update(secret.as_bytes());
        let digest = hasher.finalize();
        let mut bytes = [0u8; KEY_LEN];
        bytes.copy_from_slice(&digest);
        Self { bytes }
    }

    pub fn as_bytes(&self) -> &[u8; KEY_LEN] {
        &self.bytes
    }
}

impl std::fmt::Debug for PublicKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PublicKey({})", hex_prefix(&self.bytes, 4))
    }
}

/// Mesh peer identity: public key + DHT PeerID.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerId {
    /// Public key material.
    pub public_key: PublicKey,
    /// DHT / libp2p-style peer id (hash of public key).
    pub id: PeerIdBytes,
}

impl PeerId {
    pub fn from_public_key(public_key: PublicKey) -> Self {
        let id = peer_id_from_public_key(&public_key);
        Self { public_key, id }
    }

    /// Legacy constructor accepting a raw 32-byte key field.
    pub fn from_key_bytes(key: [u8; 32]) -> Self {
        Self::from_public_key(PublicKey::from_bytes(key))
    }

    pub fn as_bytes(&self) -> &PeerIdBytes {
        &self.id
    }
}

/// Backward-compatible tuple-style view (older stubs used `PeerId { key }`).
impl PeerId {
    pub fn key(&self) -> [u8; 32] {
        self.public_key.bytes
    }
}

/// Full local identity including secret material.
#[derive(Clone)]
pub struct LocalIdentity {
    pub secret: SecretKey,
    pub peer: PeerId,
}

impl LocalIdentity {
    pub fn generate_from_seed(seed: &[u8]) -> Self {
        let secret = SecretKey::from_seed(seed);
        let public_key = PublicKey::derive_from_secret(&secret);
        let peer = PeerId::from_public_key(public_key);
        Self { secret, peer }
    }

    /// Random-ish identity using a process-local counter mix (not CSPRNG).
    /// Prefer [`LocalIdentity::generate_from_seed`] for tests and reproducible nodes.
    pub fn generate_ephemeral(_tag: &str) -> Self {
        let mut bytes = [0u8; KEY_LEN];
        getrandom::getrandom(&mut bytes).expect("OS RNG unavailable for ephemeral identity");
        let secret = SecretKey::from_bytes(bytes);
        let public_key = PublicKey::derive_from_secret(&secret);
        let peer = PeerId::from_public_key(public_key);
        Self { secret, peer }
    }
}

impl std::fmt::Debug for LocalIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalIdentity")
            .field("peer", &self.peer)
            .field("secret", &self.secret)
            .finish()
    }
}

/// Reticulum-style destination name: `app_name` + `aspects` bound to an identity.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DestinationName {
    pub app_name: String,
    pub aspects: Vec<String>,
    /// 32-byte destination hash (truncated full digest).
    pub hash: [u8; 32],
}

impl DestinationName {
    /// Build a destination hash:
    /// `SHA256("dest" || peer_id || app || 0x00 || aspect1 || 0x00 || …)`.
    pub fn new(peer: &PeerId, app_name: &str, aspects: &[&str]) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(b"mossymesh/destination/v1");
        hasher.update(peer.as_bytes());
        hasher.update(app_name.as_bytes());
        for aspect in aspects {
            hasher.update([0u8]);
            hasher.update(aspect.as_bytes());
        }
        let digest = hasher.finalize();
        let mut hash = [0u8; 32];
        hash.copy_from_slice(&digest);
        Self {
            app_name: app_name.to_string(),
            aspects: aspects.iter().map(|s| (*s).to_string()).collect(),
            hash,
        }
    }

    /// Human-readable reticulum-like path: `app/aspect1/aspect2`.
    pub fn path(&self) -> String {
        let mut parts = vec![self.app_name.clone()];
        parts.extend(self.aspects.iter().cloned());
        parts.join("/")
    }
}

/// Manages local identity and known peer / destination directory.
#[derive(Debug, Default)]
pub struct IdentityManager {
    pub local: Option<LocalIdentity>,
    peers: Vec<PeerId>,
    destinations: Vec<DestinationName>,
}

impl IdentityManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Install a local identity (replaces any previous).
    pub fn set_local(&mut self, identity: LocalIdentity) -> &PeerId {
        self.local = Some(identity);
        &self.local.as_ref().unwrap().peer
    }

    pub fn bootstrap_from_seed(&mut self, seed: &[u8]) -> &PeerId {
        self.set_local(LocalIdentity::generate_from_seed(seed))
    }

    pub fn local_peer_id(&self) -> Option<&PeerId> {
        self.local.as_ref().map(|i| &i.peer)
    }

    pub fn local_id_bytes(&self) -> Option<PeerIdBytes> {
        self.local_peer_id().map(|p| p.id)
    }

    /// Register a remote peer.
    pub fn add_peer(&mut self, peer: PeerId) {
        if !self.peers.iter().any(|p| p.id == peer.id) {
            self.peers.push(peer);
        }
    }

    pub fn peers(&self) -> &[PeerId] {
        &self.peers
    }

    pub fn find_peer(&self, id: &PeerIdBytes) -> Option<&PeerId> {
        self.peers.iter().find(|p| p.id == *id)
    }

    /// Announce a named destination for the local peer.
    pub fn announce_destination(
        &mut self,
        app_name: &str,
        aspects: &[&str],
    ) -> Option<DestinationName> {
        let peer = self.local_peer_id()?;
        let dest = DestinationName::new(peer, app_name, aspects);
        if !self.destinations.iter().any(|d| d.hash == dest.hash) {
            self.destinations.push(dest.clone());
        }
        Some(dest)
    }

    pub fn destinations(&self) -> &[DestinationName] {
        &self.destinations
    }

    pub fn resolve_destination(&self, hash: &[u8; 32]) -> Option<&DestinationName> {
        self.destinations.iter().find(|d| d.hash == *hash)
    }

    /// Outcome of a NodeId collision check (same 32-byte id, different keys).
    ///
    /// Resolution is deterministic: of two claimants to one id, the one with
    /// the lexicographically greater public key keeps the id. Every node that
    /// runs the same check reaches the same answer with no coordination.
    /// Fixes #29.
    pub fn add_peer_checked(&mut self, peer: PeerId) -> IdCollisionOutcome {
        // True duplicate: same id, same key. Harmless, ignore.
        if self
            .peers
            .iter()
            .any(|p| p.id == peer.id && p.public_key == peer.public_key)
        {
            return IdCollisionOutcome::DuplicateIgnored;
        }
        // Collision: same id, different key. Deterministic winner keeps the id.
        if let Some(pos) = self.peers.iter().position(|p| p.id == peer.id) {
            let existing = self.peers[pos].clone();
            let winner = deterministic_id_winner(&existing, &peer);
            let loser = if winner == existing {
                peer.clone()
            } else {
                existing
            };
            if winner == peer {
                self.peers[pos] = peer;
            }
            return IdCollisionOutcome::CollisionResolved { winner, loser };
        }
        // Collision with our own id: resolve the same way. If we lose, the
        // caller must rekey via [`IdentityManager::rekey_after_collision`].
        if let Some(local) = self.local_peer_id() {
            if local.id == peer.id {
                let local_peer = local.clone();
                let winner = deterministic_id_winner(&local_peer, &peer);
                let loser = if winner == local_peer {
                    peer.clone()
                } else {
                    local_peer.clone()
                };
                return IdCollisionOutcome::LocalCollisionResolved {
                    winner: winner.clone(),
                    loser,
                    local_lost: winner == peer,
                };
            }
        }
        self.peers.push(peer);
        IdCollisionOutcome::Accepted
    }

    /// Deterministically re-generate the local identity after losing a NodeId
    /// collision. Domain-separated from the old id plus a collision counter,
    /// so the new id is stable per attempt but differs from the contested one.
    pub fn rekey_after_collision(&mut self) -> &PeerId {
        let mut seed = b"mossymesh/collision-rekey/v1".to_vec();
        if let Some(old) = self.local_id_bytes() {
            seed.extend_from_slice(&old);
        }
        let new_id = LocalIdentity::generate_from_seed(&seed);
        // The derived seed above is fixed, so a second collision would repeat
        // it; mix the attempt count in to guarantee forward progress.
        let mut attempt = 0u32;
        let mut candidate = new_id;
        while self.peers.iter().any(|p| p.id == candidate.peer.id) {
            attempt = attempt.wrapping_add(1);
            seed.extend_from_slice(&attempt.to_le_bytes());
            candidate = LocalIdentity::generate_from_seed(&seed);
        }
        self.local = Some(candidate);
        &self.local.as_ref().unwrap().peer
    }
}

/// Deterministic winner of a NodeId collision: the claimant with the
/// lexicographically greater public key keeps the id. Every observer computes
/// the same winner independently.
pub fn deterministic_id_winner(a: &PeerId, b: &PeerId) -> PeerId {
    if a.public_key.bytes >= b.public_key.bytes {
        a.clone()
    } else {
        b.clone()
    }
}

/// Outcome of registering a peer when a NodeId collision is possible.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdCollisionOutcome {
    /// New id, registered normally.
    Accepted,
    /// Same id and same key: harmless duplicate, ignored.
    DuplicateIgnored,
    /// Same id, different key: winner keeps the id, loser must rekey.
    CollisionResolved { winner: PeerId, loser: PeerId },
    /// The contested id is our own local id. If `local_lost`, call
    /// [`IdentityManager::rekey_after_collision`].
    LocalCollisionResolved {
        winner: PeerId,
        loser: PeerId,
        local_lost: bool,
    },
}

/// Hash public key → PeerID (domain-separated SHA-256).
pub fn peer_id_from_public_key(pk: &PublicKey) -> PeerIdBytes {
    let mut hasher = Sha256::new();
    hasher.update(b"mossymesh/peer-id/v1");
    hasher.update(pk.as_bytes());
    let digest = hasher.finalize();
    let mut id = [0u8; 32];
    id.copy_from_slice(&digest);
    id
}

fn hex_prefix(bytes: &[u8], n: usize) -> String {
    bytes
        .iter()
        .take(n)
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join("")
}

pub fn init_identity_manager() {
    println!("Initializing Identity Manager (PeerID + destination-name identities).");
    let mut mgr = IdentityManager::new();
    let peer = mgr
        .bootstrap_from_seed(b"mossymesh-bootstrap-node-0")
        .clone();
    let dest = mgr
        .announce_destination("mesh", &["lxmf", "delivery"])
        .expect("local identity set");
    println!(
        "Local PeerID prefix={}… destination={} hash={}…",
        hex_prefix(&peer.id, 4),
        dest.path(),
        hex_prefix(&dest.hash, 4)
    );
}

// ---------------------------------------------------------------------------
// Backward-compatible re-export surface: older code used `PeerId { key }`.
// Provide a thin alias module pattern via inherent methods above.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ephemeral_keys_are_unpredictable_and_unique() {
        let a = LocalIdentity::generate_ephemeral("node");
        let b = LocalIdentity::generate_ephemeral("node");
        // Same tag must NOT reproduce the same key (the old PID-seeded
        // scheme did). Two calls must differ.
        assert_ne!(a.secret.expose(), b.secret.expose());
        assert_ne!(a.peer.id, b.peer.id);
    }

    #[test]
    fn seed_identity_is_deterministic() {
        let a = LocalIdentity::generate_from_seed(b"unit-test-seed");
        let b = LocalIdentity::generate_from_seed(b"unit-test-seed");
        assert_eq!(a.peer.id, b.peer.id);
        assert_eq!(a.peer.public_key, b.peer.public_key);
        assert_eq!(a.secret.expose(), b.secret.expose());
    }

    #[test]
    fn different_seeds_different_ids() {
        let a = LocalIdentity::generate_from_seed(b"seed-a");
        let b = LocalIdentity::generate_from_seed(b"seed-b");
        assert_ne!(a.peer.id, b.peer.id);
    }

    #[test]
    fn secret_debug_redacts() {
        let sk = SecretKey::from_seed(b"x");
        let s = format!("{:?}", sk);
        assert!(s.contains("REDACTED"));
        assert!(!s.contains(&format!("{:02x}", sk.expose()[0])));
    }

    #[test]
    fn destination_name_stable() {
        let id = LocalIdentity::generate_from_seed(b"dest-seed");
        let d1 = DestinationName::new(&id.peer, "chat", &["group", "alpha"]);
        let d2 = DestinationName::new(&id.peer, "chat", &["group", "alpha"]);
        assert_eq!(d1.hash, d2.hash);
        assert_eq!(d1.path(), "chat/group/alpha");

        let d3 = DestinationName::new(&id.peer, "chat", &["group", "beta"]);
        assert_ne!(d1.hash, d3.hash);
    }

    #[test]
    fn manager_announce_and_resolve() {
        let mut mgr = IdentityManager::new();
        mgr.bootstrap_from_seed(b"mgr");
        let dest = mgr.announce_destination("mesh", &["rpc"]).unwrap();
        assert!(mgr.resolve_destination(&dest.hash).is_some());
        assert_eq!(mgr.destinations().len(), 1);
        // Idempotent announce
        mgr.announce_destination("mesh", &["rpc"]);
        assert_eq!(mgr.destinations().len(), 1);
    }

    #[test]
    fn peer_directory() {
        let mut mgr = IdentityManager::new();
        mgr.bootstrap_from_seed(b"local");
        let remote = LocalIdentity::generate_from_seed(b"remote").peer;
        mgr.add_peer(remote.clone());
        mgr.add_peer(remote.clone());
        assert_eq!(mgr.peers().len(), 1);
        assert!(mgr.find_peer(&remote.id).is_some());
    }

    #[test]
    fn peer_id_from_key_bytes_compat() {
        let pk_bytes = [7u8; 32];
        let p = PeerId::from_key_bytes(pk_bytes);
        assert_eq!(p.key(), pk_bytes);
        assert_eq!(
            p.id,
            peer_id_from_public_key(&PublicKey::from_bytes(pk_bytes))
        );
    }

    fn colliding_peer(id: PeerIdBytes, key_byte: u8) -> PeerId {
        let pk = PublicKey::from_bytes([key_byte; 32]);
        PeerId { public_key: pk, id }
    }

    #[test]
    fn collision_resolves_to_greater_key_deterministically() {
        let mut mgr = IdentityManager::new();
        mgr.bootstrap_from_seed(b"local-id-29");
        let contested = [9u8; 32];
        let low = colliding_peer(contested, 0x11);
        let high = colliding_peer(contested, 0xAA);

        // Register low first, then high collides.
        assert_eq!(
            mgr.add_peer_checked(low.clone()),
            IdCollisionOutcome::Accepted
        );
        let out = mgr.add_peer_checked(high.clone());
        match out {
            IdCollisionOutcome::CollisionResolved { winner, loser } => {
                assert_eq!(winner, high, "greater key must win");
                assert_eq!(loser, low);
            }
            other => panic!("expected CollisionResolved, got {other:?}"),
        }
        // Peer list holds the winner under the contested id.
        assert_eq!(
            mgr.find_peer(&contested).unwrap().public_key,
            high.public_key
        );

        // Same inputs, same verdict: fully deterministic.
        let mut mgr2 = IdentityManager::new();
        mgr2.bootstrap_from_seed(b"local-id-29");
        mgr2.add_peer_checked(low);
        let out2 = mgr2.add_peer_checked(high.clone());
        assert!(matches!(
            out2,
            IdCollisionOutcome::CollisionResolved { winner, .. } if winner == high
        ));
    }

    #[test]
    fn exact_duplicate_is_ignored() {
        let mut mgr = IdentityManager::new();
        mgr.bootstrap_from_seed(b"local-id-29");
        let p = LocalIdentity::generate_from_seed(b"remote-29").peer;
        assert_eq!(
            mgr.add_peer_checked(p.clone()),
            IdCollisionOutcome::Accepted
        );
        assert_eq!(
            mgr.add_peer_checked(p),
            IdCollisionOutcome::DuplicateIgnored
        );
    }

    #[test]
    fn local_collision_rekeys_local_identity() {
        let mut mgr = IdentityManager::new();
        mgr.bootstrap_from_seed(b"local-will-collide");
        let local = mgr.local_peer_id().unwrap().clone();
        // Craft a claimant for the same id whose key is lexicographically
        // greater, forcing the local identity to lose.
        let mut claimant = local.clone();
        claimant.public_key = PublicKey::from_bytes([0xFF; 32]);
        assert!(claimant.public_key.bytes > local.public_key.bytes);
        let out = mgr.add_peer_checked(claimant);
        match out {
            IdCollisionOutcome::LocalCollisionResolved { local_lost, .. } => {
                assert!(local_lost, "local must lose to greater key");
            }
            other => panic!("expected LocalCollisionResolved, got {other:?}"),
        }
        let before = mgr.local_id_bytes().unwrap();
        let new_peer = mgr.rekey_after_collision().clone();
        assert_ne!(new_peer.id, before, "rekey must change the local id");
        assert_ne!(new_peer.id, local.id);
    }
}
