import { useEffect, useState } from 'react'
import './App.css'

/**
 * MessyMash captive-portal landing.
 * Shown when devices join the offline mesh Wi-Fi AP and OS connectivity
 * checks are redirected by nginx (see nginx.conf).
 */

const THEME_KEY = 'mossymesh-portal-theme'
type Theme = 'dark' | 'light'

function initialTheme(): Theme {
  try {
    const saved = window.localStorage.getItem(THEME_KEY)
    if (saved === 'light' || saved === 'dark') return saved
  } catch {
    // Storage unavailable (private mode); fall back to dark.
  }
  return 'dark'
}

function App() {
  const [theme, setTheme] = useState<Theme>(initialTheme)

  useEffect(() => {
    document.documentElement.dataset.theme = theme
    try {
      window.localStorage.setItem(THEME_KEY, theme)
    } catch {
      // Storage unavailable; theme still applies for this session.
    }
  }, [theme])

  const toggleTheme = () => setTheme((t) => (t === 'dark' ? 'light' : 'dark'))

  return (
    <div className="shell">
      <div className="glass-panel">
        <button
          type="button"
          className="theme-toggle"
          onClick={toggleTheme}
          aria-label={theme === 'dark' ? 'Switch to light theme' : 'Switch to dark theme'}
          title={theme === 'dark' ? 'Light theme' : 'Dark theme'}
        >
          <span aria-hidden="true">{theme === 'dark' ? '☀' : '☾'}</span>
        </button>

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
