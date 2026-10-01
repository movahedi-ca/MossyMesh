import { useMemo, useState } from "react";
import { useMeshNetwork, rssiBars, type MeshNode } from "../hooks/useMeshNetwork";
import { getStrings, initialLang, type AppLang } from "../i18n";

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
  const [lang] = useState<AppLang>(initialLang);
  const t = getStrings(lang);
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
        <h2 className="visualizer-title">{t.visualizer.title}</h2>
        <p className="subtitle visualizer-sub">
          {mesh.nodes.length === 0
            ? t.visualizer.scanning
            : t.visualizer.summary(islands.length, mesh.peerCount, mesh.islandName)}
        </p>
      </div>
      {mesh.nodes.length === 0 ? (
        <div className="visualizer-scanning">
          <span className="status-dot" />
          {t.visualizer.probing}
        </div>
      ) : (
        <div className="island-list">
          {islands.map(([islandId, members]) => (
            <section key={islandId} className="island-group">
              <div className="island-label">
                <span className="island-chip">{islandId}</span>
                <span className="island-count">{t.visualizer.nodes(members.length)}</span>
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
                    <p>{t.visualizer.ping(node.latency, node.hops)}</p>
                    <p
                      className="node-rssi"
                      title={t.visualizer.signal(node.rssi)}
                      aria-label={t.visualizer.signalAria(node.rssi, rssiBars(node.rssi))}
                    >
                      <span aria-hidden="true" style={{ display: "inline-flex", gap: 2, marginRight: 6, alignItems: "flex-end" }}>
                        {[0, 1, 2, 3].map((i) => (
                          <span
                            key={i}
                            style={{
                              width: 3,
                              height: 4 + i * 3,
                              borderRadius: 1,
                              backgroundColor: i < rssiBars(node.rssi) ? "#45f3ff" : "#3a3d4a",
                            }}
                          />
                        ))}
                      </span>
                      {node.rssi} dBm
                    </p>
                    <p className="node-type-tag">{node.type}</p>
                  </div>
                ))}
              </div>
            </section>
          ))}
        </div>
      )}
      <div className="mesh-topology-hint">
        {t.visualizer.hint}
      </div>
    </div>
  );
};
