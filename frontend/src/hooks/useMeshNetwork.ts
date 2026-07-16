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
  status: "active" | "syncing" | "offline" | "island";
  islandId: string;
  hops: number;
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
  { id: "MM-GEN-01", type: "genesis", latency: 4, status: "active", islandId: "island-alpha", hops: 0 },
  { id: "LORA-EDGE-3A", type: "edge", latency: 124, status: "active", islandId: "island-alpha", hops: 1 },
  { id: "HUB-NVME-X9", type: "hub", latency: 12, status: "syncing", islandId: "island-alpha", hops: 1 },
  { id: "PEER-PHN-7C", type: "peer", latency: 48, status: "active", islandId: "island-beta", hops: 2 },
  { id: "LORA-EDGE-9F", type: "edge", latency: 210, status: "island", islandId: "island-beta", hops: 2 },
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
          let status = n.status;
          if (n.status === "syncing" && Math.random() > 0.7) status = "active";
          return { ...n, latency, status };
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