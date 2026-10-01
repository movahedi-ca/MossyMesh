/**
 * WebBluetooth BLE mesh relay fallback. EXPERIMENTAL.
 *
 * When the native Rust mesh daemon is unreachable, the React frontend can
 * act as a relay itself: it connects over Web Bluetooth to a nearby
 * MossyMesh peripheral and forwards framed mesh packets through a GATT
 * relay service.
 *
 * Hard requirements and trust model (read before enabling):
 * - Secure context only. Web Bluetooth refuses non-secure origins, and the
 *   captive portal serves plain HTTP, so the toggle is hidden unless
 *   `window.isSecureContext` is true (localhost counts as secure).
 * - No peer implementation ships in this repo (the UUIDs below appear
 *   nowhere else). The toggle is therefore experimental: any peripheral
 *   that connects must implement the minimal peripheral contract below,
 *   or the connection fails closed before a single frame is relayed.
 * - Peripherals are authenticated, never trusted by name. After GATT
 *   connect the browser runs a challenge-response handshake against the
 *   auth service: the peripheral proves possession of the private key
 *   matching its advertised P-256 identity, and the browser pins that
 *   identity per device. A "MossyMesh-*" name alone grants nothing.
 *
 * Minimal peripheral contract (for a future firmware / phone app):
 * - Advertise MESH_RELAY_SERVICE_UUID with a "MossyMesh-*" name.
 * - MESH_AUTH_SERVICE_UUID:
 *   - MESH_AUTH_IDENTITY_UUID (read): 65-byte uncompressed P-256 public key.
 *   - MESH_AUTH_CHALLENGE_UUID (write): 32-byte challenge from the browser.
 *   - MESH_AUTH_RESPONSE_UUID (read): 64-byte raw ECDSA P-256/SHA-256
 *     signature over the last challenge.
 * - MESH_RELAY_SERVICE_UUID:
 *   - MESH_RELAY_RX_UUID (notify): peripheral -> browser frames.
 *   - MESH_RELAY_TX_UUID (write): browser -> peripheral frames.
 * - Framing: every frame is a 2-byte big-endian length prefix followed by
 *   the payload, split into 20-byte ATT writes. The receiver reassembles
 *   by length prefix; chunks are never interpreted standalone.
 */

/** GATT service exposing the MossyMesh packet relay. */
export const MESH_RELAY_SERVICE_UUID = "7d9a4b2e-1f3c-4e5a-9b6d-2c8e0f1a3b5d";
/** Peripheral -> browser: inbound mesh frames (notify). */
export const MESH_RELAY_RX_UUID = "7d9a4b2e-1f3c-4e5a-9b6d-2c8e0f1a3b5e";
/** Browser -> peripheral: outbound mesh frames (write). */
export const MESH_RELAY_TX_UUID = "7d9a4b2e-1f3c-4e5a-9b6d-2c8e0f1a3b5f";

/** GATT service for the peripheral authentication handshake. */
export const MESH_AUTH_SERVICE_UUID = "7d9a4b2e-1f3c-4e5a-9b6d-2c8e0f1a3b60";
/** Read: 65-byte uncompressed P-256 peripheral identity key. */
export const MESH_AUTH_IDENTITY_UUID = "7d9a4b2e-1f3c-4e5a-9b6d-2c8e0f1a3b61";
/** Write: 32-byte challenge issued by the browser. */
export const MESH_AUTH_CHALLENGE_UUID = "7d9a4b2e-1f3c-4e5a-9b6d-2c8e0f1a3b62";
/** Read: 64-byte raw ECDSA signature over the challenge. */
export const MESH_AUTH_RESPONSE_UUID = "7d9a4b2e-1f3c-4e5a-9b6d-2c8e0f1a3b63";

/** BLE ATT MTU leaves 20 bytes per write on legacy links. */
export const BLE_FRAME_MAX = 20;
/** Length prefix (bytes, big-endian) prepended to every relayed frame. */
export const BLE_LENGTH_PREFIX = 2;
/** Largest single frame accepted (prefix + payload). */
export const BLE_FRAME_MAX_TOTAL = 64 * 1024;

export type BleRelayState =
  | "unsupported"
  | "idle"
  | "scanning"
  | "connected"
  | "error";

export interface BleRelay {
  readonly deviceName: string;
  /** Queue a mesh frame for the peripheral (framed, 20-byte writes). */
  send(frame: Uint8Array): Promise<void>;
  disconnect(): void;
}

export function isBleSupported(): boolean {
  return (
    typeof navigator !== "undefined" &&
    typeof navigator.bluetooth !== "undefined"
  );
}

/**
 * True only when the relay can actually work: Web Bluetooth present AND a
 * secure context. The portal serves plain HTTP, so this is false on the
 * real deployment and the toggle stays hidden there.
 */
export function isBleRelayAvailable(): boolean {
  return (
    isBleSupported() &&
    typeof window !== "undefined" &&
    window.isSecureContext === true
  );
}

/**
 * Frame one mesh packet for the wire: 2-byte big-endian length prefix +
 * payload, split into 20-byte ATT writes. The prefix is what lets the
 * receiver reassemble; raw 20-byte chunks are never meaningful alone.
 */
export function frameToChunks(frame: Uint8Array): Uint8Array<ArrayBuffer>[] {
  if (frame.length > BLE_FRAME_MAX_TOTAL - BLE_LENGTH_PREFIX) {
    throw new Error("BLE frame exceeds maximum relay size");
  }
  const framed = new Uint8Array(BLE_LENGTH_PREFIX + frame.length);
  framed[0] = (frame.length >>> 8) & 0xff;
  framed[1] = frame.length & 0xff;
  framed.set(frame, BLE_LENGTH_PREFIX);
  const chunks: Uint8Array<ArrayBuffer>[] = [];
  for (let i = 0; i < framed.length; i += BLE_FRAME_MAX) {
    // slice() copies into a fresh ArrayBuffer; subarray() would keep the
    // ArrayBufferLike view type, which writeValue()'s BufferSource rejects.
    chunks.push(framed.slice(i, i + BLE_FRAME_MAX));
  }
  return chunks;
}

/**
 * Reassembles length-prefixed frames from 20-byte notify payloads.
 * Incomplete trailing bytes are buffered until the rest arrives.
 */
export class BleFrameReassembler {
  private buffer = new Uint8Array(0);

  push(chunk: Uint8Array): Uint8Array[] {
    const merged = new Uint8Array(this.buffer.length + chunk.length);
    merged.set(this.buffer, 0);
    merged.set(chunk, this.buffer.length);
    this.buffer = merged;
    const frames: Uint8Array[] = [];
    while (this.buffer.length >= BLE_LENGTH_PREFIX) {
      const len = (this.buffer[0] << 8) | this.buffer[1];
      if (len > BLE_FRAME_MAX_TOTAL - BLE_LENGTH_PREFIX) {
        throw new Error("BLE peer sent an impossible frame length");
      }
      if (this.buffer.length < BLE_LENGTH_PREFIX + len) break;
      frames.push(this.buffer.slice(BLE_LENGTH_PREFIX, BLE_LENGTH_PREFIX + len));
      this.buffer = this.buffer.slice(BLE_LENGTH_PREFIX + len);
    }
    return frames;
  }

  reset(): void {
    this.buffer = new Uint8Array(0);
  }
}

const PEER_PIN_KEY = "mossymesh-ble-peer-pin";

function loadPeerPins(): Record<string, string> {
  try {
    const raw = window.localStorage.getItem(PEER_PIN_KEY);
    const parsed: unknown = raw ? JSON.parse(raw) : {};
    if (parsed && typeof parsed === "object") return parsed as Record<string, string>;
  } catch {
    // Corrupt pin store: treat as empty rather than bricking the relay.
  }
  return {};
}

function toBase64(bytes: Uint8Array): string {
  let s = "";
  for (const b of bytes) s += String.fromCharCode(b);
  return btoa(s);
}

/**
 * Challenge-response authentication against the peripheral's auth service.
 * Fails closed: missing characteristics, bad key format, bad signature, or
 * a changed pinned identity all throw before any frame is relayed.
 */
async function authenticatePeripheral(
  deviceId: string,
  server: BluetoothRemoteGATTServer,
): Promise<void> {
  let auth: BluetoothRemoteGATTService;
  try {
    auth = await server.getPrimaryService(MESH_AUTH_SERVICE_UUID);
  } catch {
    throw new Error(
      "Peripheral does not implement the MossyMesh BLE auth contract " +
        "(no peer implementation yet) - refusing to relay",
    );
  }
  let identity: DataView;
  let challengeChar: BluetoothRemoteGATTCharacteristic;
  let responseChar: BluetoothRemoteGATTCharacteristic;
  try {
    identity = await (await auth.getCharacteristic(MESH_AUTH_IDENTITY_UUID)).readValue();
    challengeChar = await auth.getCharacteristic(MESH_AUTH_CHALLENGE_UUID);
    responseChar = await auth.getCharacteristic(MESH_AUTH_RESPONSE_UUID);
  } catch {
    throw new Error("Peripheral auth service is incomplete - refusing to relay");
  }

  const pubRaw = new Uint8Array(identity.buffer.slice(0));
  if (pubRaw.length !== 65 || pubRaw[0] !== 0x04) {
    throw new Error("Peripheral identity key is not an uncompressed P-256 point");
  }
  const verifyKey = await crypto.subtle.importKey(
    "raw",
    pubRaw.buffer as ArrayBuffer,
    { name: "ECDSA", namedCurve: "P-256" },
    false,
    ["verify"],
  );

  const challenge = crypto.getRandomValues(new Uint8Array(32));
  await challengeChar.writeValue(challenge);
  const response = new Uint8Array((await responseChar.readValue()).buffer.slice(0));
  if (response.length !== 64) {
    throw new Error("Peripheral auth response has the wrong length");
  }
  const ok = await crypto.subtle.verify(
    { name: "ECDSA", hash: { name: "SHA-256" } },
    verifyKey,
    response.buffer as ArrayBuffer,
    challenge.buffer as ArrayBuffer,
  );
  if (!ok) throw new Error("Peripheral failed the auth challenge - refusing to relay");

  // Pin the identity per device so a swapped peripheral is caught next time.
  const pins = loadPeerPins();
  const fingerprint = toBase64(pubRaw);
  const pinned = pins[deviceId];
  if (pinned && pinned !== fingerprint) {
    throw new Error("Peripheral identity changed since last session - refusing to relay");
  }
  if (!pinned) {
    pins[deviceId] = fingerprint;
    try {
      window.localStorage.setItem(PEER_PIN_KEY, JSON.stringify(pins));
    } catch {
      // Pinning is best-effort; the handshake itself already passed.
    }
  }
}

export async function connectBleRelay(
  onFrame: (frame: Uint8Array) => void,
  onDisconnect: () => void,
): Promise<BleRelay> {
  if (!isBleRelayAvailable()) {
    throw new Error(
      "BLE relay needs a secure context (HTTPS or localhost); " +
        "the plain-HTTP portal cannot use Web Bluetooth",
    );
  }
  const bluetooth = navigator.bluetooth as Bluetooth;

  const device = await bluetooth.requestDevice({
    filters: [{ services: [MESH_RELAY_SERVICE_UUID], namePrefix: "MossyMesh" }],
    optionalServices: [MESH_RELAY_SERVICE_UUID, MESH_AUTH_SERVICE_UUID],
  });
  if (!device.gatt) throw new Error("Selected device exposes no GATT server");

  device.addEventListener("gattserverdisconnected", onDisconnect);
  const server = await device.gatt.connect();

  // Authenticate before touching the relay service: an unauthenticated
  // peripheral never sees a single frame.
  await authenticatePeripheral(device.id, server);

  const service = await server.getPrimaryService(MESH_RELAY_SERVICE_UUID);
  const rx = await service.getCharacteristic(MESH_RELAY_RX_UUID);
  const tx = await service.getCharacteristic(MESH_RELAY_TX_UUID);

  const reassembler = new BleFrameReassembler();
  const handleNotify = (event: Event) => {
    const target = event.target as BluetoothRemoteGATTCharacteristic | null;
    const value = target?.value;
    if (value) {
      for (const frame of reassembler.push(new Uint8Array(value.buffer.slice(0)))) {
        onFrame(frame);
      }
    }
  };
  rx.addEventListener("characteristicvaluechanged", handleNotify);
  await rx.startNotifications();

  let closed = false;
  return {
    deviceName: device.name ?? device.id,
    async send(frame: Uint8Array): Promise<void> {
      if (closed) throw new Error("BLE relay is closed");
      for (const chunk of frameToChunks(frame)) {
        await tx.writeValue(chunk);
      }
    },
    disconnect(): void {
      closed = true;
      reassembler.reset();
      rx.removeEventListener("characteristicvaluechanged", handleNotify);
      void rx.stopNotifications().catch(() => undefined);
      server.disconnect();
    },
  };
}
