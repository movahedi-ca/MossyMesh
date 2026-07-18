import { useCallback, useEffect, useState } from "react";
import {
  type HostReachability,
  probeMeshHost,
  submitJob,
} from "../lib/meshApi";
import "./JobSubmit.css";

export interface JobSubmitProps {
  /** Current board FEN to attach when submitting a chess job. */
  fen?: string;
}

/**
 * Explicit mesh job panel — probes /api/v1/health and POSTs /api/v1/submit_job
 * when the local mesh host is up. Works on captive islands even when the
 * browser reports offline (navigator.onLine is often false on mesh APs).
 */
export function JobSubmit({ fen }: JobSubmitProps) {
  const [host, setHost] = useState<HostReachability>("unknown");
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState("Probe mesh host when ready.");
  const [payload, setPayload] = useState("chess_eval");

  const refresh = useCallback(async () => {
    setHost("unknown");
    setNote("Probing /api/v1/health…");
    const up = await probeMeshHost();
    setHost(up ? "up" : "down");
    setNote(
      up
        ? "Mesh host up — job submit available"
        : "Host down or unreachable — play stays local",
    );
  }, []);

  useEffect(() => {
    void refresh();
    const id = window.setInterval(() => {
      void probeMeshHost().then((up) => setHost(up ? "up" : "down"));
    }, 15000);
    return () => window.clearInterval(id);
  }, [refresh]);

  const onSubmit = useCallback(async () => {
    setBusy(true);
    setNote("POST /api/v1/submit_job…");
    const result = await submitJob({
      action: "submit_job",
      kind: payload || "chess_eval",
      fen: fen ?? undefined,
      payload: payload || "chess_eval",
    });
    setBusy(false);
    if (result.ok) {
      setHost("up");
      setNote(
        result.body
          ? `Accepted: ${result.body.slice(0, 120)}`
          : "Job accepted by mesh host",
      );
    } else if (result.status === 0) {
      setHost("down");
      setNote("Mesh unreachable — kept local (no host)");
    } else {
      setNote(`Host responded ${result.status} — kept local state`);
    }
  }, [fen, payload]);

  const tone = host === "up" ? "up" : host === "down" ? "down" : "unknown";

  return (
    <section className="job-submit" aria-label="Mesh job submit">
      <div className="job-submit-header">
        <h2 className="job-submit-title">Mesh job</h2>
        <span className={`job-host job-host--${tone}`} role="status">
          <span className="status-dot" aria-hidden="true" />
          {host === "up" ? "Host up" : host === "down" ? "Host down" : "Checking…"}
        </span>
      </div>
      <p className="job-submit-lede">
        When the interop host is listening, jobs go to{" "}
        <code>/api/v1/submit_job</code> via nginx. Offline islands keep chess local.
      </p>
      <label className="job-field">
        <span>Job kind</span>
        <input
          type="text"
          value={payload}
          onChange={(e) => setPayload(e.target.value)}
          placeholder="chess_eval"
          disabled={busy}
          autoComplete="off"
        />
      </label>
      {fen && (
        <div className="job-fen" title={fen}>
          FEN attached · {fen.split(" ").slice(0, 2).join(" ")}…
        </div>
      )}
      <div className="job-submit-actions">
        <button
          type="button"
          className="mesh-btn primary"
          onClick={() => void onSubmit()}
          disabled={busy}
        >
          {busy ? "Submitting…" : "Submit job"}
        </button>
        <button
          type="button"
          className="mesh-btn secondary"
          onClick={() => void refresh()}
          disabled={busy}
        >
          Probe host
        </button>
      </div>
      <div className="job-submit-note" aria-live="polite">
        {note}
      </div>
    </section>
  );
}
