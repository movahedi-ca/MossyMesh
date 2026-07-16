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
    pub fn generate_ephemeral(tag: &str) -> Self {
        let mut seed = Vec::from(b"ephemeral:".as_slice());
        seed.extend_from_slice(tag.as_bytes());
        seed.extend_from_slice(&std::process::id().to_le_bytes());
        Self::generate_from_seed(&seed)
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
    let peer = mgr.bootstrap_from_seed(b"mossymesh-bootstrap-node-0").clone();
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
        assert_eq!(p.id, peer_id_from_public_key(&PublicKey::from_bytes(pk_bytes)));
    }
}
