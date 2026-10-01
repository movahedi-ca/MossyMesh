import { useEffect, useState } from 'react'
import './App.css'
import {
  getStrings,
  initialLang,
  persistLang,
  LANG_LABEL,
  PORTAL_LANGS,
  type PortalLang,
} from './i18n'

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

  const [lang, setLang] = useState<PortalLang>(initialLang)
  const t = getStrings(lang)

  useEffect(() => {
    document.documentElement.lang = lang
  }, [])

  const changeLang = (next: PortalLang) => {
    setLang(next)
    persistLang(next)
    document.documentElement.lang = next
  }

  return (
    <div className="shell">
      <div className="glass-panel">
        <div
          className="portal-controls"
          style={{ alignSelf: "flex-end", display: "flex", gap: 8, alignItems: "center" }}
        >
          <button
            type="button"
            className="theme-toggle"
            onClick={toggleTheme}
            aria-label={theme === 'dark' ? t.themeToLightAria : t.themeToDarkAria}
            title={theme === 'dark' ? t.themeLightTitle : t.themeDarkTitle}
          >
            <span aria-hidden="true">{theme === 'dark' ? '☀' : '☾'}</span>
          </button>
          <div
            className="lang-picker"
            style={{ display: "flex", gap: 8, alignItems: "center", fontSize: 12, opacity: 0.9 }}
          >
            <label htmlFor="portal-lang">{t.languageLabel}</label>
            <select
              id="portal-lang"
              value={lang}
              onChange={(e) => changeLang(e.target.value as PortalLang)}
            >
              {PORTAL_LANGS.map((l) => (
                <option key={l} value={l}>
                  {LANG_LABEL[l]}
                </option>
              ))}
            </select>
          </div>
        </div>

        <div className="status-badge" role="status">
          <span className="status-dot" aria-hidden="true" />
          {t.badge}
        </div>

        <div className="brand-mark" aria-hidden="true">
          <span className="brand-glyph">M</span>
        </div>

        <h1>MessyMash</h1>
        <p className="subtitle">{t.subtitle}</p>

        <p className="lede">{t.lede}</p>

        <div className="actions">
          <a className="btn btn-primary" href="/app/">
            {t.enterGrid}
          </a>
          <a className="btn btn-ghost" href="/app/#network">
            {t.viewMesh}
          </a>
        </div>

        <ul className="feature-list">
          {t.features.map((f) => (
            <li key={f.title}>
              <strong>{f.title}</strong>
              <span>{f.body}</span>
            </li>
          ))}
        </ul>

        <footer className="portal-footer">
          <span>MossyMesh · MessyMash.com</span>
          <span className="sep">·</span>
          <span>{t.footerNote}</span>
        </footer>
      </div>
    </div>
  )
}

export default App
