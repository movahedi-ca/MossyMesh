import { NetworkVisualizer } from './components/NetworkVisualizer';
import { Chessboard } from './components/Chessboard';
import { NetworkStatus } from './components/NetworkStatus';
import './App.css';

function App() {
  return (
    <>
      <NetworkStatus />
      <div className="glass-panel">
        <div className="status-badge">
          <span className="status-dot"></span>
          CAPTIVE PORTAL ACTIVE
        </div>
        
        <h1>MessyMash</h1>
        <p className="subtitle">Offline-First Decentralized Chess Grid</p>

        <p style={{ lineHeight: '1.6', color: 'var(--text-muted)' }}>
          Welcome to the offline Captive Portal. You are currently connected to an isolated mesh island.
          No internet is required. All data is routed locally via Kademlia DHT and LoRa.
        </p>

        <Chessboard />
        
        <div style={{ marginTop: '2rem' }}>
          <NetworkVisualizer />
        </div>
      </div>
    </>
  );
}

export default App;
