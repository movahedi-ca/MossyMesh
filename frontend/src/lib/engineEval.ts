import { useEffect, useState } from "react";

/**
 * Engine evaluation score for the React UI.
 *
 * The score is a white-perspective centipawn value. When the mesh daemon is
 * reachable it is fetched from `/api/v1/engine_eval` (served by the WAMR
 * sandbox build of the Rust engine); otherwise a deterministic local
 * material evaluation runs on-device so the offline-first UI never blocks.
 */

const PIECE_VALUE: Record<string, number> = {
  p: 100, n: 320, b: 330, r: 500, q: 900, k: 0,
};

/** Small centrality bonus so the local eval is not pure material. */
function squareBonus(file: number, rank: number): number {
  const centerFile = 3.5 - Math.abs(3.5 - file);
  const centerRank = 3.5 - Math.abs(3.5 - rank);
  return Math.round((centerFile + centerRank) * 2);
}

/** White-perspective centipawn score parsed from a FEN string. */
export function evaluateFenMaterial(fen: string): number {
  const placement = fen.split(" ")[0] ?? "";
  let score = 0;
  let rank = 0;
  let file = 0;
  for (const ch of placement) {
    if (ch === "/") { rank += 1; file = 0; continue; }
    if (ch >= "1" && ch <= "8") { file += Number(ch); continue; }
    const value = PIECE_VALUE[ch.toLowerCase()] ?? 0;
    const bonus = squareBonus(file, rank);
    score += ch === ch.toUpperCase() ? value + bonus : -(value + bonus);
    file += 1;
  }
  return score;
}

/** "+1.25" style rendering of a centipawn score. */
export function formatEval(centipawns: number): string {
  const pawns = centipawns / 100;
  const sign = pawns > 0 ? "+" : "";
  return `${sign}${pawns.toFixed(2)}`;
}

async function fetchDaemonEval(fen: string): Promise<number | null> {
  try {
    const res = await fetch(
      `/api/v1/engine_eval?fen=${encodeURIComponent(fen)}`,
      { method: "GET" },
    );
    if (!res.ok) return null;
    const data: unknown = await res.json();
    if (
      typeof data === "object" && data !== null &&
      typeof (data as { score_cp?: unknown }).score_cp === "number"
    ) {
      return (data as { score_cp: number }).score_cp;
    }
    return null;
  } catch {
    return null;
  }
}

export interface EngineEval {
  /** White-perspective centipawns, or null while loading. */
  score: number | null;
  source: "daemon" | "local";
}

/** Engine score for a position; prefers the daemon, falls back to local eval. */
export function useEngineEval(fen: string): EngineEval {
  const [evalState, setEvalState] = useState<EngineEval>({ score: null, source: "local" });

  useEffect(() => {
    let cancelled = false;
    // Instant local answer so the UI never waits on the network.
    setEvalState({ score: evaluateFenMaterial(fen), source: "local" });
    if (typeof navigator !== "undefined" && navigator.onLine) {
      void fetchDaemonEval(fen).then((score) => {
        if (!cancelled && score !== null) {
          setEvalState({ score, source: "daemon" });
        }
      });
    }
    return () => { cancelled = true; };
  }, [fen]);

  return evalState;
}
