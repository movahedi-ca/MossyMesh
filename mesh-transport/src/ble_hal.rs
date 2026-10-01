//! BLE peripheral hardware abstraction layer (HAL).
//!
//! This trait is the seam between the portable peripheral-role protocol logic
//! ([`crate::ble_peripheral`]) and the radio. A real firmware target implements
//! it on top of its BLE stack (e.g. Embassy + nrf-softdevice, ESP-IDF Bluedroid,
//! or BlueZ over D-Bus); tests implement it in memory.
//!
//! The GATT schema mirrors the WebBluetooth client in
//! `frontend/src/lib/bleRelay.ts` byte-for-byte: same service UUID, same RX/TX
//! characteristic UUIDs, same 20-byte ATT ceiling. That closes the "orphaned
//! UUID" half of issue #155: a peripheral built on this HAL advertises exactly
//! the service the frontend scans for.

use std::fmt;

/// GATT service exposing the MossyMesh relay. Must match
/// `MESH_RELAY_SERVICE_UUID` in `frontend/src/lib/bleRelay.ts`.
pub const MESH_RELAY_SERVICE_UUID: &str = "7d9a4b2e-1f3c-4e5a-9b6d-2c8e0f1a3b5d";

/// Peripheral -> central: inbound mesh frames (notify).
/// Must match `MESH_RELAY_RX_UUID` in `frontend/src/lib/bleRelay.ts`.
pub const MESH_RELAY_RX_UUID: &str = "7d9a4b2e-1f3c-4e5a-9b6d-2c8e0f1a3b5e";

/// Central -> peripheral: outbound mesh frames (write).
/// Must match `MESH_RELAY_TX_UUID` in `frontend/src/lib/bleRelay.ts`.
pub const MESH_RELAY_TX_UUID: &str = "7d9a4b2e-1f3c-4e5a-9b6d-2c8e0f1a3b5f";

/// Peripheral -> central: compact beacon snapshot (read). New characteristic
/// for the daemon-to-daemon use case; the browser client ignores it.
pub const MESH_BEACON_UUID: &str = "7d9a4b2e-1f3c-4e5a-9b6d-2c8e0f1a3b60";

/// Legacy BLE ATT payload ceiling. Notifications and writes larger than this
/// are rejected by the HAL; the protocol layer fragments above it.
pub const ATT_MTU_LEGACY: usize = 20;

/// Device-name prefix the frontend filters on (`namePrefix: "MossyMesh"`).
pub const DEVICE_NAME_PREFIX: &str = "MossyMesh";

/// Opaque handle for one central connection, issued by the HAL.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ConnectionId(pub u32);

/// GATT characteristics of the mesh relay service.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GattChar {
    /// Peripheral -> central notifications (mesh frames toward the central).
    RelayRx,
    /// Central -> peripheral writes (mesh frames toward the mesh).
    RelayTx,
    /// Read-only beacon snapshot.
    Beacon,
}

impl GattChar {
    pub fn uuid(self) -> &'static str {
        match self {
            GattChar::RelayRx => MESH_RELAY_RX_UUID,
            GattChar::RelayTx => MESH_RELAY_TX_UUID,
            GattChar::Beacon => MESH_BEACON_UUID,
        }
    }

    /// Characteristics the central is allowed to write to.
    pub fn writable(self) -> bool {
        matches!(self, GattChar::RelayTx)
    }
}

/// Parameters for one advertising set.
#[derive(Debug, Clone)]
pub struct AdvertiseParams {
    /// Full device name, e.g. `MossyMesh-node-7` (must carry the prefix).
    pub device_name: String,
    /// 128-bit service UUID string advertised in the AD packet.
    pub service_uuid: &'static str,
    /// Advertisement interval in milliseconds.
    pub interval_ms: u64,
}

/// Why a connection went away.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisconnectReason {
    PeerInitiated,
    LocalRequest,
    LinkLoss,
    AuthFailure,
    ProtocolViolation,
    Timeout,
}

/// Asynchronous events the radio reports to the protocol layer.
#[derive(Debug, Clone)]
pub enum HalEvent {
    Connected {
        conn: ConnectionId,
        /// RSSI in dBm when the stack reports it; used for link quality.
        rssi_dbm: Option<i8>,
    },
    Disconnected {
        conn: ConnectionId,
        reason: DisconnectReason,
    },
    /// One ATT write from the central (a single <= 20 byte fragment).
    GattWrite {
        conn: ConnectionId,
        char: GattChar,
        payload: Vec<u8>,
    },
}

/// Errors a HAL implementation can report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BleHalError {
    AlreadyAdvertising,
    NotAdvertising,
    UnknownConnection(ConnectionId),
    /// Payload exceeded the ATT ceiling; the caller must fragment.
    PayloadTooLarge {
        len: usize,
        max: usize,
    },
    RngFailure,
    Unprovisioned,
    Hardware(String),
}

impl fmt::Display for BleHalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BleHalError::AlreadyAdvertising => write!(f, "already advertising"),
            BleHalError::NotAdvertising => write!(f, "not advertising"),
            BleHalError::UnknownConnection(c) => write!(f, "unknown connection {:?}", c.0),
            BleHalError::PayloadTooLarge { len, max } => {
                write!(f, "payload {len} bytes exceeds ATT ceiling of {max}")
            }
            BleHalError::RngFailure => write!(f, "hardware RNG failure"),
            BleHalError::Unprovisioned => {
                write!(f, "no island secret provisioned; refusing to authenticate")
            }
            BleHalError::Hardware(msg) => write!(f, "hardware error: {msg}"),
        }
    }
}

impl std::error::Error for BleHalError {}

/// Portable BLE peripheral role: advertise, accept one central, GATT notify.
///
/// The trait is synchronous and poll-based so the protocol layer stays
/// deterministic and unit-testable without an async runtime. A real firmware
/// HAL bridges its interrupt/callback BLE stack into `poll_event`.
///
/// Contract notes for implementors:
/// - `notify` MUST reject payloads larger than [`ATT_MTU_LEGACY`] with
///   [`BleHalError::PayloadTooLarge`]; fragmentation is the caller's job.
/// - `poll_event` returns `None` when the event queue is drained.
/// - `random_bytes` must come from a hardware RNG on real targets.
pub trait BlePeripheralHal {
    fn start_advertising(&mut self, params: &AdvertiseParams) -> Result<(), BleHalError>;
    fn stop_advertising(&mut self) -> Result<(), BleHalError>;
    fn poll_event(&mut self) -> Option<HalEvent>;
    fn notify(
        &mut self,
        conn: ConnectionId,
        char: GattChar,
        payload: &[u8],
    ) -> Result<(), BleHalError>;
    fn disconnect(&mut self, conn: ConnectionId) -> Result<(), BleHalError>;
    fn random_bytes(&mut self, out: &mut [u8]) -> Result<(), BleHalError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gatt_uuids_match_frontend_contract() {
        // These strings are copied from frontend/src/lib/bleRelay.ts. If the
        // frontend changes them, this test fails and the peripheral schema
        // must be updated to match (issue #155: orphaned UUIDs).
        assert_eq!(
            MESH_RELAY_SERVICE_UUID,
            "7d9a4b2e-1f3c-4e5a-9b6d-2c8e0f1a3b5d"
        );
        assert_eq!(MESH_RELAY_RX_UUID, "7d9a4b2e-1f3c-4e5a-9b6d-2c8e0f1a3b5e");
        assert_eq!(MESH_RELAY_TX_UUID, "7d9a4b2e-1f3c-4e5a-9b6d-2c8e0f1a3b5f");
        assert_eq!(GattChar::RelayRx.uuid(), MESH_RELAY_RX_UUID);
        assert_eq!(GattChar::RelayTx.uuid(), MESH_RELAY_TX_UUID);
        assert!(GattChar::RelayTx.writable());
        assert!(!GattChar::RelayRx.writable());
    }

    #[test]
    fn att_ceiling_matches_legacy_relay_mtu() {
        assert_eq!(ATT_MTU_LEGACY, crate::ble_mesh::BLE_LEGACY_MTU);
    }
}
