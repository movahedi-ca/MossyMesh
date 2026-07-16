import { useMeshNetwork, type MeshLinkMode } from "../hooks/useMeshNetwork";

function labelFor(mode: MeshLinkMode, islandName: string): { text: string; tone: "online" | "island" | "local" } {
  switch (mode) {
    case "internet": return { text: "Mesh Bridged · WAN", tone: "online" };
    case "island": return { text: `Island · ${islandName}`, tone: "island" };
    default: return { text: "Offline · Local Island", tone: "local" };
  }
}

export const NetworkStatus = () => {
  const mesh = useMeshNetwork();
  const badge = labelFor(mesh.linkMode, mesh.islandName);
  return (
    <div className="network-status-stack">
      <div className={`status-badge mesh-link mesh-link--${badge.tone}`}>
        <span className={`status-dot mesh-dot--${badge.tone}`} />
        {badge.text}
      </div>
      <div className="mesh-metrics" title={mesh.lastSyncLabel}>
        <span><strong>{mesh.peerCount}</strong> peers</span>
        <span className="mesh-metrics-sep">·</span>
        <span><strong>{mesh.loraPeers}</strong> LoRa</span>
        <span className="mesh-metrics-sep">·</span>
        <span className={mesh.dhtReady ? "dht-ready" : "dht-boot"}>
          {mesh.dhtReady ? "DHT ready" : "DHT…"}
        </span>
      </div>
    </div>
  );
};