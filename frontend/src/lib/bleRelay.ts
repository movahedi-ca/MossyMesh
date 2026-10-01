/**
 * WebBluetooth BLE mesh relay fallback.
 *
 * When the native Rust mesh daemon is unreachable, the React frontend can
 * act as a relay itself: it connects over Web Bluetooth to a nearby
 * MossyMesh peripheral and forwards framed mesh packets through a GATT
 * relay service. All browser support checks are explicit so the UI can
 * degrade gracefully (HTTPS/localhost + Chrome/Edge required).
 */

/** GATT service exposing the MossyMesh packet relay. */
export const MESH_RELAY_SERVICE_UUID = "7d9a4b2e-1f3c-4e5a-9b6d-2c8e0f1a3b5d";
/** Peripheral -> browser: inbound mesh frames (notify). */
export const MESH_RELAY_RX_UUID = "7d9a4b2e-1f3c-4e5a-9b6d-2c8e0f1a3b5e";
/** Browser -> peripheral: outbound mesh frames (write). */
export const MESH_RELAY_TX_UUID = "7d9a4b2e-1f3c-4e5a-9b6d-2c8e0f1a3b5f";

/** BLE ATT MTU leaves 20 bytes per write on legacy links. */
export const BLE_FRAME_MAX = 20;

export type BleRelayState =
  | "unsupported"
  | "idle"
  | "scanning"
  | "connected"
  | "error";

export interface BleRelay {
  readonly deviceName: string;
  /** Queue a mesh frame for the peripheral (fragmented to 20-byte writes). */
  send(frame: Uint8Array): Promise<void>;
  disconnect(): void;
}

export function isBleSupported(): boolean {
  return (
    typeof navigator !== "undefined" &&
    typeof navigator.bluetooth !== "undefined"
  );
}

function toChunks(frame: Uint8Array): Uint8Array<ArrayBuffer>[] {
  const chunks: Uint8Array<ArrayBuffer>[] = [];
  for (let i = 0; i < frame.length; i += BLE_FRAME_MAX) {
    // slice() copies into a fresh ArrayBuffer; subarray() would keep the
    // ArrayBufferLike view type, which writeValue()'s BufferSource rejects.
    chunks.push(frame.slice(i, i + BLE_FRAME_MAX));
  }
  return chunks.length > 0 ? chunks : [new Uint8Array(0)];
}

export async function connectBleRelay(
  onFrame: (frame: Uint8Array) => void,
  onDisconnect: () => void,
): Promise<BleRelay> {
  if (!isBleSupported()) {
    throw new Error("Web Bluetooth is not available in this browser");
  }
  const bluetooth = navigator.bluetooth as Bluetooth;

  const device = await bluetooth.requestDevice({
    filters: [{ services: [MESH_RELAY_SERVICE_UUID], namePrefix: "MossyMesh" }],
    optionalServices: [MESH_RELAY_SERVICE_UUID],
  });
  if (!device.gatt) throw new Error("Selected device exposes no GATT server");

  device.addEventListener("gattserverdisconnected", onDisconnect);
  const server = await device.gatt.connect();
  const service = await server.getPrimaryService(MESH_RELAY_SERVICE_UUID);
  const rx = await service.getCharacteristic(MESH_RELAY_RX_UUID);
  const tx = await service.getCharacteristic(MESH_RELAY_TX_UUID);

  const handleNotify = (event: Event) => {
    const target = event.target as BluetoothRemoteGATTCharacteristic | null;
    const value = target?.value;
    if (value) onFrame(new Uint8Array(value.buffer.slice(0)));
  };
  rx.addEventListener("characteristicvaluechanged", handleNotify);
  await rx.startNotifications();

  let closed = false;
  return {
    deviceName: device.name ?? device.id,
    async send(frame: Uint8Array): Promise<void> {
      if (closed) throw new Error("BLE relay is closed");
      for (const chunk of toChunks(frame)) {
        await tx.writeValue(chunk);
      }
    },
    disconnect(): void {
      closed = true;
      rx.removeEventListener("characteristicvaluechanged", handleNotify);
      void rx.stopNotifications().catch(() => undefined);
      server.disconnect();
    },
  };
}
