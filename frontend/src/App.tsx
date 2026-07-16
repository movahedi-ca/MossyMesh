import { useState } from 'react'
import './App.css'
import { NetworkVisualizer } from './components/NetworkVisualizer';
import { Chessboard } from './components/Chessboard';
import { NetworkStatus } from './components/NetworkStatus';
import { ErrorBoundary } from './components/ErrorBoundary';

// Update 4: Offline Fallback Stub
const OfflineFallback = () => <div className="offline-notice">You are currently offline. Local mesh functions remain available.</div>;

// Update 5: 404 Route Stub
const NotFound = () => <div className="not-found">Error 404: The requested Captive Portal page was not found on this node.</div>;

function App() {
  const path = window.location.pathname;

  return (
    <ErrorBoundary>
      <div className="glass-panel">
        <NetworkStatus />
        
        {path === '/404' && <NotFound />}
        {!navigator.onLine && <OfflineFallback />}

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
