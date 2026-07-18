/**
 * Mesh AsyncAPI client (interop contract §6).
 * Hits same-origin /api/v1/* — nginx proxies to the mesh host when up.
 * Captive islands often report navigator.onLine=false even when the local
 * host answers; callers should probe rather than gate solely on online.
 */

export const MESH_API = {
  health: "/api/v1/health",
  submitJob: "/api/v1/submit_job",
} as const;

export type HostReachability = "unknown" | "up" | "down";

export interface SubmitJobBody {
  action: string;
  from?: string;
  to?: string;
  fen?: string;
  payload?: string;
  [key: string]: unknown;
}

export interface SubmitJobResult {
  ok: boolean;
  status: number;
  body: string;
  error?: string;
}

const DEFAULT_TIMEOUT_MS = 4000;

async function fetchWithTimeout(
  input: RequestInfo | URL,
  init: RequestInit = {},
  timeoutMs = DEFAULT_TIMEOUT_MS,
): Promise<Response> {
  const controller = new AbortController();
  const timer = window.setTimeout(() => controller.abort(), timeoutMs);
  try {
    return await fetch(input, { ...init, signal: controller.signal });
  } finally {
    window.clearTimeout(timer);
  }
}

/** GET /api/v1/health — true when mesh host (or nginx stub) answers 2xx. */
export async function probeMeshHost(timeoutMs = 2500): Promise<boolean> {
  try {
    const res = await fetchWithTimeout(
      MESH_API.health,
      { method: "GET", cache: "no-store" },
      timeoutMs,
    );
    return res.ok;
  } catch {
    return false;
  }
}

/** POST /api/v1/submit_job — enqueue chess/compute job when host is up. */
export async function submitJob(
  body: SubmitJobBody,
  timeoutMs = DEFAULT_TIMEOUT_MS,
): Promise<SubmitJobResult> {
  try {
    const res = await fetchWithTimeout(
      MESH_API.submitJob,
      {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          Accept: "application/json, text/plain, */*",
        },
        body: JSON.stringify(body),
        cache: "no-store",
      },
      timeoutMs,
    );
    const text = await res.text().catch(() => "");
    return { ok: res.ok, status: res.status, body: text };
  } catch (err) {
    const message = err instanceof Error ? err.message : "network error";
    return { ok: false, status: 0, body: "", error: message };
  }
}
