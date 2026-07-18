import { useCallback, useState } from "react";
import "./App.css";
import { NetworkVisualizer } from "./components/NetworkVisualizer";
import { Chessboard } from "./components/Chessboard";
import { NetworkStatus } from "./components/NetworkStatus";
import { ErrorBoundary } from "./components/ErrorBoundary";
import { JobSubmit } from "./components/JobSubmit";
import { MeshProvider, useOnlineStatus } from "./hooks/useMeshNetwork";

const OfflineFallback = () => (
  <div className="offline-notice" role="status">
    You are currently offline. Local mesh functions and chess remain available on this island.
  </div>
);

const NotFound = () => (
  <div className="not-found" role="alert">
    Error 404: The requested Captive Portal page was not found on this node.
  </div>
);

function PortalBody() {
  const path = window.location.pathname;
  const online = useOnlineStatus();
  const [fen, setFen] = useState<string | undefined>(undefined);
  const onFenChange = useCallback((next: string) => setFen(next), []);

  return (
    <div className="app-shell">
      <div className="glass-panel">
        <NetworkStatus />
        {path === "/404" && <NotFound />}
        {!online && <OfflineFallback />}
        <div className="status-badge portal-badge">
          <span className="status-dot" />
          CAPTIVE PORTAL ACTIVE
        </div>
        <h1>MessyMash</h1>
        <p className="subtitle">Offline-First Decentralized Chess Grid</p>
        <p className="portal-copy">
          Welcome to the offline Captive Portal. You are connected to an isolated mesh island.
          No internet is required. Game state lives on-device; peers sync via Kademlia DHT and LoRa
          when available.
        </p>
        <Chessboard onFenChange={onFenChange} />
        <JobSubmit fen={fen} />
        <div id="network" className="visualizer-section">
          <NetworkVisualizer />
        </div>
      </div>
    </div>
  );
}

function App() {
  return (
    <ErrorBoundary>
      <MeshProvider>
        <PortalBody />
      </MeshProvider>
    </ErrorBoundary>
  );
}

export default App;
