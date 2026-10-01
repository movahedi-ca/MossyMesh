import { useEffect, useState } from "react";
import { useDaemonHealth } from "../lib/daemonHealth";

/**
 * Toast shown when the local mesh daemon becomes unreachable (#19, #204).
 *
 * This watches the daemon itself (periodic GET /api/v1/health), not the
 * browser's network state: navigator.onLine can be true while the daemon
 * is down, and false while the island mesh is fine.
 *
 * The chessboard fetch path already degrades silently into island mode;
 * this makes the disconnect visible so failures are not silent.
 */
export const DaemonToast = () => {
  const health = useDaemonHealth();
  const [dismissed, setDismissed] = useState(false);

  useEffect(() => {
    if (health === "reachable") setDismissed(false);
  }, [health]);

  if (health !== "unreachable" || dismissed) return null;

  return (
    <div className="daemon-toast" role="alert" aria-live="assertive">
      <span className="status-dot daemon-toast-dot" aria-hidden="true" />
      <span>
        Local daemon unreachable. Island mode active: chess and mesh
        continue on-device.
      </span>
      <button
        type="button"
        className="daemon-toast-close"
        aria-label="Dismiss notification"
        onClick={() => setDismissed(true)}
      >
        ✕
      </button>
    </div>
  );
};
