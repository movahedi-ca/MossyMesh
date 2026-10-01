import { useEffect, useState } from "react";
import "./App.css";
import { NetworkVisualizer } from "./components/NetworkVisualizer";
import { Chessboard } from "./components/Chessboard";
import { NetworkStatus } from "./components/NetworkStatus";
import { DaemonToast } from "./components/DaemonToast";
import { ErrorBoundary } from "./components/ErrorBoundary";
import { MeshProvider, useOnlineStatus } from "./hooks/useMeshNetwork";
import { getStrings, initialLang, type AppLang } from "./i18n";

const OfflineFallback = ({ lang }: { lang: AppLang }) => (
  <div className="offline-notice" role="status">
    {getStrings(lang).app.offlineNotice}
  </div>
);

const NotFound = ({ lang }: { lang: AppLang }) => (
  <div className="not-found" role="alert">
    {getStrings(lang).app.notFound}
  </div>
);

function PortalBody() {
  const path = window.location.pathname;
  const online = useOnlineStatus();
  const [lang] = useState<AppLang>(initialLang);
  const t = getStrings(lang);

  useEffect(() => {
    document.documentElement.lang = lang;
  }, [lang]);

  return (
    <div className="app-shell">
      <div className="glass-panel">
        <NetworkStatus />
        {path === "/404" && <NotFound lang={lang} />}
        {!online && <OfflineFallback lang={lang} />}
        <div className="status-badge portal-badge">
          <span className="status-dot" />
          {t.app.badge}
        </div>
        <h1>MessyMash</h1>
        <p className="subtitle">{t.app.subtitle}</p>
        <p className="portal-copy">{t.app.portalCopy}</p>
        <Chessboard />
        <div className="visualizer-section">
          <NetworkVisualizer />
        </div>
      </div>
      <DaemonToast />
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
