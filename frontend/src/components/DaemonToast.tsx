import { useEffect, useState } from "react";
import { useOnlineStatus } from "../hooks/useMeshNetwork";

/**
 * Toast shown when the local mesh daemon becomes unreachable (#19).
 *
 * The chessboard fetch path already degrades silently into island mode;
 * this makes the disconnect visible so failures are not silent.
 */
export const DaemonToast = () => {
  const online = useOnlineStatus();
  const [dismissed, setDismissed] = useState(false);

  useEffect(() => {
    if (online) setDismissed(false);
  }, [online]);

  if (online || dismissed) return null;

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
