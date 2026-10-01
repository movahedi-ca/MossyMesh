//! BLE link authentication and frame codec.
//!
//! Issue #155 defect 3: anyone spoofing a `MossyMesh-*` peripheral could get a
//! central to relay mesh frames through it. This module implements mutual
//! challenge-response authentication over GATT, keyed by a provisioned island
//! secret, plus the framed message codec carried inside the 20-byte ATT
//! fragments defined in [`crate::ble_mesh`].
//!
//! Protocol (per connection, all frames fragmented to [`crate::ble_hal::ATT_MTU_LEGACY`]):
//!
//! ```text
//!   peripheral (A)              central (B)
//!        |--- Hello(ch_a) ------------>|  notify on RelayRx
//!        |<-- AuthInit(ch_b) ----------|  write on RelayTx
//!        |--- AuthProof(mac_a) ------->|  mac_a = MAC(key, A-role | ch_a | ch_b)
//!        |<-- AuthProof(mac_b) --------|  mac_b = MAC(key, B-role | ch_b | ch_a)
//!        |--- AuthOk ----------------->|  only now is the link trusted
//!        |<-- LSA / LSA -------------->|  link-state exchange
//! ```
//!
//! Challenges are fresh per connection, direction labels prevent reflection
//! attacks, and no LSA or relayed frame is accepted before `AuthOk`.
//!
//! The MAC is HMAC-SHA256 over the `sha2` crate (already a dependency),
//! implemented here directly with RFC 4231 known-answer tests. The island
//! secret is provisioned out of band (island CA); an all-zero secret means
//! "unprovisioned" and the peripheral refuses to run.

use sha2::{Digest, Sha256};

use crate::ble_mesh::LinkStateAdvertisement;

/// Domain separation for the authentication MAC.
pub const AUTH_DOMAIN: &[u8] = b"mossymesh/ble-auth/v1";

/// Role label mixed into the peripheral's proof (prevents reflection).
pub const ROLE_PERIPHERAL: u8 = 0xA1;
/// Role label mixed into the central's proof.
pub const ROLE_CENTRAL: u8 = 0xB1;

/// Challenge length in bytes.
pub const CHALLENGE_LEN: usize = 32;
/// MAC length in bytes (HMAC-SHA256).
pub const MAC_LEN: usize = 32;

/// Maximum node-name bytes carried in handshake frames.
pub const MAX_NODE_NAME_LEN: usize = 64;

/// An all-zero island secret means "not provisioned".
pub fn is_provisioned(secret: &[u8; 32]) -> bool {
    secret.iter().any(|&b| b != 0)
}

/// HMAC-SHA256 implemented over the `sha2` crate (RFC 2104).
pub fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; MAC_LEN] {
    const BLOCK: usize = 64;
    let mut k = [0u8; BLOCK];
    if key.len() > BLOCK {
        let digest = Sha256::digest(key);
        k[..32].copy_from_slice(&digest);
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let mut ipad = [0x36u8; BLOCK];
    let mut opad = [0x5cu8; BLOCK];
    for i in 0..BLOCK {
        ipad[i] ^= k[i];
        opad[i] ^= k[i];
    }
    let mut inner = Sha256::new();
    inner.update(ipad);
    inner.update(message);
    let inner_digest = inner.finalize();
    let mut outer = Sha256::new();
    outer.update(opad);
    outer.update(inner_digest);
    let out = outer.finalize();
    let mut mac = [0u8; MAC_LEN];
    mac.copy_from_slice(&out);
    mac
}

/// Compute one side's authentication proof.
///
/// Binds the provisioned secret to both fresh challenges and the speaker's
/// role, so a captured proof cannot be reflected back or replayed on another
/// connection.
pub fn auth_proof(
    island_secret: &[u8; 32],
    role: u8,
    own_challenge: &[u8; CHALLENGE_LEN],
    peer_challenge: &[u8; CHALLENGE_LEN],
) -> [u8; MAC_LEN] {
    let mut msg = Vec::with_capacity(AUTH_DOMAIN.len() + 1 + 2 * CHALLENGE_LEN);
    msg.extend_from_slice(AUTH_DOMAIN);
    msg.push(role);
    msg.extend_from_slice(own_challenge);
    msg.extend_from_slice(peer_challenge);
    hmac_sha256(island_secret, &msg)
}

/// Verify a peer's proof by recomputing the expected MAC (constant shape,
/// variable-time compare is unnecessary here: the MAC is not secret).
pub fn verify_proof(
    island_secret: &[u8; 32],
    peer_role: u8,
    peer_challenge: &[u8; CHALLENGE_LEN],
    own_challenge: &[u8; CHALLENGE_LEN],
    proof: &[u8; MAC_LEN],
) -> bool {
    let expected = auth_proof(island_secret, peer_role, peer_challenge, own_challenge);
    expected == *proof
}

// ---------------------------------------------------------------------------
// Frame codec
// ---------------------------------------------------------------------------

/// Frame type tags on the wire.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameTag {
    Hello = 0x10,
    AuthInit = 0x11,
    AuthProof = 0x12,
    AuthOk = 0x13,
    AuthReject = 0x14,
    Lsa = 0x20,
    Bye = 0x22,
}

impl FrameTag {
    fn from_u8(b: u8) -> Option<Self> {
        match b {
            0x10 => Some(FrameTag::Hello),
            0x11 => Some(FrameTag::AuthInit),
            0x12 => Some(FrameTag::AuthProof),
            0x13 => Some(FrameTag::AuthOk),
            0x14 => Some(FrameTag::AuthReject),
            0x20 => Some(FrameTag::Lsa),
            0x22 => Some(FrameTag::Bye),
            _ => None,
        }
    }
}

/// Why authentication was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum AuthRejectReason {
    BadProof = 0x01,
    Unprovisioned = 0x02,
    ProtocolViolation = 0x03,
}

impl AuthRejectReason {
    fn from_u8(b: u8) -> Option<Self> {
        match b {
            0x01 => Some(AuthRejectReason::BadProof),
            0x02 => Some(AuthRejectReason::Unprovisioned),
            0x03 => Some(AuthRejectReason::ProtocolViolation),
            _ => None,
        }
    }
}

/// One decoded protocol frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MeshFrame {
    Hello {
        node_name: String,
        battery_level: u8,
        challenge: [u8; CHALLENGE_LEN],
    },
    AuthInit {
        node_name: String,
        battery_level: u8,
        challenge: [u8; CHALLENGE_LEN],
    },
    AuthProof {
        mac: [u8; MAC_LEN],
    },
    AuthOk,
    AuthReject {
        reason: AuthRejectReason,
    },
    Lsa(LinkStateAdvertisement),
    Bye,
}

/// Codec errors: malformed or out-of-policy frames.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameError {
    Empty,
    UnknownTag(u8),
    Truncated,
    BadUtf8,
    NameTooLong(usize),
    BadLsa,
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FrameError::Empty => write!(f, "empty frame"),
            FrameError::UnknownTag(t) => write!(f, "unknown frame tag {t:#04x}"),
            FrameError::Truncated => write!(f, "truncated frame"),
            FrameError::BadUtf8 => write!(f, "node name is not valid UTF-8"),
            FrameError::NameTooLong(n) => write!(f, "node name {n} bytes exceeds limit"),
            FrameError::BadLsa => write!(f, "LSA payload failed to decode"),
        }
    }
}

impl std::error::Error for FrameError {}

fn encode_name(out: &mut Vec<u8>, name: &str) {
    let b = name.as_bytes();
    out.push(b.len() as u8);
    out.extend_from_slice(b);
}

fn decode_name(bytes: &[u8], i: &mut usize) -> Result<String, FrameError> {
    if *i >= bytes.len() {
        return Err(FrameError::Truncated);
    }
    let len = bytes[*i] as usize;
    *i += 1;
    if len > MAX_NODE_NAME_LEN {
        return Err(FrameError::NameTooLong(len));
    }
    if *i + len > bytes.len() {
        return Err(FrameError::Truncated);
    }
    let s = std::str::from_utf8(&bytes[*i..*i + len]).map_err(|_| FrameError::BadUtf8)?;
    *i += len;
    Ok(s.to_string())
}

fn decode_fixed<const N: usize>(bytes: &[u8], i: &mut usize) -> Result<[u8; N], FrameError> {
    if *i + N > bytes.len() {
        return Err(FrameError::Truncated);
    }
    let mut arr = [0u8; N];
    arr.copy_from_slice(&bytes[*i..*i + N]);
    *i += N;
    Ok(arr)
}

impl MeshFrame {
    /// Encode to the assembled (pre-fragmentation) wire bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        match self {
            MeshFrame::Hello {
                node_name,
                battery_level,
                challenge,
            } => {
                out.push(FrameTag::Hello as u8);
                encode_name(&mut out, node_name);
                out.push(*battery_level);
                out.extend_from_slice(challenge);
            }
            MeshFrame::AuthInit {
                node_name,
                battery_level,
                challenge,
            } => {
                out.push(FrameTag::AuthInit as u8);
                encode_name(&mut out, node_name);
                out.push(*battery_level);
                out.extend_from_slice(challenge);
            }
            MeshFrame::AuthProof { mac } => {
                out.push(FrameTag::AuthProof as u8);
                out.extend_from_slice(mac);
            }
            MeshFrame::AuthOk => out.push(FrameTag::AuthOk as u8),
            MeshFrame::AuthReject { reason } => {
                out.push(FrameTag::AuthReject as u8);
                out.push(*reason as u8);
            }
            MeshFrame::Lsa(lsa) => {
                out.push(FrameTag::Lsa as u8);
                out.extend_from_slice(&lsa.encode());
            }
            MeshFrame::Bye => out.push(FrameTag::Bye as u8),
        }
        out
    }

    /// Decode assembled wire bytes. Returns `Err` on any malformed input.
    pub fn decode(bytes: &[u8]) -> Result<Self, FrameError> {
        if bytes.is_empty() {
            return Err(FrameError::Empty);
        }
        let tag = FrameTag::from_u8(bytes[0]).ok_or(FrameError::UnknownTag(bytes[0]))?;
        let mut i = 1usize;
        match tag {
            FrameTag::Hello => {
                let node_name = decode_name(bytes, &mut i)?;
                let battery_level = *bytes.get(i).ok_or(FrameError::Truncated)?;
                i += 1;
                let challenge = decode_fixed(bytes, &mut i)?;
                Ok(MeshFrame::Hello {
                    node_name,
                    battery_level,
                    challenge,
                })
            }
            FrameTag::AuthInit => {
                let node_name = decode_name(bytes, &mut i)?;
                let battery_level = *bytes.get(i).ok_or(FrameError::Truncated)?;
                i += 1;
                let challenge = decode_fixed(bytes, &mut i)?;
                Ok(MeshFrame::AuthInit {
                    node_name,
                    battery_level,
                    challenge,
                })
            }
            FrameTag::AuthProof => {
                let mac = decode_fixed(bytes, &mut i)?;
                Ok(MeshFrame::AuthProof { mac })
            }
            FrameTag::AuthOk => Ok(MeshFrame::AuthOk),
            FrameTag::AuthReject => {
                let b = *bytes.get(i).ok_or(FrameError::Truncated)?;
                let reason = AuthRejectReason::from_u8(b).ok_or(FrameError::Truncated)?;
                Ok(MeshFrame::AuthReject { reason })
            }
            FrameTag::Lsa => {
                let lsa = LinkStateAdvertisement::decode(&bytes[i..]).ok_or(FrameError::BadLsa)?;
                Ok(MeshFrame::Lsa(lsa))
            }
            FrameTag::Bye => Ok(MeshFrame::Bye),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 4231 test case 1: key = 20 x 0x0b, data = "Hi There".
    #[test]
    fn hmac_sha256_rfc4231_case1() {
        let key = [0x0bu8; 20];
        let mac = hmac_sha256(&key, b"Hi There");
        assert_eq!(
            hex(&mac),
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
    }

    /// RFC 4231 test case 2: key = "Jefe", data = "what do ya want for nothing?".
    #[test]
    fn hmac_sha256_rfc4231_case2() {
        let mac = hmac_sha256(b"Jefe", b"what do ya want for nothing?");
        // Cross-checked against Python hmac.new(b"Jefe", data, hashlib.sha256).
        assert_eq!(
            hex(&mac),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }

    /// RFC 4231 test case 3: key = 20 x 0xaa, data = 50 x 0xdd.
    #[test]
    fn hmac_sha256_rfc4231_case3() {
        let key = [0xaau8; 20];
        let data = [0xddu8; 50];
        let mac = hmac_sha256(&key, &data);
        // Cross-checked against Python hmac.
        assert_eq!(
            hex(&mac),
            "773ea91e36800e46854db8ebd09181a72959098b3ef8c122d9635514ced565fe"
        );
    }

    /// RFC 4231 test case 6: 131-byte key (larger than the 64-byte block).
    #[test]
    fn hmac_sha256_rfc4231_case6_long_key() {
        let key = [0xaau8; 131];
        let mac = hmac_sha256(
            &key,
            b"Test Using Larger Than Block-Sized Key - Hash Key First",
        );
        // Cross-checked against Python hmac.
        assert_eq!(
            hex(&mac),
            "ad47763c29284e9399feab51e0e79c9c5869cd06617d14fe69f0af19ffbd51cd"
        );
    }

    #[test]
    fn auth_proof_roundtrip_and_tamper() {
        let secret = [0x42u8; 32];
        let ch_a = [0x11u8; 32];
        let ch_b = [0x22u8; 32];
        let mac = auth_proof(&secret, ROLE_PERIPHERAL, &ch_a, &ch_b);
        assert!(verify_proof(&secret, ROLE_PERIPHERAL, &ch_a, &ch_b, &mac));
        // Wrong role label must not verify (reflection resistance).
        assert!(!verify_proof(&secret, ROLE_CENTRAL, &ch_a, &ch_b, &mac));
        // Swapped challenges must not verify.
        assert!(!verify_proof(&secret, ROLE_PERIPHERAL, &ch_b, &ch_a, &mac));
        // Tampered MAC must not verify.
        let mut bad = mac;
        bad[0] ^= 0x01;
        assert!(!verify_proof(&secret, ROLE_PERIPHERAL, &ch_a, &ch_b, &bad));
        // Wrong secret must not verify.
        let other = [0x43u8; 32];
        assert!(!verify_proof(&other, ROLE_PERIPHERAL, &ch_a, &ch_b, &mac));
    }

    #[test]
    fn frame_codec_roundtrips() {
        let frames = vec![
            MeshFrame::Hello {
                node_name: "node-7".into(),
                battery_level: 88,
                challenge: [0xAA; 32],
            },
            MeshFrame::AuthInit {
                node_name: "peer-9".into(),
                battery_level: 61,
                challenge: [0xBB; 32],
            },
            MeshFrame::AuthProof { mac: [0xCC; 32] },
            MeshFrame::AuthOk,
            MeshFrame::AuthReject {
                reason: AuthRejectReason::BadProof,
            },
            MeshFrame::Lsa(LinkStateAdvertisement::new(
                "peer-9",
                3,
                70,
                vec![("x".into(), 2)],
                1000,
            )),
            MeshFrame::Bye,
        ];
        for f in frames {
            let bytes = f.encode();
            let back = MeshFrame::decode(&bytes).expect("roundtrip");
            assert_eq!(f, back);
        }
    }

    #[test]
    fn frame_decode_rejects_garbage() {
        assert_eq!(MeshFrame::decode(&[]), Err(FrameError::Empty));
        assert_eq!(
            MeshFrame::decode(&[0xFF]),
            Err(FrameError::UnknownTag(0xFF))
        );
        // Truncated Hello (tag + short name length, no body).
        assert_eq!(
            MeshFrame::decode(&[0x10, 0x05, b'a', b'b']),
            Err(FrameError::Truncated)
        );
        // Name length exceeds policy.
        let mut evil = vec![0x10, 200];
        assert_eq!(MeshFrame::decode(&evil), Err(FrameError::NameTooLong(200)));
        evil.extend_from_slice(&[0u8; 200]);
        // Garbage LSA bytes.
        assert_eq!(
            MeshFrame::decode(&[0x20, 0xFF, 0xFF]),
            Err(FrameError::BadLsa)
        );
        // Invalid UTF-8 node name.
        let bad_utf8 = vec![0x11, 0x02, 0xFF, 0xFE];
        assert_eq!(MeshFrame::decode(&bad_utf8), Err(FrameError::BadUtf8));
        // Unknown reject reason.
        assert_eq!(MeshFrame::decode(&[0x14, 0x09]), Err(FrameError::Truncated));
    }

    #[test]
    fn unprovisioned_secret_detected() {
        assert!(!is_provisioned(&[0u8; 32]));
        let mut s = [0u8; 32];
        s[31] = 1;
        assert!(is_provisioned(&s));
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }
}
