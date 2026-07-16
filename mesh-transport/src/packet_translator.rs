//! Smartphone / Wi-Fi packet ↔ LoRa frame fragmentation and reassembly.
//!
//! Phase 1 DoD: a smartphone test packet successfully translates to LoRa
//! transmission frames. Fragmentation is deterministic (fixed MTU, big-endian
//! headers) so reassembly yields byte-identical payloads across peers.

use crate::lora_mac::{self, LoraFrame, LORA_MAX_PAYLOAD};

/// Bytes reserved for the MossyMesh LoRa fragment header.
///
/// Layout (14 bytes):
/// ```text
/// 0      magic 0x4D4D ("MM")
/// 2      version = 1
/// 3      flags (bit0 = more_fragments)
/// 4..8   message_id (u32 BE)
/// 8..10  fragment_index (u16 BE)
/// 10..12 fragment_count (u16 BE)
/// 12..14 payload_len (u16 BE)
/// 14..   payload
/// ```
pub const FRAG_HEADER_LEN: usize = 14;

/// Maximum application bytes per LoRa fragment.
pub const LORA_FRAG_MTU: usize = LORA_MAX_PAYLOAD - FRAG_HEADER_LEN;

const MAGIC: [u8; 2] = [0x4D, 0x4D];
const VERSION: u8 = 1;
const FLAG_MORE: u8 = 0x01;

/// IP-to-Reticulum style mesh packet (identity destination + payload).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReticulumPacket {
    pub destination_hash: [u8; 16], // 128-bit truncated hash for fast routing
    pub data: Vec<u8>,
}

/// Smartphone / captive-portal side packet before mesh translation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SmartphonePacket {
    pub src_ip: String,
    pub dst_ip: String,
    pub payload: Vec<u8>,
}

/// One LoRa-bound fragment with header metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoRaFragment {
    pub message_id: u32,
    pub index: u16,
    pub count: u16,
    pub more: bool,
    pub payload: Vec<u8>,
}

impl LoRaFragment {
    pub fn encode(&self) -> Result<Vec<u8>, TranslateError> {
        if self.payload.len() > LORA_FRAG_MTU {
            return Err(TranslateError::FragmentTooLarge {
                len: self.payload.len(),
                max: LORA_FRAG_MTU,
            });
        }
        let mut out = Vec::with_capacity(FRAG_HEADER_LEN + self.payload.len());
        out.extend_from_slice(&MAGIC);
        out.push(VERSION);
        out.push(if self.more { FLAG_MORE } else { 0 });
        out.extend_from_slice(&self.message_id.to_be_bytes());
        out.extend_from_slice(&self.index.to_be_bytes());
        out.extend_from_slice(&self.count.to_be_bytes());
        out.extend_from_slice(&(self.payload.len() as u16).to_be_bytes());
        out.extend_from_slice(&self.payload);
        debug_assert!(out.len() <= LORA_MAX_PAYLOAD);
        Ok(out)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, TranslateError> {
        if bytes.len() < FRAG_HEADER_LEN {
            return Err(TranslateError::TruncatedHeader);
        }
        if bytes[0..2] != MAGIC {
            return Err(TranslateError::BadMagic);
        }
        if bytes[2] != VERSION {
            return Err(TranslateError::UnsupportedVersion(bytes[2]));
        }
        let flags = bytes[3];
        let message_id = u32::from_be_bytes(bytes[4..8].try_into().unwrap());
        let index = u16::from_be_bytes(bytes[8..10].try_into().unwrap());
        let count = u16::from_be_bytes(bytes[10..12].try_into().unwrap());
        let payload_len = u16::from_be_bytes(bytes[12..14].try_into().unwrap()) as usize;
        if FRAG_HEADER_LEN + payload_len > bytes.len() {
            return Err(TranslateError::TruncatedPayload);
        }
        if count == 0 || index >= count {
            return Err(TranslateError::InvalidFragmentIndex { index, count });
        }
        let payload = bytes[FRAG_HEADER_LEN..FRAG_HEADER_LEN + payload_len].to_vec();
        Ok(Self {
            message_id,
            index,
            count,
            more: (flags & FLAG_MORE) != 0,
            payload,
        })
    }

    /// Wrap as a CRC-protected LoRa MAC frame.
    pub fn to_lora_frame(&self) -> Result<LoraFrame, TranslateError> {
        let bytes = self.encode()?;
        LoraFrame::new(bytes).map_err(|e| match e {
            lora_mac::LoraMacError::PayloadTooLarge { len, max } => {
                TranslateError::FragmentTooLarge { len, max }
            }
            other => TranslateError::Mac(format!("{:?}", other)),
        })
    }

    pub fn from_lora_frame(frame: &LoraFrame) -> Result<Self, TranslateError> {
        if !frame.verify_crc() {
            return Err(TranslateError::CrcMismatch);
        }
        Self::decode(&frame.payload)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TranslateError {
    UnroutableIp(String),
    EmptyPayload,
    FragmentTooLarge { len: usize, max: usize },
    TruncatedHeader,
    TruncatedPayload,
    BadMagic,
    UnsupportedVersion(u8),
    InvalidFragmentIndex { index: u16, count: u16 },
    IncompleteReassembly { have: usize, need: usize },
    CrcMismatch,
    InconsistentMessage,
    Mac(String),
}

/// Deterministic FNV-1a 32-bit hash for message ids (no OS entropy).
pub fn fnv1a32(data: &[u8]) -> u32 {
    let mut hash: u32 = 0x811c_9dc5;
    for &b in data {
        hash ^= b as u32;
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash
}

/// Derive a stable message id from destination + full payload.
pub fn message_id_for(dest: &[u8; 16], payload: &[u8]) -> u32 {
    let mut buf = Vec::with_capacity(16 + payload.len());
    buf.extend_from_slice(dest);
    buf.extend_from_slice(payload);
    fnv1a32(&buf)
}

/// A simulated IP-to-Mesh mapping table. When a device accesses an IP like 192.168.4.1,
/// the translator catches the TCP packet and wraps it in a ReticulumPacket destined for the
/// specific node's PeerID.
pub fn translate_ip_to_mesh(ip_string: &str, raw_tcp_data: Vec<u8>) -> Option<ReticulumPacket> {
    let dest_hash = match ip_string {
        "192.168.4.1" => [0x01; 16], // Route to Group Owner
        "192.168.4.2" => [0x02; 16], // Route to Client A
        "192.168.4.3" => [0x03; 16],
        _ => return None, // Drop unroutable packets
    };

    Some(ReticulumPacket {
        destination_hash: dest_hash,
        data: raw_tcp_data,
    })
}

/// Translate a smartphone packet into a mesh (Reticulum) packet via captive-portal IP map.
pub fn smartphone_to_mesh(pkt: &SmartphonePacket) -> Result<ReticulumPacket, TranslateError> {
    translate_ip_to_mesh(&pkt.dst_ip, pkt.payload.clone())
        .ok_or_else(|| TranslateError::UnroutableIp(pkt.dst_ip.clone()))
}

/// Fragment an arbitrary byte blob into LoRa-sized fragments (deterministic order).
pub fn fragment_payload(
    message_id: u32,
    payload: &[u8],
) -> Result<Vec<LoRaFragment>, TranslateError> {
    if payload.is_empty() {
        return Err(TranslateError::EmptyPayload);
    }
    let mtu = LORA_FRAG_MTU;
    let count = ((payload.len() + mtu - 1) / mtu) as u16;
    if count == 0 {
        return Err(TranslateError::EmptyPayload);
    }
    let mut frags = Vec::with_capacity(count as usize);
    for index in 0..count {
        let start = (index as usize) * mtu;
        let end = (start + mtu).min(payload.len());
        frags.push(LoRaFragment {
            message_id,
            index,
            count,
            more: index + 1 < count,
            payload: payload[start..end].to_vec(),
        });
    }
    Ok(frags)
}

/// Fragment a mesh packet for LoRa TX. Returns CRC'd LoRa frames in order.
pub fn mesh_to_lora_frames(pkt: &ReticulumPacket) -> Result<Vec<LoraFrame>, TranslateError> {
    // Wire format of one mesh datagram over LoRa: dest(16) || data
    let mut blob = Vec::with_capacity(16 + pkt.data.len());
    blob.extend_from_slice(&pkt.destination_hash);
    blob.extend_from_slice(&pkt.data);
    let mid = message_id_for(&pkt.destination_hash, &pkt.data);
    let frags = fragment_payload(mid, &blob)?;
    frags.iter().map(|f| f.to_lora_frame()).collect()
}

/// Full Phase-1 path: smartphone packet → mesh → LoRa frames.
pub fn smartphone_to_lora_frames(
    pkt: &SmartphonePacket,
) -> Result<Vec<LoraFrame>, TranslateError> {
    let mesh = smartphone_to_mesh(pkt)?;
    mesh_to_lora_frames(&mesh)
}

/// Reassembly buffer for a single message id.
#[derive(Debug, Clone)]
pub struct ReassemblyBuffer {
    pub message_id: u32,
    pub count: u16,
    slots: Vec<Option<Vec<u8>>>,
}

impl ReassemblyBuffer {
    pub fn new(message_id: u32, count: u16) -> Self {
        Self {
            message_id,
            count,
            slots: vec![None; count as usize],
        }
    }

    pub fn accept(&mut self, frag: &LoRaFragment) -> Result<bool, TranslateError> {
        if frag.message_id != self.message_id {
            return Err(TranslateError::InconsistentMessage);
        }
        if frag.count != self.count {
            return Err(TranslateError::InconsistentMessage);
        }
        if frag.index as usize >= self.slots.len() {
            return Err(TranslateError::InvalidFragmentIndex {
                index: frag.index,
                count: self.count,
            });
        }
        // Idempotent: same index must carry identical payload (determinism check).
        if let Some(ref existing) = self.slots[frag.index as usize] {
            if existing != &frag.payload {
                return Err(TranslateError::InconsistentMessage);
            }
        } else {
            self.slots[frag.index as usize] = Some(frag.payload.clone());
        }
        Ok(self.is_complete())
    }

    pub fn is_complete(&self) -> bool {
        self.slots.iter().all(|s| s.is_some())
    }

    pub fn have_count(&self) -> usize {
        self.slots.iter().filter(|s| s.is_some()).count()
    }

    pub fn assemble(&self) -> Result<Vec<u8>, TranslateError> {
        if !self.is_complete() {
            return Err(TranslateError::IncompleteReassembly {
                have: self.have_count(),
                need: self.count as usize,
            });
        }
        let mut out = Vec::new();
        for slot in &self.slots {
            out.extend_from_slice(slot.as_ref().unwrap());
        }
        Ok(out)
    }
}

/// Reassemble ordered or unordered LoRa fragments into a mesh packet.
pub fn reassemble_mesh_packet(
    frames: &[LoraFrame],
) -> Result<ReticulumPacket, TranslateError> {
    if frames.is_empty() {
        return Err(TranslateError::EmptyPayload);
    }
    let mut frags: Vec<LoRaFragment> = frames
        .iter()
        .map(LoRaFragment::from_lora_frame)
        .collect::<Result<Vec<_>, _>>()?;

    // Sort by index for determinism regardless of delivery order.
    frags.sort_by_key(|f| f.index);

    let first = &frags[0];
    let mut buf = ReassemblyBuffer::new(first.message_id, first.count);
    for f in &frags {
        buf.accept(f)?;
    }
    let blob = buf.assemble()?;
    if blob.len() < 16 {
        return Err(TranslateError::TruncatedPayload);
    }
    let mut destination_hash = [0u8; 16];
    destination_hash.copy_from_slice(&blob[..16]);
    let data = blob[16..].to_vec();
    Ok(ReticulumPacket {
        destination_hash,
        data,
    })
}

/// Round-trip helper used by tests and simulators.
pub fn translate_roundtrip(pkt: &SmartphonePacket) -> Result<ReticulumPacket, TranslateError> {
    let frames = smartphone_to_lora_frames(pkt)?;
    reassemble_mesh_packet(&frames)
}

pub fn init_packet_translator() {
    println!("Initializing Packet Translator for IP-to-Identity routing mapping.");
    let phone = SmartphonePacket {
        src_ip: "192.168.4.10".into(),
        dst_ip: "192.168.4.1".into(),
        payload: b"GET /chess HTTP/1.1".to_vec(),
    };
    match smartphone_to_lora_frames(&phone) {
        Ok(frames) => {
            println!(
                "Smartphone→LoRa: {} frame(s), first {}B CRC ok={}",
                frames.len(),
                frames[0].payload.len(),
                frames[0].verify_crc()
            );
        }
        Err(e) => println!("Smartphone→LoRa failed: {:?}", e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_phone(payload: Vec<u8>) -> SmartphonePacket {
        SmartphonePacket {
            src_ip: "192.168.4.50".into(),
            dst_ip: "192.168.4.1".into(),
            payload,
        }
    }

    #[test]
    fn phase1_smartphone_packet_translates_to_lora() {
        let phone = sample_phone(b"test-packet-from-phone".to_vec());
        let frames = smartphone_to_lora_frames(&phone).unwrap();
        assert!(!frames.is_empty());
        for f in &frames {
            assert!(f.verify_crc());
            assert!(f.payload.len() <= LORA_MAX_PAYLOAD);
            let frag = LoRaFragment::from_lora_frame(f).unwrap();
            assert_eq!(frag.message_id, frames_message_id(&frames));
        }
    }

    fn frames_message_id(frames: &[LoraFrame]) -> u32 {
        LoRaFragment::from_lora_frame(&frames[0]).unwrap().message_id
    }

    #[test]
    fn fragmentation_reassembly_determinism() {
        // Payload larger than one LoRa fragment → multiple frames
        let big = (0u8..200).cycle().take(600).collect::<Vec<_>>();
        let phone = sample_phone(big.clone());
        let frames_a = smartphone_to_lora_frames(&phone).unwrap();
        let frames_b = smartphone_to_lora_frames(&phone).unwrap();
        assert!(frames_a.len() > 1);
        assert_eq!(frames_a, frames_b, "fragmentation must be deterministic");

        let mesh = reassemble_mesh_packet(&frames_a).unwrap();
        assert_eq!(mesh.destination_hash, [0x01; 16]);
        assert_eq!(mesh.data, big);
    }

    #[test]
    fn reassembly_order_independent() {
        let payload = (0u8..=255).cycle().take(500).collect::<Vec<_>>();
        let phone = sample_phone(payload.clone());
        let mut frames = smartphone_to_lora_frames(&phone).unwrap();
        assert!(frames.len() >= 2);
        frames.reverse();
        // Also swap middle if present
        if frames.len() > 2 {
            frames.swap(0, 1);
        }
        let mesh = reassemble_mesh_packet(&frames).unwrap();
        assert_eq!(mesh.data, payload);
    }

    #[test]
    fn roundtrip_small_and_exact_mtu_boundary() {
        for len in [1usize, LORA_FRAG_MTU - 16, LORA_FRAG_MTU, LORA_FRAG_MTU + 1, 1000] {
            // mesh blob = 16 dest + data; fragmenter sees dest||data
            let data = vec![0xABu8; len];
            let phone = sample_phone(data.clone());
            let out = translate_roundtrip(&phone).unwrap();
            assert_eq!(out.data, data, "len={len}");
        }
    }

    #[test]
    fn unroutable_ip_rejected() {
        let phone = SmartphonePacket {
            src_ip: "10.0.0.1".into(),
            dst_ip: "8.8.8.8".into(),
            payload: b"x".to_vec(),
        };
        assert!(matches!(
            smartphone_to_mesh(&phone),
            Err(TranslateError::UnroutableIp(_))
        ));
    }

    #[test]
    fn message_id_stable() {
        let dest = [0x01; 16];
        let p = b"stable";
        assert_eq!(message_id_for(&dest, p), message_id_for(&dest, p));
        assert_ne!(message_id_for(&dest, p), message_id_for(&dest, b"other"));
    }

    #[test]
    fn crc_failure_detected() {
        let phone = sample_phone(b"hello".to_vec());
        let mut frames = smartphone_to_lora_frames(&phone).unwrap();
        frames[0].payload[0] ^= 0xFF;
        // CRC in frame struct is stale relative to corrupted payload
        assert!(!frames[0].verify_crc());
        assert!(matches!(
            reassemble_mesh_packet(&frames),
            Err(TranslateError::CrcMismatch)
        ));
    }

    #[test]
    fn fragment_header_roundtrip() {
        let f = LoRaFragment {
            message_id: 0xDEAD_BEEF,
            index: 1,
            count: 3,
            more: true,
            payload: b"abc".to_vec(),
        };
        let enc = f.encode().unwrap();
        let dec = LoRaFragment::decode(&enc).unwrap();
        assert_eq!(f, dec);
    }

    #[test]
    fn incomplete_reassembly_errors() {
        let phone = sample_phone(vec![1u8; 500]);
        let frames = smartphone_to_lora_frames(&phone).unwrap();
        assert!(frames.len() > 1);
        let partial = &frames[..frames.len() - 1];
        assert!(matches!(
            reassemble_mesh_packet(partial),
            Err(TranslateError::IncompleteReassembly { .. })
        ));
    }
}
