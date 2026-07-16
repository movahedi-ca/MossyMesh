//! Identity-based AEAD encryption layer for mesh payloads.
//!
//! Uses ChaCha20-Poly1305 with keys derived from peer identity material via SHA-256.
//! Nonces are 12 bytes; callers must never reuse `(key, nonce)` pairs.
//!
//! The historical DHKE helpers remain for handshake demos / forward-secrecy sketches.

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use sha2::{Digest, Sha256};

/// 32-byte symmetric key for AEAD.
pub type AeadKey = [u8; 32];
/// 12-byte ChaCha20-Poly1305 nonce.
pub type AeadNonce = [u8; 12];

/// Errors from the encryption layer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EncryptionError {
    /// AEAD open failed (tamper / wrong key / wrong AAD).
    DecryptFailed,
    /// Nonce length invalid (must be 12 bytes).
    InvalidNonce,
    /// Key material empty.
    EmptyIdentity,
}

/// Identity-based AEAD wrapper.
///
/// Key derivation: `key = SHA-256(domain || local_id || remote_id || context)`.
/// Encryption is directional — swapping local/remote yields a different key, so
/// each direction of a session can use independent keys when desired.
#[derive(Clone, Debug)]
pub struct IdentityAead {
    key: AeadKey,
}

impl IdentityAead {
    /// Derive a session key from two identity blobs and an optional context label.
    pub fn derive(
        local_identity: &[u8],
        remote_identity: &[u8],
        context: &[u8],
    ) -> Result<Self, EncryptionError> {
        if local_identity.is_empty() || remote_identity.is_empty() {
            return Err(EncryptionError::EmptyIdentity);
        }
        let mut hasher = Sha256::new();
        hasher.update(b"mossymesh/id-aead/v1");
        hasher.update(local_identity);
        hasher.update(remote_identity);
        hasher.update(context);
        let digest = hasher.finalize();
        let mut key = [0u8; 32];
        key.copy_from_slice(&digest);
        Ok(Self { key })
    }

    /// Construct from a raw 32-byte key (e.g. after an external DHKE).
    pub fn from_raw_key(key: AeadKey) -> Self {
        Self { key }
    }

    pub fn key_bytes(&self) -> &AeadKey {
        &self.key
    }

    /// Encrypt `plaintext` with associated data `aad`.
    /// Returns ciphertext || 16-byte Poly1305 tag.
    pub fn seal(
        &self,
        nonce: &AeadNonce,
        plaintext: &[u8],
        aad: &[u8],
    ) -> Result<Vec<u8>, EncryptionError> {
        let cipher = ChaCha20Poly1305::new(Key::from_slice(&self.key));
        let n = Nonce::from_slice(nonce);
        cipher
            .encrypt(
                n,
                Payload {
                    msg: plaintext,
                    aad,
                },
            )
            .map_err(|_| EncryptionError::DecryptFailed)
    }

    /// Decrypt ciphertext produced by [`IdentityAead::seal`].
    pub fn open(
        &self,
        nonce: &AeadNonce,
        ciphertext: &[u8],
        aad: &[u8],
    ) -> Result<Vec<u8>, EncryptionError> {
        let cipher = ChaCha20Poly1305::new(Key::from_slice(&self.key));
        let n = Nonce::from_slice(nonce);
        cipher
            .decrypt(
                n,
                Payload {
                    msg: ciphertext,
                    aad,
                },
            )
            .map_err(|_| EncryptionError::DecryptFailed)
    }
}

/// Build a deterministic demo nonce from a counter (tests / non-production only).
pub fn nonce_from_counter(counter: u64) -> AeadNonce {
    let mut nonce = [0u8; 12];
    nonce[4..].copy_from_slice(&counter.to_be_bytes());
    nonce
}

pub fn init_encryption_layer() {
    println!("Initializing Encryption Layer (identity AEAD ChaCha20-Poly1305 + DHKE helpers).");
    match perform_handshake() {
        Ok(()) => {}
        Err(e) => println!("DHKE demo handshake error: {}", e),
    }
    if let Ok(aead) = IdentityAead::derive(b"alice-id", b"bob-id", b"mesh-session") {
        let nonce = nonce_from_counter(1);
        if let Ok(ct) = aead.seal(&nonce, b"hello mesh", b"hdr") {
            println!("AEAD seal ok, ciphertext_len={}", ct.len());
        }
    }
}

/// Simulated Diffie-Hellman Key Exchange (DHKE) using prime modulus math.
/// Guarantees a forward-secrecy sketch: ephemeral keys generated per session.
/// `(base^private_key) % prime`
pub fn compute_dhke_public_key(base: u64, private_key: u64, prime: u64) -> u64 {
    let mut res = 1u64;
    let mut b = base % prime;
    let mut p = private_key;

    while p > 0 {
        if p % 2 == 1 {
            res = ((res as u128 * b as u128) % prime as u128) as u64;
        }
        p >>= 1;
        b = ((b as u128 * b as u128) % prime as u128) as u64;
    }
    res
}

/// Derive an AEAD key from a shared DHKE secret (low-entropy demo path).
pub fn aead_key_from_shared_secret(shared: u64, session_label: &[u8]) -> AeadKey {
    let mut hasher = Sha256::new();
    hasher.update(b"mossymesh/dhke-aead/v1");
    hasher.update(shared.to_be_bytes());
    hasher.update(session_label);
    let digest = hasher.finalize();
    let mut key = [0u8; 32];
    key.copy_from_slice(&digest);
    key
}

pub fn perform_handshake() -> Result<(), &'static str> {
    let prime = 23;
    let base = 5;

    let alice_private = 4;
    let alice_public = compute_dhke_public_key(base, alice_private, prime);

    let bob_private = 3;
    let bob_public = compute_dhke_public_key(base, bob_private, prime);

    let alice_shared = compute_dhke_public_key(bob_public, alice_private, prime);
    let bob_shared = compute_dhke_public_key(alice_public, bob_private, prime);

    if alice_shared == bob_shared {
        println!("DHKE Handshake Successful: Shared Secret is {}", alice_shared);
        Ok(())
    } else {
        Err("DHKE Handshake failed to produce symmetric key.")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_aead_roundtrip() {
        let aead = IdentityAead::derive(b"node-A", b"node-B", b"job-42").unwrap();
        let nonce = nonce_from_counter(7);
        let pt = b"ephemeral payload";
        let aad = b"mesh-header-v1";
        let ct = aead.seal(&nonce, pt, aad).unwrap();
        let opened = aead.open(&nonce, &ct, aad).unwrap();
        assert_eq!(opened, pt);
    }

    #[test]
    fn test_aead_rejects_tamper() {
        let aead = IdentityAead::derive(b"A", b"B", b"ctx").unwrap();
        let nonce = nonce_from_counter(1);
        let mut ct = aead.seal(&nonce, b"data", b"aad").unwrap();
        let last = ct.len() - 1;
        ct[last] ^= 0xff;
        assert_eq!(aead.open(&nonce, &ct, b"aad"), Err(EncryptionError::DecryptFailed));
    }

    #[test]
    fn test_aead_rejects_wrong_aad() {
        let aead = IdentityAead::derive(b"A", b"B", b"ctx").unwrap();
        let nonce = nonce_from_counter(2);
        let ct = aead.seal(&nonce, b"data", b"aad-correct").unwrap();
        assert_eq!(
            aead.open(&nonce, &ct, b"aad-wrong"),
            Err(EncryptionError::DecryptFailed)
        );
    }

    #[test]
    fn test_directional_keys_differ() {
        let ab = IdentityAead::derive(b"A", b"B", b"s").unwrap();
        let ba = IdentityAead::derive(b"B", b"A", b"s").unwrap();
        assert_ne!(ab.key_bytes(), ba.key_bytes());
    }

    #[test]
    fn test_dhke_shared_secret() {
        let prime = 23;
        let base = 5;
        let alice_pub = compute_dhke_public_key(base, 4, prime);
        let bob_pub = compute_dhke_public_key(base, 3, prime);
        let alice_shared = compute_dhke_public_key(bob_pub, 4, prime);
        let bob_shared = compute_dhke_public_key(alice_pub, 3, prime);
        assert_eq!(alice_shared, bob_shared);
        assert!(perform_handshake().is_ok());
    }

    #[test]
    fn test_empty_identity_rejected() {
        assert_eq!(
            IdentityAead::derive(b"", b"B", b"c").err(),
            Some(EncryptionError::EmptyIdentity)
        );
    }
}
