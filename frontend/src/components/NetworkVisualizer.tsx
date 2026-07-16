import React, { useEffect, useState } from 'react';

interface NodeInfo {
  id: string;
  type: 'hub' | 'edge' | 'genesis';
  latency: number;
  status: 'active' | 'syncing' | 'offline';
}

const mockNodes: NodeInfo[] = [
  { id: 'MM-GEN-01', type: 'genesis', latency: 4, status: 'active' },
  { id: 'LORA-EDGE-3A', type: 'edge', latency: 124, status: 'active' },
  { id: 'HUB-NVME-X9', type: 'hub', latency: 12, status: 'syncing' },
];

export const NetworkVisualizer: React.FC = () => {
  const [nodes, setNodes] = useState<NodeInfo[]>([]);

  useEffect(() => {
    // Simulate discovering nodes sequentially
    let delay = 500;
    mockNodes.forEach((node) => {
      setTimeout(() => {
        setNodes(prev => [...prev, node]);
      }, delay);
      delay += Math.random() * 800 + 400; // random delay between 400-1200ms
    });
  }, []);

  const getIcon = (type: string) => {
    switch (type) {
      case 'genesis': return 'π'; // Pi symbol for Raspberry Pi Genesis
      case 'edge': return '〰'; // Radio wave for LoRa Edge
      case 'hub': return '⛁'; // Database for NVMe Hub
      default: return '●';
    }
  };

  const getStatusColor = (status: string) => {
    switch (status) {
      case 'active': return '#45f3ff';
      case 'syncing': return '#c72dfb';
      case 'offline': return '#8b8d98';
      default: return '#fff';
    }
  };

  return (
    <div className="visualizer-container">
      {nodes.length === 0 ? (
        <p className="subtitle">Scanning local mesh environment...</p>
      ) : (
        nodes.map((node) => (
          <div key={node.id} className="node-card">
            <div className="node-icon">{getIcon(node.type)}</div>
            <h3>{node.id}</h3>
            <p style={{ color: getStatusColor(node.status) }}>
              <span className="status-dot" style={{ backgroundColor: getStatusColor(node.status), animation: 'none', width: 6, height: 6 }} />
              {node.status.toUpperCase()}
            </p>
            <p>Ping: {node.latency}ms</p>
          </div>
        ))
      )}
    </div>
  );
};
