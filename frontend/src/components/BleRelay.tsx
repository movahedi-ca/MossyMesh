import { useCallback, useEffect, useRef, useState } from "react";
import {
  connectBleRelay,
  isBleRelayAvailable,
  type BleRelay,
  type BleRelayState,
} from "../lib/bleRelay";

const LABEL: Record<BleRelayState, string> = {
  unsupported: "BLE unavailable",
  idle: "Enable BLE relay",
  scanning: "Scanning…",
  connected: "BLE relay on",
  error: "BLE failed — retry",
};

/**
 * Lets the browser act as a mesh relay over Web Bluetooth when the native
 * daemon is unreachable. EXPERIMENTAL: no peer implementation ships yet,
 * and the toggle renders only in a secure context (Web Bluetooth refuses
 * plain HTTP, so it stays hidden on the real portal deployment).
 */
export const BleRelayToggle = () => {
  const [state, setState] = useState<BleRelayState>(
    isBleRelayAvailable() ? "idle" : "unsupported",
  );
  const [peer, setPeer] = useState<string | null>(null);
  const [frames, setFrames] = useState(0);
  const relayRef = useRef<BleRelay | null>(null);

  useEffect(() => {
    return () => {
      relayRef.current?.disconnect();
      relayRef.current = null;
    };
  }, []);

  const toggle = useCallback(async () => {
    if (state === "connected") {
      relayRef.current?.disconnect();
      relayRef.current = null;
      setPeer(null);
      setState("idle");
      return;
    }
    if (state === "scanning") return;
    setState("scanning");
    try {
      const relay = await connectBleRelay(
        () => setFrames((n) => n + 1),
        () => {
          relayRef.current = null;
          setPeer(null);
          setState("idle");
        },
      );
      relayRef.current = relay;
      setPeer(relay.deviceName);
      setState("connected");
    } catch {
      // User cancelled the picker, auth failed, or the link dropped:
      // stay idle, not stuck.
      setState(isBleRelayAvailable() ? "idle" : "unsupported");
    }
  }, [state]);

  if (state === "unsupported") return null;

  return (
    <button
      type="button"
      className={`ble-relay ble-relay--${state}`}
      onClick={toggle}
      disabled={state === "scanning"}
      title={
        state === "connected"
          ? `Relaying via ${peer ?? "BLE peer"} · ${frames} frames`
          : "Use this browser as a BLE mesh relay"
      }
      aria-pressed={state === "connected"}
      style={{
        marginTop: 8,
        padding: "6px 12px",
        borderRadius: 999,
        border: "1px solid #45f3ff",
        background: state === "connected" ? "#45f3ff" : "transparent",
        color: state === "connected" ? "#0b0d12" : "#45f3ff",
        fontSize: 12,
        cursor: state === "scanning" ? "wait" : "pointer",
      }}
    >
      <span aria-hidden="true">📡 </span>
      {LABEL[state]}
      {state === "connected" && frames > 0 ? ` · ${frames}` : ""}
    </button>
  );
};
