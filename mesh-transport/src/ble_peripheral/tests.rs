//! Unit tests for [`super::BlePeripheral`] using a scripted in-memory HAL.
//!
//! The [`ScriptedHal`] below is a *test double*: it implements the HAL trait
//! with queued events and recorded calls so the state machine can be driven
//! deterministically. The faithful two-endpoint simulated link lives in the
//! integration tests (`mesh-transport/tests/ble_peripheral_integration.rs`).

use super::*;
use crate::ble_auth::{
    auth_proof, AuthRejectReason, MeshFrame, CHALLENGE_LEN, ROLE_CENTRAL, ROLE_PERIPHERAL,
};
use crate::ble_hal::{
    AdvertiseParams, BleHalError, BlePeripheralHal, ConnectionId, GattChar, HalEvent,
    ATT_MTU_LEGACY, DEVICE_NAME_PREFIX, MESH_RELAY_SERVICE_UUID,
};
use crate::ble_mesh::{
    fragment_ble_payload, BleMeshNode, BleRelayFragment, BleRelayReassembler,
    LinkStateAdvertisement,
};
use std::collections::VecDeque;

const SECRET: [u8; 32] = [0x5E; 32];
const WRONG_SECRET: [u8; 32] = [0xE5; 32];
const CONN: ConnectionId = ConnectionId(7);

// ---------------------------------------------------------------------------
// Scripted HAL test double
// ---------------------------------------------------------------------------

struct ScriptedHal {
    advertising: bool,
    adv_params: Option<AdvertiseParams>,
    events: VecDeque<HalEvent>,
    notifies: Vec<(ConnectionId, GattChar, Vec<u8>)>,
    disconnects: Vec<ConnectionId>,
    rng_state: u64,
    fail_notify: bool,
    fail_random: bool,
    fail_advertise: bool,
}

impl ScriptedHal {
    fn new() -> Self {
        Self {
            advertising: false,
            adv_params: None,
            events: VecDeque::new(),
            notifies: Vec::new(),
            disconnects: Vec::new(),
            rng_state: 0x1234_5678_9ABC_DEF1,
            fail_notify: false,
            fail_random: false,
            fail_advertise: false,
        }
    }
}

impl BlePeripheralHal for ScriptedHal {
    fn start_advertising(&mut self, params: &AdvertiseParams) -> Result<(), BleHalError> {
        if self.fail_advertise {
            return Err(BleHalError::Hardware("no radio".into()));
        }
        if self.advertising {
            return Err(BleHalError::AlreadyAdvertising);
        }
        self.advertising = true;
        self.adv_params = Some(params.clone());
        Ok(())
    }

    fn stop_advertising(&mut self) -> Result<(), BleHalError> {
        if !self.advertising {
            return Err(BleHalError::NotAdvertising);
        }
        self.advertising = false;
        Ok(())
    }

    fn poll_event(&mut self) -> Option<HalEvent> {
        self.events.pop_front()
    }

    fn notify(
        &mut self,
        conn: ConnectionId,
        char: GattChar,
        payload: &[u8],
    ) -> Result<(), BleHalError> {
        if payload.len() > ATT_MTU_LEGACY {
            return Err(BleHalError::PayloadTooLarge {
                len: payload.len(),
                max: ATT_MTU_LEGACY,
            });
        }
        if self.fail_notify {
            return Err(BleHalError::Hardware("tx path down".into()));
        }
        self.notifies.push((conn, char, payload.to_vec()));
        Ok(())
    }

    fn disconnect(&mut self, conn: ConnectionId) -> Result<(), BleHalError> {
        self.disconnects.push(conn);
        Ok(())
    }

    fn random_bytes(&mut self, out: &mut [u8]) -> Result<(), BleHalError> {
        if self.fail_random {
            return Err(BleHalError::RngFailure);
        }
        // xorshift64*: deterministic, nonzero seed.
        for b in out.iter_mut() {
            self.rng_state ^= self.rng_state >> 12;
            self.rng_state ^= self.rng_state << 25;
            self.rng_state ^= self.rng_state >> 27;
            *b = (self.rng_state.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 56) as u8;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Central-side transmit helper: fragments frames and queues them as writes.
struct CentralTx {
    next_id: u16,
}

impl CentralTx {
    fn new() -> Self {
        Self { next_id: 100 }
    }

    fn send(&mut self, p: &mut BlePeripheral<ScriptedHal>, conn: ConnectionId, frame: &MeshFrame) {
        let bytes = frame.encode();
        self.next_id = self.next_id.wrapping_add(1);
        for frag in fragment_ble_payload(self.next_id, &bytes) {
            self.send_raw(p, conn, GattChar::RelayTx, frag.encode());
        }
    }

    fn send_raw(
        &mut self,
        p: &mut BlePeripheral<ScriptedHal>,
        conn: ConnectionId,
        char: GattChar,
        payload: Vec<u8>,
    ) {
        p.hal_mut().events.push_back(HalEvent::GattWrite {
            conn,
            char,
            payload,
        });
    }
}

fn setup() -> (BlePeripheral<ScriptedHal>, CentralTx) {
    let control = BleMeshNode::new("periph-1", 88);
    let config = BlePeripheralConfig::new(SECRET);
    let p = BlePeripheral::new(ScriptedHal::new(), control, config).unwrap();
    (p, CentralTx::new())
}

/// Bring the peripheral to Advertising at t=0.
fn start(p: &mut BlePeripheral<ScriptedHal>) {
    p.tick(0);
    p.pump();
    assert_eq!(p.state(), PeripheralState::Advertising);
}

fn connect(p: &mut BlePeripheral<ScriptedHal>, conn: ConnectionId) {
    p.hal_mut().events.push_back(HalEvent::Connected {
        conn,
        rssi_dbm: Some(-55),
    });
    p.pump();
}

/// Reassemble everything the peripheral notified into decoded frames.
fn drain_frames(p: &mut BlePeripheral<ScriptedHal>, conn: ConnectionId) -> Vec<MeshFrame> {
    let notifies = std::mem::take(&mut p.hal_mut().notifies);
    let mut reasm = BleRelayReassembler::new();
    let mut out = Vec::new();
    for (c, ch, payload) in notifies {
        assert_eq!(c, conn, "notify went to the wrong connection");
        assert_eq!(ch, GattChar::RelayRx, "peripheral must notify on RelayRx");
        assert!(
            payload.len() <= ATT_MTU_LEGACY,
            "ATT ceiling violated: {} bytes",
            payload.len()
        );
        let frag = BleRelayFragment::decode(&payload).expect("valid fragment");
        if let Some(assembled) = reasm.accept(&frag) {
            out.push(MeshFrame::decode(&assembled).expect("valid frame"));
        }
    }
    out
}

fn find_hello(frames: &[MeshFrame]) -> (String, [u8; CHALLENGE_LEN]) {
    frames
        .iter()
        .find_map(|f| match f {
            MeshFrame::Hello {
                node_name,
                challenge,
                ..
            } => Some((node_name.clone(), *challenge)),
            _ => None,
        })
        .expect("Hello frame")
}

/// Drive the central side of a correct handshake. Returns (ch_a, ch_b).
fn central_handshake(
    p: &mut BlePeripheral<ScriptedHal>,
    tx: &mut CentralTx,
    conn: ConnectionId,
    secret: &[u8; 32],
    name: &str,
) -> ([u8; CHALLENGE_LEN], [u8; CHALLENGE_LEN]) {
    let frames = drain_frames(p, conn);
    let (node_name, ch_a) = find_hello(&frames);
    assert_eq!(node_name, "periph-1");

    let ch_b = [0xBBu8; 32];
    tx.send(
        p,
        conn,
        &MeshFrame::AuthInit {
            node_name: name.into(),
            battery_level: 61,
            challenge: ch_b,
        },
    );
    p.pump();

    let frames = drain_frames(p, conn);
    let mac_a = frames
        .iter()
        .find_map(|f| match f {
            MeshFrame::AuthProof { mac } => Some(*mac),
            _ => None,
        })
        .expect("peripheral AuthProof");
    assert_eq!(
        mac_a,
        auth_proof(secret, ROLE_PERIPHERAL, &ch_a, &ch_b),
        "peripheral proof must verify under the island secret"
    );

    let mac_b = auth_proof(secret, ROLE_CENTRAL, &ch_b, &ch_a);
    tx.send(p, conn, &MeshFrame::AuthProof { mac: mac_b });
    p.pump();
    (ch_a, ch_b)
}

fn lsa_from(name: &str, seq: u32) -> LinkStateAdvertisement {
    LinkStateAdvertisement::new(name, seq, 61, vec![("leaf".into(), 2)], 0)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn rejects_unprovisioned_secret() {
    let control = BleMeshNode::new("periph-1", 88);
    let err = BlePeripheral::new(
        ScriptedHal::new(),
        control,
        BlePeripheralConfig::new([0u8; 32]),
    )
    .err()
    .expect("unprovisioned peripheral must fail closed");
    assert_eq!(err, BleHalError::Unprovisioned);
}

#[test]
fn tick_starts_advertising_with_mesh_prefix_and_service_uuid() {
    let (mut p, _) = setup();
    assert_eq!(p.state(), PeripheralState::Idle);
    p.tick(0);
    assert_eq!(p.state(), PeripheralState::Advertising);
    let params = p.hal_mut().adv_params.clone().expect("advertising params");
    assert!(
        params.device_name.starts_with(DEVICE_NAME_PREFIX),
        "device name must carry the frontend's namePrefix filter"
    );
    assert!(params.device_name.contains("periph-1"));
    assert_eq!(params.service_uuid, MESH_RELAY_SERVICE_UUID);
}

#[test]
fn advertise_failure_stays_idle_and_retries() {
    let (mut p, _) = setup();
    p.hal_mut().fail_advertise = true;
    p.tick(0);
    assert_eq!(p.state(), PeripheralState::Idle);
    p.hal_mut().fail_advertise = false;
    p.tick(1);
    assert_eq!(p.state(), PeripheralState::Advertising);
}

#[test]
fn connect_sends_hello_with_fresh_challenge() {
    let (mut p, _) = setup();
    start(&mut p);
    connect(&mut p, CONN);
    assert_eq!(p.state(), PeripheralState::Authenticating);
    let (_, ch_a1) = find_hello(&drain_frames(&mut p, CONN));

    // Drop the link, reconnect: the challenge must be fresh.
    p.hal_mut().events.push_back(HalEvent::Disconnected {
        conn: CONN,
        reason: crate::ble_hal::DisconnectReason::PeerInitiated,
    });
    p.pump();
    p.tick(1);
    p.tick(1);
    assert_eq!(p.state(), PeripheralState::Advertising);
    connect(&mut p, CONN);
    let (_, ch_a2) = find_hello(&drain_frames(&mut p, CONN));
    assert_ne!(ch_a1, ch_a2, "challenge must be fresh per connection");
}

#[test]
fn full_handshake_reaches_active_and_registers_neighbor() {
    let (mut p, mut tx) = setup();
    start(&mut p);
    connect(&mut p, CONN);
    central_handshake(&mut p, &mut tx, CONN, &SECRET, "central-1");

    assert_eq!(p.state(), PeripheralState::Active);
    let frames = drain_frames(&mut p, CONN);
    assert!(
        frames.contains(&MeshFrame::AuthOk),
        "peripheral must confirm with AuthOk, got {frames:?}"
    );
    // hear_peer ran at auth success: the peer is a one-hop neighbor.
    let n = p.control().get_neighbor("central-1").expect("neighbor");
    assert_eq!(n.battery_level, 61);
    assert_eq!(p.stats().auth_success, 1);
    assert_eq!(p.stats().auth_failures, 0);
}

#[test]
fn wrong_secret_gets_reject_and_backoff() {
    let (mut p, mut tx) = setup();
    start(&mut p);
    connect(&mut p, CONN);
    let frames = drain_frames(&mut p, CONN);
    let (_, ch_a) = find_hello(&frames);

    let ch_b = [0xBBu8; 32];
    tx.send(
        &mut p,
        CONN,
        &MeshFrame::AuthInit {
            node_name: "rogue".into(),
            battery_level: 10,
            challenge: ch_b,
        },
    );
    p.pump();
    // Attacker cannot verify our proof (wrong secret) but sends one anyway.
    let bad_mac = auth_proof(&WRONG_SECRET, ROLE_CENTRAL, &ch_b, &ch_a);
    tx.send(&mut p, CONN, &MeshFrame::AuthProof { mac: bad_mac });
    p.pump();

    assert_eq!(p.state(), PeripheralState::Backoff);
    let frames = drain_frames(&mut p, CONN);
    assert!(frames.contains(&MeshFrame::AuthReject {
        reason: AuthRejectReason::BadProof
    }));
    assert_eq!(p.hal_mut().disconnects, vec![CONN]);
    assert_eq!(p.stats().auth_failures, 1);
    assert!(p.control().get_neighbor("rogue").is_none());
}

#[test]
fn tampered_proof_rejected() {
    let (mut p, mut tx) = setup();
    start(&mut p);
    connect(&mut p, CONN);
    let (ch_a, ch_b) = {
        let frames = drain_frames(&mut p, CONN);
        let (_, ch_a) = find_hello(&frames);
        let ch_b = [0xBBu8; 32];
        tx.send(
            &mut p,
            CONN,
            &MeshFrame::AuthInit {
                node_name: "central-1".into(),
                battery_level: 61,
                challenge: ch_b,
            },
        );
        p.pump();
        let _ = drain_frames(&mut p, CONN);
        (ch_a, ch_b)
    };
    let mut mac = auth_proof(&SECRET, ROLE_CENTRAL, &ch_b, &ch_a);
    mac[0] ^= 0x01;
    tx.send(&mut p, CONN, &MeshFrame::AuthProof { mac });
    p.pump();
    assert_eq!(p.state(), PeripheralState::Backoff);
    assert_eq!(p.stats().auth_failures, 1);
}

#[test]
fn proof_before_auth_init_is_protocol_violation() {
    let (mut p, mut tx) = setup();
    start(&mut p);
    connect(&mut p, CONN);
    let _ = drain_frames(&mut p, CONN);
    tx.send(&mut p, CONN, &MeshFrame::AuthProof { mac: [0xAA; 32] });
    p.pump();
    // One reject counted, handshake still in progress.
    assert_eq!(p.state(), PeripheralState::Authenticating);
    assert_eq!(p.stats().frame_rejects, 1);
}

#[test]
fn auth_timeout_disconnects_and_backs_off() {
    let (mut p, _) = setup();
    start(&mut p);
    connect(&mut p, CONN);
    assert_eq!(p.state(), PeripheralState::Authenticating);
    p.tick(9_999);
    assert_eq!(p.state(), PeripheralState::Authenticating);
    p.tick(10_000);
    assert_eq!(p.state(), PeripheralState::Backoff);
    assert_eq!(p.hal_mut().disconnects, vec![CONN]);
}

#[test]
fn rng_failure_on_connect_backs_off() {
    let (mut p, _) = setup();
    start(&mut p);
    p.hal_mut().fail_random = true;
    connect(&mut p, CONN);
    assert_eq!(p.state(), PeripheralState::Backoff);
}

#[test]
fn notify_failure_on_hello_drops_link() {
    let (mut p, _) = setup();
    start(&mut p);
    p.hal_mut().fail_notify = true;
    connect(&mut p, CONN);
    assert_eq!(p.state(), PeripheralState::Backoff);
}

#[test]
fn lsa_exchange_updates_neighbor_table() {
    let (mut p, mut tx) = setup();
    start(&mut p);
    connect(&mut p, CONN);
    central_handshake(&mut p, &mut tx, CONN, &SECRET, "central-1");
    let _ = drain_frames(&mut p, CONN); // AuthOk + first LSA
    assert_eq!(p.stats().lsas_tx, 1);

    tx.send(&mut p, CONN, &MeshFrame::Lsa(lsa_from("central-1", 9)));
    p.pump();
    assert_eq!(p.stats().lsas_rx, 1);
    let n = p.control().get_neighbor("central-1").expect("neighbor");
    assert_eq!(n.last_seq, 9, "LSA seq must advance the neighbor entry");
}

#[test]
fn lsa_before_auth_is_ignored() {
    let (mut p, mut tx) = setup();
    start(&mut p);
    connect(&mut p, CONN);
    let _ = drain_frames(&mut p, CONN);
    tx.send(&mut p, CONN, &MeshFrame::Lsa(lsa_from("central-1", 1)));
    p.pump();
    assert_eq!(p.stats().lsas_rx, 0);
    assert!(p.control().get_neighbor("central-1").is_none());
    assert_eq!(p.stats().frame_rejects, 1);
}

#[test]
fn malformed_frames_trigger_reject_limit_disconnect() {
    let (mut p, mut tx) = setup();
    start(&mut p);
    connect(&mut p, CONN);
    assert_eq!(p.state(), PeripheralState::Authenticating);
    for _ in 0..5 {
        tx.send_raw(&mut p, CONN, GattChar::RelayTx, vec![]); // not a fragment
    }
    p.pump();
    assert_eq!(p.stats().frame_rejects, 5);
    assert_eq!(p.state(), PeripheralState::Backoff);
    assert_eq!(p.hal_mut().disconnects, vec![CONN]);
}

#[test]
fn interleaved_fragment_message_ids_rejected() {
    let (mut p, mut tx) = setup();
    start(&mut p);
    connect(&mut p, CONN);
    let _ = drain_frames(&mut p, CONN);

    // First fragment of message 50...
    let frame = MeshFrame::AuthInit {
        node_name: "central-1".into(),
        battery_level: 61,
        challenge: [0xBBu8; 32],
    };
    let frags50 = fragment_ble_payload(50, &frame.encode());
    assert!(frags50.len() > 1);
    tx.send_raw(&mut p, CONN, GattChar::RelayTx, frags50[0].encode());
    // ...interleaved with a fragment of message 51.
    let frags51 = fragment_ble_payload(51, &frame.encode());
    tx.send_raw(&mut p, CONN, GattChar::RelayTx, frags51[0].encode());
    p.pump();
    assert_eq!(p.stats().frame_rejects, 1);
    assert_eq!(p.state(), PeripheralState::Authenticating);
    // The original message can still complete afterwards.
    for frag in &frags50[1..] {
        tx.send_raw(&mut p, CONN, GattChar::RelayTx, frag.encode());
    }
    p.pump();
    let frames = drain_frames(&mut p, CONN);
    assert!(frames
        .iter()
        .any(|f| matches!(f, MeshFrame::AuthProof { .. })));
}

#[test]
fn write_to_non_writable_characteristic_rejected() {
    let (mut p, mut tx) = setup();
    start(&mut p);
    connect(&mut p, CONN);
    tx.send_raw(
        &mut p,
        CONN,
        GattChar::RelayRx, // central must not write the notify char
        vec![0x01, 0x02, 0x03, 0x04],
    );
    p.pump();
    assert_eq!(p.stats().frame_rejects, 1);
}

#[test]
fn second_connection_while_active_is_refused() {
    let (mut p, mut tx) = setup();
    start(&mut p);
    connect(&mut p, CONN);
    central_handshake(&mut p, &mut tx, CONN, &SECRET, "central-1");
    assert_eq!(p.state(), PeripheralState::Active);

    let conn2 = ConnectionId(8);
    connect(&mut p, conn2);
    assert_eq!(
        p.state(),
        PeripheralState::Active,
        "session must be undisturbed"
    );
    assert_eq!(p.hal_mut().disconnects, vec![conn2]);
    assert_eq!(p.stats().refused_connections, 1);
}

#[test]
fn bye_returns_to_advertising_without_backoff() {
    let (mut p, mut tx) = setup();
    start(&mut p);
    connect(&mut p, CONN);
    central_handshake(&mut p, &mut tx, CONN, &SECRET, "central-1");
    let backoffs_before = p.stats().backoffs;
    tx.send(&mut p, CONN, &MeshFrame::Bye);
    p.pump();
    assert_eq!(p.state(), PeripheralState::Idle);
    p.tick(1);
    assert_eq!(p.state(), PeripheralState::Advertising);
    assert_eq!(p.stats().backoffs, backoffs_before);
}

#[test]
fn peer_timeout_disconnects_and_backs_off() {
    let (mut p, mut tx) = setup();
    start(&mut p);
    connect(&mut p, CONN);
    central_handshake(&mut p, &mut tx, CONN, &SECRET, "central-1");
    assert_eq!(p.state(), PeripheralState::Active);
    // peer_timeout = max(3 * adv_interval, auth_timeout) = 10_000.
    p.tick(9_999);
    assert_eq!(p.state(), PeripheralState::Active);
    p.tick(10_000);
    assert_eq!(p.state(), PeripheralState::Backoff);
}

#[test]
fn lsa_rebroadcast_cadence() {
    let (mut p, mut tx) = setup();
    start(&mut p);
    connect(&mut p, CONN);
    central_handshake(&mut p, &mut tx, CONN, &SECRET, "central-1");
    let _ = drain_frames(&mut p, CONN);
    assert_eq!(p.stats().lsas_tx, 1);

    // Keep the peer alive with an inbound LSA, then advance past the cadence.
    tx.send(&mut p, CONN, &MeshFrame::Lsa(lsa_from("central-1", 1)));
    p.pump();
    p.tick(1_999);
    assert_eq!(p.stats().lsas_tx, 1);
    p.tick(2_000);
    assert_eq!(p.stats().lsas_tx, 2);

    let frames = drain_frames(&mut p, CONN);
    let seqs: Vec<u32> = frames
        .iter()
        .filter_map(|f| match f {
            MeshFrame::Lsa(lsa) => Some(lsa.sequence),
            _ => None,
        })
        .collect();
    assert_eq!(seqs, vec![2], "rebroadcast LSA carries the next sequence");
}

#[test]
fn backoff_is_exponential_and_recovers() {
    let (mut p, mut tx) = setup();
    start(&mut p);
    // Failure 1: wrong-secret handshake -> backoff 2 * base = 2000 ms.
    connect(&mut p, CONN);
    let _ = drain_frames(&mut p, CONN);
    tx.send(
        &mut p,
        CONN,
        &MeshFrame::AuthInit {
            node_name: "rogue".into(),
            battery_level: 1,
            challenge: [0xBBu8; 32],
        },
    );
    p.pump();
    tx.send(&mut p, CONN, &MeshFrame::AuthProof { mac: [0x00; 32] });
    p.pump();
    assert_eq!(p.state(), PeripheralState::Backoff);
    p.tick(1_999);
    assert_eq!(p.state(), PeripheralState::Backoff);
    p.tick(2_000);
    assert_eq!(p.state(), PeripheralState::Idle);
    p.tick(2_000);
    assert_eq!(p.state(), PeripheralState::Advertising);

    // Failure 2: consecutive -> backoff 4 * base = 4000 ms.
    connect(&mut p, CONN);
    p.tick(12_000); // auth timeout fires
    assert_eq!(p.state(), PeripheralState::Backoff);
    p.tick(15_999);
    assert_eq!(p.state(), PeripheralState::Backoff);
    p.tick(16_000);
    assert_eq!(p.state(), PeripheralState::Idle);
}

#[test]
fn successful_auth_resets_backoff() {
    let (mut p, mut tx) = setup();
    start(&mut p);
    // One failure -> consecutive_failures = 1.
    connect(&mut p, CONN);
    let _ = drain_frames(&mut p, CONN); // clear the stale Hello before the timeout
    p.tick(10_000);
    assert_eq!(p.state(), PeripheralState::Backoff);
    p.tick(12_000);
    p.tick(12_000);
    assert_eq!(p.state(), PeripheralState::Advertising);
    // One success -> counter reset; next failure backs off 2 * base again.
    connect(&mut p, CONN);
    central_handshake(&mut p, &mut tx, CONN, &SECRET, "central-1");
    assert_eq!(p.state(), PeripheralState::Active);
    tx.send(&mut p, CONN, &MeshFrame::Bye);
    p.pump();
    p.tick(12_001);
    assert_eq!(p.state(), PeripheralState::Advertising);
    connect(&mut p, CONN);
    p.tick(22_001); // auth timeout
    assert_eq!(p.state(), PeripheralState::Backoff);
    p.tick(24_000);
    assert_eq!(p.state(), PeripheralState::Backoff);
    p.tick(24_001);
    assert_eq!(
        p.state(),
        PeripheralState::Idle,
        "backoff was reset by the success"
    );
}

#[test]
fn unknown_connection_events_ignored() {
    let (mut p, mut tx) = setup();
    start(&mut p);
    connect(&mut p, CONN);
    let ghost = ConnectionId(999);
    tx.send_raw(
        &mut p,
        ghost,
        GattChar::RelayTx,
        vec![0x01, 0x02, 0x03, 0x04],
    );
    p.hal_mut().events.push_back(HalEvent::Disconnected {
        conn: ghost,
        reason: crate::ble_hal::DisconnectReason::LinkLoss,
    });
    p.pump();
    assert_eq!(p.state(), PeripheralState::Authenticating);
    assert_eq!(p.stats().frame_rejects, 0);
    assert!(p.hal_mut().disconnects.is_empty());
}

#[test]
fn empty_node_name_rejected() {
    let (mut p, mut tx) = setup();
    start(&mut p);
    connect(&mut p, CONN);
    let _ = drain_frames(&mut p, CONN);
    tx.send(
        &mut p,
        CONN,
        &MeshFrame::AuthInit {
            node_name: String::new(),
            battery_level: 61,
            challenge: [0xBBu8; 32],
        },
    );
    p.pump();
    assert_eq!(p.stats().frame_rejects, 1);
    assert_eq!(p.state(), PeripheralState::Authenticating);
}

#[test]
fn hal_rejects_oversized_notify() {
    let mut hal = ScriptedHal::new();
    let err = hal
        .notify(CONN, GattChar::RelayRx, &[0u8; 21])
        .expect_err("21-byte notify must be rejected");
    assert_eq!(err, BleHalError::PayloadTooLarge { len: 21, max: 20 });
    assert!(hal.notify(CONN, GattChar::RelayRx, &[0u8; 20]).is_ok());
}
