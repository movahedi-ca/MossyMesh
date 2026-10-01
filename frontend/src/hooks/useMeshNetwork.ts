import {
  createContext,
  createElement,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react";

export type MeshLinkMode = "internet" | "island" | "local";

export interface MeshNode {
  id: string;
  type: "hub" | "edge" | "genesis" | "peer";
  latency: number;
  /** Received signal strength in dBm (negative; closer to 0 is stronger). */
  rssi: number;
  status: "active" | "syncing" | "offline" | "island";
  islandId: string;
  hops: number;
}

/** Link-quality bars (0-4) derived from RSSI. */
export function rssiBars(rssi: number): number {
  if (rssi >= -60) return 4;
  if (rssi >= -70) return 3;
  if (rssi >= -80) return 2;
  if (rssi >= -90) return 1;
  return 0;
}

export interface MeshSnapshot {
  browserOnline: boolean;
  linkMode: MeshLinkMode;
  islandId: string;
  islandName: string;
  peerCount: number;
  dhtReady: boolean;
  loraPeers: number;
  nodes: MeshNode[];
  lastSyncLabel: string;
}

const SEED_NODES: MeshNode[] = [
  { id: "MM-GEN-01", type: "genesis", latency: 4, rssi: -52, status: "active", islandId: "island-alpha", hops: 0 },
  { id: "LORA-EDGE-3A", type: "edge", latency: 124, rssi: -78, status: "active", islandId: "island-alpha", hops: 1 },
  { id: "HUB-NVME-X9", type: "hub", latency: 12, rssi: -61, status: "syncing", islandId: "island-alpha", hops: 1 },
  { id: "PEER-PHN-7C", type: "peer", latency: 48, rssi: -69, status: "active", islandId: "island-beta", hops: 2 },
  { id: "LORA-EDGE-9F", type: "edge", latency: 210, rssi: -88, status: "island", islandId: "island-beta", hops: 2 },
];

function deriveLinkMode(browserOnline: boolean, peerCount: number): MeshLinkMode {
  if (browserOnline && peerCount > 0) return "internet";
  if (peerCount > 0) return "island";
  return "local";
}

function useMeshNetworkState(): MeshSnapshot {
  const [browserOnline, setBrowserOnline] = useState(
    typeof navigator !== "undefined" ? navigator.onLine : false,
  );
  const [nodes, setNodes] = useState<MeshNode[]>([]);
  const [dhtReady, setDhtReady] = useState(false);

  useEffect(() => {
    const on = () => setBrowserOnline(true);
    const off = () => setBrowserOnline(false);
    window.addEventListener("online", on);
    window.addEventListener("offline", off);
    return () => {
      window.removeEventListener("online", on);
      window.removeEventListener("offline", off);
    };
  }, []);

  useEffect(() => {
    const timers: number[] = [];
    let delay = 350;
    timers.push(window.setTimeout(() => setDhtReady(true), 600));
    SEED_NODES.forEach((node) => {
      timers.push(
        window.setTimeout(() => {
          setNodes((prev) => {
            if (prev.some((n) => n.id === node.id)) return prev;
            return [...prev, node];
          });
        }, delay),
      );
      delay += 450 + Math.floor(Math.random() * 700);
    });
    const jitter = window.setInterval(() => {
      setNodes((prev) =>
        prev.map((n) => {
          if (n.status === "offline") return n;
          const delta = Math.floor(Math.random() * 11) - 5;
          const latency = Math.max(2, n.latency + delta);
          const rssiDelta = Math.floor(Math.random() * 7) - 3;
          const rssi = Math.min(-30, Math.max(-95, n.rssi + rssiDelta));
          let status = n.status;
          if (n.status === "syncing" && Math.random() > 0.7) status = "active";
          return { ...n, latency, rssi, status };
        }),
      );
    }, 4000);
    return () => {
      timers.forEach((t) => clearTimeout(t));
      clearInterval(jitter);
    };
  }, []);

  return useMemo(() => {
    const peerCount = nodes.filter((n) => n.status !== "offline").length;
    const loraPeers = nodes.filter((n) => n.type === "edge" && n.status !== "offline").length;
    const linkMode = deriveLinkMode(browserOnline, peerCount);
    return {
      browserOnline,
      linkMode,
      islandId: "island-alpha",
      islandName: browserOnline ? "Bridge · Alpha" : "Local Island · Alpha",
      peerCount,
      dhtReady,
      loraPeers,
      nodes,
      lastSyncLabel: dhtReady ? "DHT warm · local store" : "Bootstrapping Kademlia…",
    };
  }, [browserOnline, nodes, dhtReady]);
}

const MeshContext = createContext<MeshSnapshot | null>(null);

export function MeshProvider({ children }: { children: ReactNode }) {
  const value = useMeshNetworkState();
  return createElement(MeshContext.Provider, { value }, children);
}

export function useMeshNetwork(): MeshSnapshot {
  const ctx = useContext(MeshContext);
  if (!ctx) throw new Error("useMeshNetwork must be used within MeshProvider");
  return ctx;
}

export function useOnlineStatus(): boolean {
  const [online, setOnline] = useState(
    typeof navigator !== "undefined" ? navigator.onLine : false,
  );
  const sync = useCallback(() => setOnline(navigator.onLine), []);
  useEffect(() => {
    window.addEventListener("online", sync);
    window.addEventListener("offline", sync);
    return () => {
      window.removeEventListener("online", sync);
      window.removeEventListener("offline", sync);
    };
  }, [sync]);
  return online;
}