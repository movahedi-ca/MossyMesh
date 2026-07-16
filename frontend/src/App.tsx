import { NetworkVisualizer } from './components/NetworkVisualizer';

function App() {
  return (
    <div className="glass-panel">
      <div className="status-badge">
        <span className="status-dot"></span>
        MESH CONNECTION ESTABLISHED
      </div>
      
      <h1>MossyMesh</h1>
      <p className="subtitle">Offline-First Decentralized Compute Grid</p>

      <p style={{ lineHeight: '1.6', color: 'var(--text-muted)' }}>
        Welcome to the offline Captive Portal. You are currently connected to an isolated mesh island.
        No internet is required. All data is routed locally via Kademlia DHT and LoRa.
      </p>

      <NetworkVisualizer />
    </div>
  );
}

export default App;
