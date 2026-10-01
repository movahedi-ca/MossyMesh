import { useEffect, useState } from "react";

/**
 * Daemon reachability probe for the "daemon unreachable" toast (#204).
 *
 * The old toast watched `navigator.onLine` (the browser's network state),
 * which says nothing about the mesh daemon. This polls the daemon's own
 * health endpoint instead:
 * - Same-origin `/api/v1/health` first. In production this goes through
 *   the nginx `/api/` proxy (see captive-portal/nginx.conf); under vite
 *   dev it hits the dev-server proxy. If #154's proxy is not deployed yet,
 *   this leg 404s and we fall through.
 * - `http://127.0.0.1:8080/api/v1/health` as fallback, for hosts where the
 *   browser runs on the same machine as the gateway (dev machines,
 *   single-host deployments).
 */

export type DaemonHealth = "unknown" | "reachable" | "unreachable";

const HEALTH_PATH = "/api/v1/health";
const DIRECT_FALLBACK = "http://127.0.0.1:8080/api/v1/health";
const PROBE_TIMEOUT_MS = 5000;

/** Interval between background health probes. */
export const DAEMON_POLL_MS = 15_000;

async function probe(url: string): Promise<boolean> {
  const ctrl = new AbortController();
  const timer = window.setTimeout(() => ctrl.abort(), PROBE_TIMEOUT_MS);
  try {
    const res = await fetch(url, {
      method: "GET",
      signal: ctrl.signal,
      cache: "no-store",
    });
    return res.ok;
  } catch {
    return false;
  } finally {
    window.clearTimeout(timer);
  }
}

/** One-shot check: true when the daemon answers on either route. */
export async function checkDaemonHealth(): Promise<boolean> {
  if (await probe(HEALTH_PATH)) return true;
  return probe(DIRECT_FALLBACK);
}

/**
 * Live daemon health. Starts "unknown" (no toast flash on load), then
 * "reachable"/"unreachable". Rechecks on an interval and immediately on
 * browser online/offline transitions (those only trigger a recheck; the
 * daemon's answer is the source of truth).
 */
export function useDaemonHealth(pollMs: number = DAEMON_POLL_MS): DaemonHealth {
  const [health, setHealth] = useState<DaemonHealth>("unknown");

  useEffect(() => {
    let cancelled = false;
    const run = async () => {
      const ok = await checkDaemonHealth();
      if (!cancelled) setHealth(ok ? "reachable" : "unreachable");
    };
    void run();
    const timer = window.setInterval(() => {
      void run();
    }, pollMs);
    const recheck = () => {
      void run();
    };
    window.addEventListener("online", recheck);
    window.addEventListener("offline", recheck);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
      window.removeEventListener("online", recheck);
      window.removeEventListener("offline", recheck);
    };
  }, [pollMs]);

  return health;
}
