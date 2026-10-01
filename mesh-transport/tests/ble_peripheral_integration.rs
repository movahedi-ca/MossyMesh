//! End-to-end BLE peripheral protocol tests over a faithful in-memory link.
//!
//! [`SimBleHal`] is a *test-only* backend: two endpoints wired through shared
//! memory with the real 20-byte ATT ceiling enforced. It exists so the full
//! peripheral protocol (advertise -> connect -> mutual auth -> LSA exchange ->
//! disconnect/retry) runs deterministically in CI. It is NOT a firmware
//! implementation; a real target implements [`BlePeripheralHal`] on its radio.
//!
//! The [`CentralPeer`] driver below implements the central side of the
//! protocol independently, using only the shared wire codec.

use mesh_transport::ble_auth::{
    auth_proof, verify_proof, MeshFrame, ROLE_CENTRAL, ROLE_PERIPHERAL,
};
use mesh_transport::ble_hal::{
    AdvertiseParams, BleHalError, BlePeripheralHal, ConnectionId, DisconnectReason, GattChar,
    HalEvent, ATT_MTU_LEGACY, DEVICE_NAME_PREFIX, MESH_RELAY_RX_UUID, MESH_RELAY_SERVICE_UUID,
    MESH_RELAY_TX_UUID,
};
use mesh_transport::ble_mesh::{
    fragment_ble_payload, BleMeshNode, BleRelayFragment, BleRelayReassembler,
};
use mesh_transport::ble_peripheral::{BlePeripheral, BlePeripheralConfig, PeripheralState};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

const SECRET: [u8; 32] = [0x5E; 32];
const ROGUE_SECRET: [u8; 32] = [0xE5; 32];
const CONN: ConnectionId = ConnectionId(1);

// ---------------------------------------------------------------------------
// Simulated link: shared airwaves between one peripheral HAL and one central
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Airwaves {
    advertising: bool,
    adv_params: Option<AdvertiseParams>,
    connected: bool,
    link_up: bool,
    p2c: VecDeque<(GattChar, Vec<u8>)>,
    c2p: VecDeque<(GattChar, Vec<u8>)>,
    control_events: VecDeque<HalEvent>,
    central_saw_disconnect: bool,
    rng_state: u64,
    max_on_air: usize,
}

/// In-memory [`BlePeripheralHal`] for tests. Enforces the 20-byte ATT
/// ceiling exactly like a legacy radio would.
struct SimBleHal {
    air: Rc<RefCell<Airwaves>>,
    conn: ConnectionId,
}

impl SimBleHal {
    fn new(air: Rc<RefCell<Airwaves>>) -> Self {
        air.borrow_mut().rng_state = 0xC0FF_EE42_DEAD_BEEF;
        Self { air, conn: CONN }
    }

    fn inject_link_loss(&self) {
        let mut air = self.air.borrow_mut();
        air.link_up = false;
        air.connected = false;
        air.control_events.push_back(HalEvent::Disconnected {
            conn: self.conn,
            reason: DisconnectReason::LinkLoss,
        });
    }

    /// Deliver a second connection report while a session is active, as some
    /// stacks do for cached-address directed connects.
    fn inject_remote_connected(&self, conn: ConnectionId) {
        self.air
            .borrow_mut()
            .control_events
            .push_back(HalEvent::Connected {
                conn,
                rssi_dbm: Some(-70),
            });
    }

    fn restore_link(&self) {
        self.air.borrow_mut().link_up = true;
    }
}

impl BlePeripheralHal for SimBleHal {
    fn start_advertising(&mut self, params: &AdvertiseParams) -> Result<(), BleHalError> {
        let mut air = self.air.borrow_mut();
        if air.advertising {
            return Err(BleHalError::AlreadyAdvertising);
        }
        air.advertising = true;
        air.adv_params = Some(params.clone());
        Ok(())
    }

    fn stop_advertising(&mut self) -> Result<(), BleHalError> {
        let mut air = self.air.borrow_mut();
        if !air.advertising {
            return Err(BleHalError::NotAdvertising);
        }
        air.advertising = false;
        Ok(())
    }

    fn poll_event(&mut self) -> Option<HalEvent> {
        let mut air = self.air.borrow_mut();
        if let Some(e) = air.control_events.pop_front() {
            return Some(e);
        }
        air.c2p
            .pop_front()
            .map(|(char, payload)| HalEvent::GattWrite {
                conn: self.conn,
                char,
                payload,
            })
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
        let mut air = self.air.borrow_mut();
        air.max_on_air = air.max_on_air.max(payload.len());
        if !air.link_up || conn != self.conn {
            return Ok(()); // RF loss: the write vanishes, like real radio.
        }
        air.p2c.push_back((char, payload.to_vec()));
        Ok(())
    }

    fn disconnect(&mut self, conn: ConnectionId) -> Result<(), BleHalError> {
        let mut air = self.air.borrow_mut();
        if conn != self.conn || !air.connected {
            return Err(BleHalError::UnknownConnection(conn));
        }
        air.connected = false;
        air.central_saw_disconnect = true;
        Ok(())
    }

    fn random_bytes(&mut self, out: &mut [u8]) -> Result<(), BleHalError> {
        let mut air = self.air.borrow_mut();
        for b in out.iter_mut() {
            air.rng_state ^= air.rng_state >> 12;
            air.rng_state ^= air.rng_state << 25;
            air.rng_state ^= air.rng_state >> 27;
            *b = (air.rng_state.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 56) as u8;
        }
        Ok(())
    }
}

/// The central side of the simulated link.
struct SimCentral {
    air: Rc<RefCell<Airwaves>>,
    conn: ConnectionId,
}

impl SimCentral {
    fn new(air: Rc<RefCell<Airwaves>>) -> Self {
        Self { air, conn: CONN }
    }

    fn advertisement(&self) -> Option<AdvertiseParams> {
        let air = self.air.borrow();
        if air.advertising {
            air.adv_params.clone()
        } else {
            None
        }
    }

    fn connect(&self) -> bool {
        let mut air = self.air.borrow_mut();
        if !air.advertising || air.connected || !air.link_up {
            return false;
        }
        air.connected = true;
        air.control_events.push_back(HalEvent::Connected {
            conn: self.conn,
            rssi_dbm: Some(-60),
        });
        true
    }

    fn is_connected(&self) -> bool {
        self.air.borrow().connected
    }

    fn poll_notify(&self) -> Option<(GattChar, Vec<u8>)> {
        self.air.borrow_mut().p2c.pop_front()
    }

    fn write(&self, char: GattChar, payload: &[u8]) -> Result<(), String> {
        if payload.len() > ATT_MTU_LEGACY {
            return Err(format!(
                "central write exceeds ATT ceiling: {}",
                payload.len()
            ));
        }
        let mut air = self.air.borrow_mut();
        if !air.connected {
            return Err("not connected".into());
        }
        air.max_on_air = air.max_on_air.max(payload.len());
        air.c2p.push_back((char, payload.to_vec()));
        Ok(())
    }

    fn disconnect_clean(&self) {
        let mut air = self.air.borrow_mut();
        air.connected = false;
        air.control_events.push_back(HalEvent::Disconnected {
            conn: self.conn,
            reason: DisconnectReason::PeerInitiated,
        });
    }

    fn saw_disconnect(&self) -> bool {
        self.air.borrow().central_saw_disconnect
    }
}

// ---------------------------------------------------------------------------
// Central protocol driver (independent implementation of the central side)
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq)]
enum CentralState {
    Start,
    AwaitingHello,
    AwaitingProof,
    AwaitingOk,
    Active,
    Failed,
}

struct CentralPeer {
    central: SimCentral,
    node: BleMeshNode,
    secret: [u8; 32],
    ch_b: [u8; 32],
    ch_a: Option<[u8; 32]>,
    state: CentralState,
    reasm: BleRelayReassembler,
    msg_id: u16,
    last_lsa_tx: u64,
    adv_interval: u64,
    verify_peer: bool,
    /// Skip the handshake and immediately send an LSA (abuse test).
    skip_handshake: bool,
}

impl CentralPeer {
    fn new(air: Rc<RefCell<Airwaves>>, name: &str, battery: u8, secret: [u8; 32]) -> Self {
        Self {
            central: SimCentral::new(air),
            node: BleMeshNode::from_seed(name.as_bytes(), battery),
            secret,
            ch_b: [0xBBu8; 32],
            ch_a: None,
            state: CentralState::Start,
            reasm: BleRelayReassembler::new(),
            msg_id: 500,
            last_lsa_tx: 0,
            adv_interval: 2_000,
            verify_peer: true,
            skip_handshake: false,
        }
    }

    fn send(&mut self, frame: &MeshFrame) {
        let bytes = frame.encode();
        self.msg_id = self.msg_id.wrapping_add(1);
        for frag in fragment_ble_payload(self.msg_id, &bytes) {
            self.central
                .write(GattChar::RelayTx, &frag.encode())
                .expect("central write");
        }
    }

    fn drive(&mut self, now_ms: u64) {
        self.node.set_time(now_ms);
        if self.state == CentralState::Start {
            if self.central.advertisement().is_some() && self.central.connect() {
                self.state = CentralState::AwaitingHello;
            }
            return;
        }
        if !self.central.is_connected() && self.state != CentralState::AwaitingOk {
            // Link gone and no handshake in flight: stay silent like a real radio.
            // (AwaitingOk still drains: an in-flight AuthReject may already be queued.)
            return;
        }
        while let Some((ch, payload)) = self.central.poll_notify() {
            assert_eq!(ch, GattChar::RelayRx);
            let frag = BleRelayFragment::decode(&payload).expect("valid fragment");
            if let Some(assembled) = self.reasm.accept(&frag) {
                let frame = MeshFrame::decode(&assembled).expect("valid frame");
                self.on_frame(frame);
            }
        }
        if !self.central.is_connected() {
            return;
        }
        if self.state == CentralState::Active
            && now_ms.saturating_sub(self.last_lsa_tx) >= self.adv_interval
        {
            let lsa = self.node.create_lsa();
            self.send(&MeshFrame::Lsa(lsa));
            self.last_lsa_tx = now_ms;
        }
    }

    fn on_frame(&mut self, frame: MeshFrame) {
        match (&self.state, frame) {
            (CentralState::AwaitingHello, MeshFrame::Hello { challenge, .. }) => {
                if self.skip_handshake {
                    // Abuse case: send an LSA with no authentication at all.
                    let lsa = self.node.create_lsa();
                    self.send(&MeshFrame::Lsa(lsa));
                    self.state = CentralState::Failed; // never becomes Active
                    return;
                }
                self.ch_a = Some(challenge);
                self.send(&MeshFrame::AuthInit {
                    node_name: self.node.node_id.clone(),
                    battery_level: self.node.battery_level,
                    challenge: self.ch_b,
                });
                self.state = CentralState::AwaitingProof;
            }
            (CentralState::AwaitingProof, MeshFrame::AuthProof { mac }) => {
                let ch_a = self.ch_a.expect("challenge");
                if self.verify_peer
                    && !verify_proof(&self.secret, ROLE_PERIPHERAL, &ch_a, &self.ch_b, &mac)
                {
                    self.state = CentralState::Failed;
                    return;
                }
                let proof = auth_proof(&self.secret, ROLE_CENTRAL, &self.ch_b, &ch_a);
                self.send(&MeshFrame::AuthProof { mac: proof });
                self.state = CentralState::AwaitingOk;
            }
            (CentralState::AwaitingOk, MeshFrame::AuthOk) => {
                self.state = CentralState::Active;
                self.last_lsa_tx = 0; // send immediately on next drive
                let lsa = self.node.create_lsa();
                self.send(&MeshFrame::Lsa(lsa));
            }
            (CentralState::AwaitingOk, MeshFrame::AuthReject { .. })
            | (CentralState::AwaitingProof, MeshFrame::AuthReject { .. }) => {
                self.state = CentralState::Failed;
            }
            (CentralState::Active, MeshFrame::Lsa(lsa)) => {
                self.node.process_lsa(&lsa);
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

fn setup_pair() -> (BlePeripheral<SimBleHal>, CentralPeer, Rc<RefCell<Airwaves>>) {
    let air = Rc::new(RefCell::new(Airwaves {
        link_up: true,
        ..Default::default()
    }));
    let hal = SimBleHal::new(Rc::clone(&air));
    let control = BleMeshNode::from_seed(b"periph-1", 88);
    let p = BlePeripheral::new(hal, control, BlePeripheralConfig::new(SECRET)).unwrap();
    let c = CentralPeer::new(Rc::clone(&air), "central-1", 61, SECRET);
    (p, c, air)
}

/// Run the virtual event loop until `cond` holds or `until_ms` passes.
fn run_until(
    p: &mut BlePeripheral<SimBleHal>,
    c: &mut CentralPeer,
    until_ms: u64,
    cond: impl Fn(&BlePeripheral<SimBleHal>, &CentralPeer) -> bool,
) -> u64 {
    let mut t = 0u64;
    while t <= until_ms {
        p.tick(t);
        p.pump();
        c.drive(t);
        if cond(p, c) {
            return t;
        }
        t += 100;
    }
    t
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn full_protocol_two_endpoints_exchange_lsas() {
    let (mut p, mut c, air) = setup_pair();

    // The central must see the exact GATT schema the frontend scans for.
    let t = run_until(&mut p, &mut c, 2_000, |_, c| {
        c.central.advertisement().is_some()
    });
    assert!(t <= 2_000, "peripheral must advertise promptly");
    let adv = c.central.advertisement().unwrap();
    assert_eq!(adv.service_uuid, MESH_RELAY_SERVICE_UUID);
    assert!(adv.device_name.starts_with(DEVICE_NAME_PREFIX));
    assert_eq!(MESH_RELAY_RX_UUID, "7d9a4b2e-1f3c-4e5a-9b6d-2c8e0f1a3b5e");
    assert_eq!(MESH_RELAY_TX_UUID, "7d9a4b2e-1f3c-4e5a-9b6d-2c8e0f1a3b5f");

    let t = run_until(&mut p, &mut c, 10_000, |p, c| {
        p.state() == PeripheralState::Active && c.state == CentralState::Active
    });
    assert!(t <= 10_000, "handshake must complete");
    assert_eq!(p.stats().auth_success, 1);

    // Let several LSA rounds flow in both directions.
    run_until(&mut p, &mut c, t + 8_000, |_, _| false);

    let pn = p
        .control()
        .get_neighbor(&c.node.node_id)
        .expect("peripheral neighbor");
    assert!(pn.last_seq >= 2, "peripheral must receive central LSAs");
    let cn = c
        .node
        .get_neighbor(&p.control().node_id)
        .expect("central neighbor");
    assert!(cn.last_seq >= 2, "central must receive peripheral LSAs");

    let max = air.borrow().max_on_air;
    assert!(
        max <= ATT_MTU_LEGACY,
        "every on-air chunk must fit the legacy ATT ceiling, saw {max}"
    );
    assert!(max > 0);
}

#[test]
fn rogue_central_with_wrong_secret_is_rejected() {
    let (mut p, _, air) = setup_pair();
    let mut rogue = CentralPeer::new(Rc::clone(&air), "rogue", 10, ROGUE_SECRET);
    rogue.verify_peer = false; // send its (wrong) proof without checking ours

    let t = run_until(&mut p, &mut rogue, 10_000, |p, _| {
        p.state() == PeripheralState::Backoff
    });
    assert!(t <= 10_000, "peripheral must reject the rogue");
    assert_eq!(rogue.state, CentralState::Failed);
    assert_eq!(p.stats().auth_failures, 1);
    assert!(p.control().get_neighbor("rogue").is_none());
    assert!(rogue.central.saw_disconnect());
}

#[test]
fn large_lsa_fragments_across_legacy_mtu() {
    let (mut p, mut c, air) = setup_pair();
    // Give the central 30 neighbors so its LSA needs many fragments.
    for i in 0..30 {
        c.node.hear_peer(&format!("leaf-{i:02}"), 50, -70);
    }
    let encoded_len = c.node.create_lsa().encode().len();
    assert!(
        encoded_len > ATT_MTU_LEGACY * 4,
        "test LSA must need several fragments, got {encoded_len}B"
    );

    let t = run_until(&mut p, &mut c, 10_000, |p, c| {
        p.state() == PeripheralState::Active && c.state == CentralState::Active
    });
    assert!(t <= 10_000);
    run_until(&mut p, &mut c, t + 6_000, |_, _| false);

    assert!(
        p.control().get_neighbor(&c.node.node_id).is_some(),
        "fragmented LSA must reassemble and apply"
    );
    assert!(air.borrow().max_on_air <= ATT_MTU_LEGACY);
}

#[test]
fn link_loss_recovers_via_backoff_and_readvertise() {
    let (mut p, mut c, air) = setup_pair();
    let t = run_until(&mut p, &mut c, 10_000, |p, c| {
        p.state() == PeripheralState::Active && c.state == CentralState::Active
    });
    assert!(t <= 10_000);

    // Kill the radio mid-exchange.
    p.hal_mut().inject_link_loss();
    let t2 = run_until(&mut p, &mut c, t + 5_000, |p, _| {
        p.state() == PeripheralState::Backoff
    });
    assert!(t2 <= t + 5_000, "link loss must drive backoff");

    // Backoff (2s after first failure) expires -> re-advertise -> new central.
    p.hal_mut().restore_link();
    let t3 = run_until(&mut p, &mut c, t2 + 10_000, |p, _| {
        p.state() == PeripheralState::Advertising
    });
    assert!(
        t3 <= t2 + 10_000,
        "peripheral must re-advertise after backoff"
    );

    let mut c2 = CentralPeer::new(Rc::clone(&air), "central-2", 70, SECRET);
    let t4 = run_until(&mut p, &mut c2, t3 + 10_000, |p, c| {
        p.state() == PeripheralState::Active && c.state == CentralState::Active
    });
    assert!(
        t4 <= t3 + 10_000,
        "second central must complete the handshake"
    );
    assert_eq!(p.stats().auth_success, 2);
    assert!(p.control().get_neighbor(&c2.node.node_id).is_some());
}

#[test]
fn unauthenticated_lsa_is_ignored_then_handshake_succeeds() {
    let (mut p, _, air) = setup_pair();
    let mut abuser = CentralPeer::new(Rc::clone(&air), "abuser", 10, SECRET);
    abuser.skip_handshake = true;

    let t = run_until(&mut p, &mut abuser, 10_000, |_, c| {
        c.state == CentralState::Failed
    });
    assert!(t <= 10_000);
    // More pump rounds: the LSA must never be applied.
    run_until(&mut p, &mut abuser, t + 2_000, |_, _| false);
    assert_eq!(p.stats().lsas_rx, 0);
    assert!(p.control().get_neighbor("abuser").is_none());
    assert!(p.stats().frame_rejects >= 1);

    // A well-behaved central can still use the peripheral afterwards.
    abuser.central.disconnect_clean();
    let mut c = CentralPeer::new(Rc::clone(&air), "central-1", 61, SECRET);
    let t2 = run_until(&mut p, &mut c, t + 15_000, |p, c| {
        p.state() == PeripheralState::Active && c.state == CentralState::Active
    });
    assert!(t2 <= t + 15_000);
}

#[test]
fn second_central_while_active_is_refused() {
    let (mut p, mut c, _air) = setup_pair();
    let t = run_until(&mut p, &mut c, 10_000, |p, c| {
        p.state() == PeripheralState::Active && c.state == CentralState::Active
    });
    assert!(t <= 10_000);

    // A cached-address directed connect can still reach the radio while a
    // session is active; the peripheral must refuse it defensively.
    p.hal_mut().inject_remote_connected(ConnectionId(2));
    p.pump();
    assert_eq!(p.stats().refused_connections, 1);
    assert_eq!(p.state(), PeripheralState::Active);
    // Post-#205 the peer name is the central's self-certifying node id.
    assert_eq!(p.peer_name(), Some(c.node.node_id.as_str()));
    // The first session is undisturbed: its LSAs still flow.
    let central_id = c.node.node_id.clone();
    let t2 = run_until(&mut p, &mut c, t + 8_000, |p, _| {
        p.control()
            .get_neighbor(&central_id)
            .map(|n| n.last_seq)
            .unwrap_or(0)
            >= 3
    });
    assert!(t2 <= t + 8_000);
}

#[test]
fn peripheral_hal_contract_holds_under_sim() {
    // ATT ceiling enforcement is what makes the fragmentation logic load-bearing.
    let air = Rc::new(RefCell::new(Airwaves {
        link_up: true,
        ..Default::default()
    }));
    let mut hal = SimBleHal::new(Rc::clone(&air));
    let err = hal
        .notify(CONN, GattChar::RelayRx, &[0u8; 21])
        .expect_err("sim HAL must enforce the ATT ceiling");
    assert_eq!(err, BleHalError::PayloadTooLarge { len: 21, max: 20 });
    assert!(hal.notify(CONN, GattChar::RelayRx, &[0u8; 20]).is_ok());
    assert_eq!(air.borrow().max_on_air, 20);
}
