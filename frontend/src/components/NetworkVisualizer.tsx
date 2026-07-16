import { useMemo } from "react";
import { useMeshNetwork, type MeshNode } from "../hooks/useMeshNetwork";

const getIcon = (type: MeshNode["type"]) => {
  switch (type) {
    case "genesis": return "π";
    case "edge": return "〰";
    case "hub": return "⛁";
    case "peer": return "◈";
    default: return "●";
  }
};

const getStatusColor = (status: MeshNode["status"]) => {
  switch (status) {
    case "active": return "#45f3ff";
    case "syncing": return "#c72dfb";
    case "island": return "#ffb347";
    case "offline": return "#8b8d98";
    default: return "#fff";
  }
};

export const NetworkVisualizer = () => {
  const mesh = useMeshNetwork();
  const islands = useMemo(() => {
    const map = new Map<string, MeshNode[]>();
    for (const node of mesh.nodes) {
      const list = map.get(node.islandId) ?? [];
      list.push(node);
      map.set(node.islandId, list);
    }
    return Array.from(map.entries());
  }, [mesh.nodes]);

  return (
    <div className="visualizer-wrap">
      <div className="visualizer-header">
        <h2 className="visualizer-title">Mesh islands</h2>
        <p className="subtitle visualizer-sub">
          {mesh.nodes.length === 0
            ? "Scanning local mesh environment…"
            : `${islands.length} island${islands.length === 1 ? "" : "s"} · ${mesh.peerCount} live nodes · ${mesh.islandName}`}
        </p>
      </div>
      {mesh.nodes.length === 0 ? (
        <div className="visualizer-scanning">
          <span className="status-dot" />
          Probing Kademlia + LoRa beacons
        </div>
      ) : (
        <div className="island-list">
          {islands.map(([islandId, members]) => (
            <section key={islandId} className="island-group">
              <div className="island-label">
                <span className="island-chip">{islandId}</span>
                <span className="island-count">{members.length} nodes</span>
              </div>
              <div className="visualizer-container">
                {members.map((node) => (
                  <div key={node.id} className={`node-card node-${node.type} node-status-${node.status}`}>
                    <div className="node-icon">{getIcon(node.type)}</div>
                    <h3>{node.id}</h3>
                    <p style={{ color: getStatusColor(node.status) }}>
                      <span className="status-dot" style={{
                        backgroundColor: getStatusColor(node.status),
                        animation: node.status === "active" ? undefined : "none",
                        width: 6, height: 6,
                      }} />
                      {node.status.toUpperCase()}
                    </p>
                    <p>Ping: {node.latency}ms · {node.hops} hop{node.hops === 1 ? "" : "s"}</p>
                    <p className="node-type-tag">{node.type}</p>
                  </div>
                ))}
              </div>
            </section>
          ))}
        </div>
      )}
      <div className="mesh-topology-hint">
        Isolated islands exchange games via store-and-forward when a bridge peer appears.
        No WAN required for local play.
      </div>
    </div>
  );
};