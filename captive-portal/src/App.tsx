import './App.css'

/**
 * MessyMash captive-portal landing.
 * Shown when devices join the offline mesh Wi-Fi AP and OS connectivity
 * checks are redirected by nginx (see nginx.conf).
 */
function App() {
  return (
    <div className="shell">
      <div className="glass-panel">
        <div className="status-badge" role="status">
          <span className="status-dot" aria-hidden="true" />
          CAPTIVE PORTAL ACTIVE
        </div>

        <div className="brand-mark" aria-hidden="true">
          <span className="brand-glyph">M</span>
        </div>

        <h1>MessyMash</h1>
        <p className="subtitle">Offline-First Decentralized Chess Grid</p>

        <p className="lede">
          Welcome to the offline Captive Portal. You are connected to an
          isolated mesh island. No internet is required — packets route locally
          via Kademlia DHT, BLE, and LoRa.
        </p>

        <div className="actions">
          <a className="btn btn-primary" href="/app/">
            Enter Chess Grid
          </a>
          <a className="btn btn-ghost" href="/app/#network">
            View Mesh Status
          </a>
        </div>

        <ul className="feature-list">
          <li>
            <strong>Serverless</strong>
            <span>No ISP, DNS, or cloud dependency</span>
          </li>
          <li>
            <strong>Deterministic</strong>
            <span>Cross-device state transitions for chess PoC</span>
          </li>
          <li>
            <strong>Edge-ready</strong>
            <span>Pi, phone, and ESP32 mesh nodes</span>
          </li>
        </ul>

        <footer className="portal-footer">
          <span>MossyMesh · MessyMash.com</span>
          <span className="sep">·</span>
          <span>150M asset transfers enabled</span>
        </footer>
      </div>
    </div>
  )
}

export default App
