//! Onion-routed honeypot mechanisms to catch and slash malicious node cartels.
//!
//! Hubs silently replay historically verified jobs via Onion-Routed Honeypots.
//! Unproven cartels agreeing on fake hashes are instantly slashed and banned.

use std::collections::{HashMap, HashSet};

/// Default onion circuit hop count for honeypot job delivery.
pub const DEFAULT_ONION_HOPS: usize = 3;

/// Slash amount (basis points of staked collateral) applied on cartel detection.
pub const CARTEL_SLASH_BPS: u32 = 10_000; // 100% slash

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrapPacket {
    pub decoy_payload: Vec<u8>,
    /// Historical job id whose correct result hash is known to the hub only.
    pub job_id: u64,
    /// Expected honest result hash (never revealed to workers before completion).
    pub expected_hash: [u8; 32],
    /// Layered onion routing path (outer → inner).
    pub onion_path: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HoneypotVerdict {
    /// Submitted hash matches the known-good historical result.
    Honest,
    /// Submitted hash mismatches — candidate for individual slash.
    FakeHash,
    /// Multiple distinct peers submitted the *same* wrong hash → cartel.
    CartelCollusion,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlashRecord {
    pub peer_id: String,
    pub slash_bps: u32,
    pub banned: bool,
    pub reason: String,
}

/// Coordinates silent honeypot job injection and cartel detection.
#[derive(Debug, Default)]
pub struct HoneypotHub {
    /// job_id → expected honest hash
    traps: HashMap<u64, [u8; 32]>,
    /// job_id → peer_id → submitted hash
    submissions: HashMap<u64, HashMap<String, [u8; 32]>>,
    /// Permanently banned peer ids
    banned: HashSet<String>,
    /// Accumulated slash ledger
    slashes: Vec<SlashRecord>,
}

impl HoneypotHub {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_banned(&self, peer_id: &str) -> bool {
        self.banned.contains(peer_id)
    }

    pub fn slashes(&self) -> &[SlashRecord] {
        &self.slashes
    }

    /// Build an onion-routed trap from a historically verified job.
    pub fn craft_trap(
        &mut self,
        job_id: u64,
        expected_hash: [u8; 32],
        decoy_payload: Vec<u8>,
        relays: Vec<String>,
    ) -> TrapPacket {
        self.traps.insert(job_id, expected_hash);
        self.submissions.entry(job_id).or_default();
        TrapPacket {
            decoy_payload,
            job_id,
            expected_hash,
            onion_path: relays,
        }
    }

    /// Peel one onion layer — returns remaining path and whether delivery is final.
    pub fn peel_onion(packet: &TrapPacket) -> (Vec<String>, bool) {
        if packet.onion_path.is_empty() {
            return (vec![], true);
        }
        let mut rest = packet.onion_path.clone();
        rest.remove(0);
        let is_final = rest.is_empty();
        (rest, is_final)
    }

    /// Replay: a worker returns a claimed result hash for a honeypot job.
    pub fn submit_result(
        &mut self,
        job_id: u64,
        peer_id: &str,
        claimed_hash: [u8; 32],
    ) -> HoneypotVerdict {
        if self.banned.contains(peer_id) {
            return HoneypotVerdict::FakeHash;
        }

        let expected = match self.traps.get(&job_id).copied() {
            Some(h) => h,
            None => return HoneypotVerdict::FakeHash,
        };

        self.submissions
            .entry(job_id)
            .or_default()
            .insert(peer_id.to_string(), claimed_hash);

        if claimed_hash == expected {
            return HoneypotVerdict::Honest;
        }

        // Check for cartel: ≥2 peers agree on the same wrong hash.
        if self.detect_cartel(job_id, &claimed_hash) {
            self.slash_cartel(job_id, &claimed_hash);
            return HoneypotVerdict::CartelCollusion;
        }

        self.slash_peer(
            peer_id,
            CARTEL_SLASH_BPS,
            format!("honeypot fake hash on job {job_id}"),
        );
        HoneypotVerdict::FakeHash
    }

    fn detect_cartel(&self, job_id: u64, fake_hash: &[u8; 32]) -> bool {
        let Some(expected) = self.traps.get(&job_id) else {
            return false;
        };
        if fake_hash == expected {
            return false;
        }
        let Some(subs) = self.submissions.get(&job_id) else {
            return false;
        };
        let colluders = subs.values().filter(|h| *h == fake_hash).count();
        colluders >= 2
    }

    fn slash_cartel(&mut self, job_id: u64, fake_hash: &[u8; 32]) {
        let peers: Vec<String> = self
            .submissions
            .get(&job_id)
            .map(|subs| {
                subs.iter()
                    .filter(|(_, h)| *h == fake_hash)
                    .map(|(p, _)| p.clone())
                    .collect()
            })
            .unwrap_or_default();

        for peer in peers {
            self.slash_peer(
                &peer,
                CARTEL_SLASH_BPS,
                format!("cartel fake hash agreement on honeypot job {job_id}"),
            );
        }
    }

    fn slash_peer(&mut self, peer_id: &str, slash_bps: u32, reason: String) {
        self.banned.insert(peer_id.to_string());
        if self
            .slashes
            .iter()
            .any(|s| s.peer_id == peer_id && s.reason == reason)
        {
            return;
        }
        self.slashes.push(SlashRecord {
            peer_id: peer_id.to_string(),
            slash_bps,
            banned: true,
            reason,
        });
    }

    /// Number of onion hops configured on a trap.
    pub fn onion_depth(packet: &TrapPacket) -> usize {
        packet.onion_path.len()
    }
}

/// Deterministic decoy hash used when building synthetic honeypot payloads in tests.
pub fn decoy_result_hash(job_id: u64, tag: u8) -> [u8; 32] {
    let mut out = [0u8; 32];
    let bytes = job_id.to_le_bytes();
    for (i, b) in out.iter_mut().enumerate() {
        *b = bytes[i % 8]
            .wrapping_add(tag)
            .wrapping_mul(31)
            .wrapping_add(i as u8);
    }
    out
}

pub fn init_honeypot() {
    println!("Initializing Onion-routed Honeypots for anti-cartel enforcement.");
    let mut hub = HoneypotHub::new();
    let honest = decoy_result_hash(1, 0);
    let trap = hub.craft_trap(
        1,
        honest,
        b"historical-job-replay".to_vec(),
        vec!["relay-a".into(), "relay-b".into(), "relay-c".into()],
    );
    println!(
        "Honeypot trap job {} onion depth {}",
        trap.job_id,
        HoneypotHub::onion_depth(&trap)
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_honest_submission() {
        let mut hub = HoneypotHub::new();
        let expected = decoy_result_hash(7, 0);
        hub.craft_trap(7, expected, vec![1, 2, 3], vec!["r1".into()]);
        let v = hub.submit_result(7, "peer-honest", expected);
        assert_eq!(v, HoneypotVerdict::Honest);
        assert!(!hub.is_banned("peer-honest"));
        assert!(hub.slashes().is_empty());
    }

    #[test]
    fn test_solo_fake_hash_slash_ban() {
        let mut hub = HoneypotHub::new();
        let expected = decoy_result_hash(8, 0);
        let fake = decoy_result_hash(8, 9);
        hub.craft_trap(8, expected, vec![], vec!["r1".into(), "r2".into()]);
        let v = hub.submit_result(8, "liar", fake);
        assert_eq!(v, HoneypotVerdict::FakeHash);
        assert!(hub.is_banned("liar"));
        assert_eq!(hub.slashes().len(), 1);
        assert_eq!(hub.slashes()[0].slash_bps, CARTEL_SLASH_BPS);
    }

    #[test]
    fn test_cartel_fake_hash_agreement() {
        let mut hub = HoneypotHub::new();
        let expected = decoy_result_hash(9, 0);
        let cartel_hash = decoy_result_hash(9, 0xAB);
        hub.craft_trap(
            9,
            expected,
            b"trap".to_vec(),
            vec!["h1".into(), "h2".into(), "h3".into()],
        );

        let v1 = hub.submit_result(9, "cartel-a", cartel_hash);
        assert_eq!(v1, HoneypotVerdict::FakeHash);
        assert!(hub.is_banned("cartel-a"));

        let v2 = hub.submit_result(9, "cartel-b", cartel_hash);
        assert_eq!(v2, HoneypotVerdict::CartelCollusion);
        assert!(hub.is_banned("cartel-b"));

        assert!(hub.slashes().len() >= 2);
        assert!(hub.slashes().iter().any(|s| s.reason.contains("cartel")));
    }

    #[test]
    fn test_onion_peel() {
        let packet = TrapPacket {
            decoy_payload: vec![],
            job_id: 1,
            expected_hash: [0u8; 32],
            onion_path: vec!["a".into(), "b".into(), "c".into()],
        };
        assert_eq!(HoneypotHub::onion_depth(&packet), DEFAULT_ONION_HOPS);
        let (rest, final_hop) = HoneypotHub::peel_onion(&packet);
        assert!(!final_hop);
        assert_eq!(rest, vec!["b".to_string(), "c".to_string()]);

        let inner = TrapPacket {
            onion_path: rest,
            ..packet.clone()
        };
        let (rest2, _) = HoneypotHub::peel_onion(&inner);
        let last = TrapPacket {
            onion_path: rest2,
            ..packet
        };
        let (empty, is_final) = HoneypotHub::peel_onion(&last);
        assert!(is_final);
        assert!(empty.is_empty());
    }
}
