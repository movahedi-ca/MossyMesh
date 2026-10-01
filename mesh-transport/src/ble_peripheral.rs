//! BLE peripheral role: advertising -> mutual authentication -> LSA exchange.
//!
//! [`BlePeripheral`] is the portable, hardware-independent peripheral
//! implementation behind issue #155. It drives a [`crate::ble_hal::BlePeripheralHal`]
//! through a deterministic state machine:
//!
//! ```text
//!   Idle -> Advertising -> Authenticating -> Active
//!                ^               |               |
//!                |               v               v
//!                +----------- Backoff <---(auth/timeout/link failure)
//! ```
//!
//! The handshake is mutual HMAC challenge-response ([`crate::ble_auth`]) keyed
//! by a provisioned island secret. No LSA or relayed frame is accepted before
//! authentication completes. LSAs ride the same 20-byte ATT fragments as the
//! frontend relay path ([`crate::ble_mesh::fragment_ble_payload`]), so the
//! peripheral speaks exactly the GATT schema the WebBluetooth client scans for.
//!
//! Time is injected via [`BlePeripheral::tick`]; a firmware main loop calls
//! `tick(now_ms)` then [`BlePeripheral::pump`] on every iteration.

use crate::ble_auth::{
    auth_proof, is_provisioned, verify_proof, AuthRejectReason, MeshFrame, CHALLENGE_LEN,
    ROLE_CENTRAL, ROLE_PERIPHERAL,
};
use crate::ble_hal::{
    AdvertiseParams, BleHalError, BlePeripheralHal, ConnectionId, DisconnectReason, GattChar,
    HalEvent, ATT_MTU_LEGACY, DEVICE_NAME_PREFIX, MESH_RELAY_SERVICE_UUID,
};
use crate::ble_mesh::{fragment_ble_payload, BleMeshNode, BleRelayFragment, BleRelayReassembler};

/// Fallback RSSI (dBm) when the HAL reports a connection without one.
const DEFAULT_RSSI_DBM: i8 = -70;

/// Peripheral behaviour knobs.
#[derive(Debug, Clone)]
pub struct BlePeripheralConfig {
    /// Provisioned island secret. All-zero means unprovisioned: the
    /// peripheral refuses to start (fail closed).
    pub island_secret: [u8; 32],
    /// LSA rebroadcast cadence while a central is connected (ms).
    pub adv_interval_ms: u64,
    /// Handshake deadline per connection (ms).
    pub auth_timeout_ms: u64,
    /// First reconnect backoff (ms); doubles per consecutive failure.
    pub backoff_base_ms: u64,
    /// Backoff ceiling (ms).
    pub backoff_max_ms: u64,
    /// Malformed-frame budget per connection before forced disconnect.
    pub max_frame_rejects: u32,
}

impl BlePeripheralConfig {
    pub fn new(island_secret: [u8; 32]) -> Self {
        Self {
            island_secret,
            adv_interval_ms: 2_000,
            auth_timeout_ms: 10_000,
            backoff_base_ms: 1_000,
            backoff_max_ms: 60_000,
            max_frame_rejects: 5,
        }
    }
}

/// Observable peripheral state (for tests and status UI).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeripheralState {
    Idle,
    Advertising,
    Authenticating,
    Active,
    Backoff,
}

/// Counters describing what the peripheral has done.
#[derive(Debug, Clone, Default)]
pub struct PeripheralStats {
    pub connects: u32,
    pub auth_success: u32,
    pub auth_failures: u32,
    pub lsas_rx: u32,
    pub lsas_tx: u32,
    pub frame_rejects: u32,
    pub backoffs: u32,
    pub refused_connections: u32,
}

#[derive(Debug)]
struct AuthSession {
    conn: ConnectionId,
    peer_name: Option<String>,
    peer_battery: u8,
    peer_challenge: Option<[u8; CHALLENGE_LEN]>,
    own_challenge: [u8; CHALLENGE_LEN],
    deadline_ms: u64,
    rssi_dbm: Option<i8>,
}

#[derive(Debug)]
struct ActiveSession {
    conn: ConnectionId,
    peer_name: String,
    last_lsa_tx_ms: u64,
    last_rx_ms: u64,
}

#[derive(Debug)]
enum State {
    Idle,
    Advertising,
    Authenticating(AuthSession),
    Active(ActiveSession),
    Backoff { until_ms: u64 },
}

/// BLE peripheral role above the HAL.
///
/// Owns the mesh control-plane node: authenticated peer LSAs are fed into
/// [`BleMeshNode::process_lsa`], and the node's own LSAs are broadcast to the
/// connected central.
pub struct BlePeripheral<H: BlePeripheralHal> {
    hal: H,
    config: BlePeripheralConfig,
    control: BleMeshNode,
    state: State,
    now_ms: u64,
    msg_id: u16,
    reassembler: BleRelayReassembler,
    /// (message_id, count) of the inbound message being reassembled.
    rx_msg: Option<(u16, u8)>,
    /// Malformed-frame budget for the current connection.
    conn_rejects: u32,
    consecutive_failures: u32,
    stats: PeripheralStats,
}

impl<H: BlePeripheralHal> BlePeripheral<H> {
    /// Build a peripheral. Fails closed when no island secret is provisioned.
    pub fn new(
        hal: H,
        control: BleMeshNode,
        config: BlePeripheralConfig,
    ) -> Result<Self, BleHalError> {
        if !is_provisioned(&config.island_secret) {
            return Err(BleHalError::Unprovisioned);
        }
        Ok(Self {
            hal,
            config,
            control,
            state: State::Idle,
            now_ms: 0,
            msg_id: 0,
            reassembler: BleRelayReassembler::new(),
            rx_msg: None,
            conn_rejects: 0,
            consecutive_failures: 0,
            stats: PeripheralStats::default(),
        })
    }

    pub fn state(&self) -> PeripheralState {
        match self.state {
            State::Idle => PeripheralState::Idle,
            State::Advertising => PeripheralState::Advertising,
            State::Authenticating(_) => PeripheralState::Authenticating,
            State::Active(_) => PeripheralState::Active,
            State::Backoff { .. } => PeripheralState::Backoff,
        }
    }

    pub fn stats(&self) -> &PeripheralStats {
        &self.stats
    }

    /// The mesh control-plane node (neighbor table, LSA seq).
    pub fn control(&self) -> &BleMeshNode {
        &self.control
    }

    /// Name of the authenticated central, when a session is active.
    pub fn peer_name(&self) -> Option<&str> {
        match &self.state {
            State::Active(s) => Some(&s.peer_name),
            _ => None,
        }
    }

    pub fn hal_mut(&mut self) -> &mut H {
        &mut self.hal
    }

    fn device_name(&self) -> String {
        format!("{DEVICE_NAME_PREFIX}-{}", self.control.node_id)
    }

    /// Advance timers: start advertising, enforce handshake/peer timeouts,
    /// rebroadcast LSAs, expire backoff.
    pub fn tick(&mut self, now_ms: u64) {
        self.now_ms = now_ms;
        match &self.state {
            State::Idle => {
                let params = AdvertiseParams {
                    device_name: self.device_name(),
                    service_uuid: MESH_RELAY_SERVICE_UUID,
                    interval_ms: self.config.adv_interval_ms,
                };
                // A HAL failure here just retries on the next tick.
                if self.hal.start_advertising(&params).is_ok() {
                    self.state = State::Advertising;
                }
            }
            State::Advertising => {}
            State::Authenticating(sess) => {
                if now_ms >= sess.deadline_ms {
                    let conn = sess.conn;
                    self.fail_connection(conn, DisconnectReason::Timeout, None);
                }
            }
            State::Active(sess) => {
                let peer_timeout = self
                    .config
                    .adv_interval_ms
                    .saturating_mul(3)
                    .max(self.config.auth_timeout_ms);
                if now_ms.saturating_sub(sess.last_rx_ms) >= peer_timeout {
                    let conn = sess.conn;
                    self.fail_connection(conn, DisconnectReason::Timeout, None);
                } else if now_ms.saturating_sub(sess.last_lsa_tx_ms) >= self.config.adv_interval_ms
                {
                    let conn = sess.conn;
                    self.send_own_lsa(conn);
                    if let State::Active(s) = &mut self.state {
                        s.last_lsa_tx_ms = now_ms;
                    }
                }
            }
            State::Backoff { until_ms } => {
                if now_ms >= *until_ms {
                    self.state = State::Idle;
                }
            }
        }
    }

    /// Drain HAL events and handle them.
    pub fn pump(&mut self) {
        while let Some(event) = self.hal.poll_event() {
            self.handle_event(event);
        }
    }

    // -- event handling ----------------------------------------------------

    fn handle_event(&mut self, event: HalEvent) {
        match event {
            HalEvent::Connected { conn, rssi_dbm } => self.on_connected(conn, rssi_dbm),
            HalEvent::Disconnected { conn, reason } => self.on_disconnected(conn, reason),
            HalEvent::GattWrite {
                conn,
                char,
                payload,
            } => self.on_gatt_write(conn, char, payload),
        }
    }

    fn on_connected(&mut self, conn: ConnectionId, rssi_dbm: Option<i8>) {
        if !matches!(self.state, State::Advertising) {
            // Single-connection peripheral: refuse anything unexpected.
            self.stats.refused_connections = self.stats.refused_connections.saturating_add(1);
            let _ = self.hal.disconnect(conn);
            return;
        }
        let _ = self.hal.stop_advertising();
        let mut challenge = [0u8; CHALLENGE_LEN];
        if self.hal.random_bytes(&mut challenge).is_err() {
            self.stats.auth_failures = self.stats.auth_failures.saturating_add(1);
            let _ = self.hal.disconnect(conn);
            self.enter_backoff();
            return;
        }
        self.stats.connects = self.stats.connects.saturating_add(1);
        self.conn_rejects = 0;
        self.rx_msg = None;
        self.reassembler = BleRelayReassembler::new();
        let deadline_ms = self.now_ms.saturating_add(self.config.auth_timeout_ms);
        let hello = MeshFrame::Hello {
            node_name: self.control.node_id.clone(),
            battery_level: self.control.battery_level,
            challenge,
        };
        // Hello is best-effort: if the radio cannot take it, drop the link.
        let hello_ok = self.send_frame(conn, GattChar::RelayRx, &hello).is_ok();
        self.state = State::Authenticating(AuthSession {
            conn,
            peer_name: None,
            peer_battery: 0,
            peer_challenge: None,
            own_challenge: challenge,
            deadline_ms,
            rssi_dbm,
        });
        if !hello_ok {
            self.fail_connection(conn, DisconnectReason::LinkLoss, None);
        }
    }

    fn on_disconnected(&mut self, conn: ConnectionId, reason: DisconnectReason) {
        let expected = match &self.state {
            State::Authenticating(s) => Some(s.conn),
            State::Active(s) => Some(s.conn),
            _ => None,
        };
        if expected != Some(conn) {
            return;
        }
        match reason {
            // Clean goodbye: go straight back to advertising, no penalty.
            DisconnectReason::PeerInitiated => {
                self.state = State::Idle;
            }
            _ => self.enter_backoff(),
        }
    }

    fn on_gatt_write(&mut self, conn: ConnectionId, char: GattChar, payload: Vec<u8>) {
        let expected = match &self.state {
            State::Authenticating(s) => Some(s.conn),
            State::Active(s) => Some(s.conn),
            _ => None,
        };
        if expected != Some(conn) {
            return;
        }
        if !char.writable() {
            self.count_reject(conn);
            return;
        }
        let frag = match BleRelayFragment::decode(&payload) {
            Some(f) => f,
            None => {
                self.count_reject(conn);
                return;
            }
        };
        // Fragments of one message must not interleave with another's.
        match self.rx_msg {
            Some((id, count)) if id != frag.message_id || count != frag.count => {
                self.count_reject(conn);
                return;
            }
            Some(_) => {}
            None => self.rx_msg = Some((frag.message_id, frag.count)),
        }
        if let Some(assembled) = self.reassembler.accept(&frag) {
            self.rx_msg = None;
            self.on_frame(conn, &assembled);
        }
    }

    fn on_frame(&mut self, conn: ConnectionId, bytes: &[u8]) {
        let frame = match MeshFrame::decode(bytes) {
            Ok(f) => f,
            Err(_) => {
                self.count_reject(conn);
                return;
            }
        };
        match &self.state {
            State::Authenticating(_) => self.on_frame_authenticating(conn, frame),
            State::Active(_) => self.on_frame_active(conn, frame),
            _ => self.count_reject(conn),
        }
    }

    fn on_frame_authenticating(&mut self, conn: ConnectionId, frame: MeshFrame) {
        match frame {
            MeshFrame::AuthInit {
                node_name,
                battery_level,
                challenge,
            } => {
                if node_name.is_empty() {
                    self.count_reject(conn);
                    return;
                }
                let (own_challenge, secret) = match &self.state {
                    State::Authenticating(s) if s.conn == conn => {
                        (s.own_challenge, self.config.island_secret)
                    }
                    _ => {
                        self.count_reject(conn);
                        return;
                    }
                };
                // Proof binds both fresh challenges under our role label.
                let mac = auth_proof(&secret, ROLE_PERIPHERAL, &own_challenge, &challenge);
                let proof_ok = self
                    .send_frame(conn, GattChar::RelayRx, &MeshFrame::AuthProof { mac })
                    .is_ok();
                if let State::Authenticating(s) = &mut self.state {
                    s.peer_name = Some(node_name);
                    s.peer_battery = battery_level;
                    s.peer_challenge = Some(challenge);
                }
                if !proof_ok {
                    self.fail_connection(conn, DisconnectReason::LinkLoss, None);
                }
            }
            MeshFrame::AuthProof { mac } => {
                let (peer_challenge, own_challenge, peer_name, peer_battery, rssi) =
                    match &self.state {
                        State::Authenticating(s) => (
                            s.peer_challenge,
                            s.own_challenge,
                            s.peer_name.clone(),
                            s.peer_battery,
                            s.rssi_dbm,
                        ),
                        _ => return,
                    };
                let (Some(peer_challenge), Some(peer_name)) = (peer_challenge, peer_name) else {
                    // Proof before AuthInit: protocol violation.
                    self.count_reject(conn);
                    return;
                };
                let ok = verify_proof(
                    &self.config.island_secret,
                    ROLE_CENTRAL,
                    &peer_challenge,
                    &own_challenge,
                    &mac,
                );
                if !ok {
                    self.stats.auth_failures = self.stats.auth_failures.saturating_add(1);
                    self.fail_connection(
                        conn,
                        DisconnectReason::AuthFailure,
                        Some(AuthRejectReason::BadProof),
                    );
                    return;
                }
                self.stats.auth_success = self.stats.auth_success.saturating_add(1);
                self.consecutive_failures = 0;
                let rssi = rssi.unwrap_or(DEFAULT_RSSI_DBM);
                self.control.hear_peer(&peer_name, peer_battery, rssi);
                let lsa_now = self.now_ms;
                self.state = State::Active(ActiveSession {
                    conn,
                    peer_name,
                    last_lsa_tx_ms: lsa_now,
                    last_rx_ms: lsa_now,
                });
                let ok_frame = self.send_frame(conn, GattChar::RelayRx, &MeshFrame::AuthOk);
                let lsa_ok = self.send_own_lsa(conn);
                if ok_frame.is_err() || !lsa_ok {
                    self.fail_connection(conn, DisconnectReason::LinkLoss, None);
                }
            }
            _ => self.count_reject(conn),
        }
    }

    fn on_frame_active(&mut self, conn: ConnectionId, frame: MeshFrame) {
        match frame {
            MeshFrame::Lsa(lsa) => {
                self.stats.lsas_rx = self.stats.lsas_rx.saturating_add(1);
                let now = self.now_ms;
                self.control.set_time(now);
                self.control.process_lsa(&lsa);
                if let State::Active(s) = &mut self.state {
                    s.last_rx_ms = now;
                }
            }
            MeshFrame::Bye => {
                let _ = self.hal.disconnect(conn);
                // Treat the goodbye as clean: no backoff penalty.
                self.state = State::Idle;
            }
            _ => self.count_reject(conn),
        }
    }

    // -- helpers ------------------------------------------------------------

    /// Fragment a frame into <= 20 byte ATT writes and notify them in order.
    fn send_frame(
        &mut self,
        conn: ConnectionId,
        char: GattChar,
        frame: &MeshFrame,
    ) -> Result<(), BleHalError> {
        let bytes = frame.encode();
        self.msg_id = self.msg_id.wrapping_add(1);
        let frags = fragment_ble_payload(self.msg_id, &bytes);
        for frag in &frags {
            let wire = frag.encode();
            debug_assert!(wire.len() <= ATT_MTU_LEGACY);
            self.hal.notify(conn, char, &wire)?;
        }
        Ok(())
    }

    fn send_own_lsa(&mut self, conn: ConnectionId) -> bool {
        let lsa = self.control.create_lsa();
        let ok = self
            .send_frame(conn, GattChar::RelayRx, &MeshFrame::Lsa(lsa))
            .is_ok();
        if ok {
            self.stats.lsas_tx = self.stats.lsas_tx.saturating_add(1);
        }
        ok
    }

    fn count_reject(&mut self, conn: ConnectionId) {
        self.stats.frame_rejects = self.stats.frame_rejects.saturating_add(1);
        self.conn_rejects = self.conn_rejects.saturating_add(1);
        if self.conn_rejects >= self.config.max_frame_rejects {
            self.fail_connection(conn, DisconnectReason::ProtocolViolation, None);
        }
    }

    /// Best-effort reject notice, then drop the link and back off.
    fn fail_connection(
        &mut self,
        conn: ConnectionId,
        reason: DisconnectReason,
        reject: Option<AuthRejectReason>,
    ) {
        if let Some(r) = reject {
            let _ = self.send_frame(
                conn,
                GattChar::RelayRx,
                &MeshFrame::AuthReject { reason: r },
            );
        }
        let _ = self.hal.disconnect(conn);
        if !matches!(reason, DisconnectReason::PeerInitiated) {
            self.enter_backoff();
        } else {
            self.state = State::Idle;
        }
    }

    fn enter_backoff(&mut self) {
        self.stats.backoffs = self.stats.backoffs.saturating_add(1);
        self.consecutive_failures = self.consecutive_failures.saturating_add(1);
        let shift = self.consecutive_failures.min(6);
        let delay = self
            .config
            .backoff_base_ms
            .saturating_mul(1 << shift)
            .min(self.config.backoff_max_ms);
        let _ = self.hal.stop_advertising();
        self.rx_msg = None;
        self.conn_rejects = 0;
        self.reassembler = BleRelayReassembler::new();
        self.state = State::Backoff {
            until_ms: self.now_ms.saturating_add(delay),
        };
    }
}

#[cfg(test)]
mod tests;
