import { useCallback, useMemo, useState } from "react";
import { Chess, type Square, type Move } from "chess.js";
import { playMoveSound, playCheckmateSound } from "../lib/sound";
import { formatEval, useEngineEval } from "../lib/engineEval";
import { getStrings, initialLang, type AppLang, type AppStrings } from "../i18n";
import "./Chessboard.css";

const FILES = ["a", "b", "c", "d", "e", "f", "g", "h"] as const;
const RANKS = [8, 7, 6, 5, 4, 3, 2, 1] as const;

const PIECE_GLYPH: Record<string, string> = {
  wK: "♔", wQ: "♕", wR: "♖", wB: "♗", wN: "♘", wP: "♙",
  bK: "♚", bQ: "♛", bR: "♜", bB: "♝", bN: "♞", bP: "♟",
};

function squareName(file: number, rank: number): Square {
  return `${FILES[file]}${rank}` as Square;
}

function describeStatus(game: Chess, t: AppStrings["chess"]): string {
  if (game.isCheckmate()) {
    return game.turn() === "w" ? t.checkmateBlackWins : t.checkmateWhiteWins;
  }
  if (game.isStalemate()) return t.stalemate;
  if (game.isThreefoldRepetition()) return t.threefold;
  if (game.isInsufficientMaterial()) return t.insufficientMaterial;
  if (game.isDraw()) return t.draw;
  if (game.isCheck()) {
    return game.turn() === "w" ? t.whiteInCheck : t.blackInCheck;
  }
  return game.turn() === "w" ? t.whiteToMove : t.blackToMove;
}

export const Chessboard = () => {
  const [lang] = useState<AppLang>(initialLang);
  const t = getStrings(lang);
  const [game, setGame] = useState(() => new Chess());
  const [selected, setSelected] = useState<Square | null>(null);
  const [legalTargets, setLegalTargets] = useState<Square[]>([]);
  const [lastMove, setLastMove] = useState<{ from: Square; to: Square } | null>(null);
  const [history, setHistory] = useState<Move[]>([]);
  const [status, setStatus] = useState(t.chess.whiteToMoveReady);
  const [meshNote, setMeshNote] = useState(t.chess.noteLocalOnly);
  const [mode, setMode] = useState<"offline" | "lora">("offline");

  const board = useMemo(() => game.board(), [game]);
  const fen = useMemo(() => game.fen(), [game]);
  const engineEval = useEngineEval(fen);

  const clearSelection = useCallback(() => {
    setSelected(null);
    setLegalTargets([]);
  }, []);

  const publishMove = useCallback(
    async (from: Square, to: Square) => {
      if (!navigator.onLine || mode === "offline") {
        setMeshNote(
          mode === "lora" ? t.chess.noteQueuedLora : t.chess.noteAppliedLocal,
        );
        return;
      }
      setMeshNote(t.chess.noteSubmitting);
      try {
        const response = await fetch("/api/v1/submit_job", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ action: "move", from, to, fen }),
        });
        if (response.ok) setMeshNote(t.chess.noteConfirmed);
        else setMeshNote(t.chess.noteRejected);
      } catch {
        setMeshNote(t.chess.noteUnreachable);
      }
    },
    [fen, mode, t],
  );

  const applyMove = useCallback(
    (from: Square, to: Square) => {
      const next = new Chess(game.fen());
      let result: Move | null = null;
      try {
        result = next.move({ from, to, promotion: "q" });
      } catch {
        result = null;
      }
      if (!result) {
        setStatus(t.chess.illegalMove);
        clearSelection();
        return;
      }
      setGame(next);
      setHistory(next.history({ verbose: true }));
      setLastMove({ from, to });
      setStatus(describeStatus(next, t.chess));
      clearSelection();
      if (next.isCheckmate()) playCheckmateSound();
      else playMoveSound();
      void publishMove(from, to);
    },
    [game, clearSelection, publishMove, t],
  );

  const onSquareClick = useCallback(
    (sq: Square) => {
      if (game.isGameOver()) return;
      if (selected) {
        if (sq === selected) {
          clearSelection();
          return;
        }
        if (legalTargets.includes(sq)) {
          applyMove(selected, sq);
          return;
        }
      }
      const piece = game.get(sq);
      if (!piece || piece.color !== game.turn()) {
        clearSelection();
        return;
      }
      const moves = game.moves({ square: sq, verbose: true });
      setSelected(sq);
      setLegalTargets(moves.map((m) => m.to));
    },
    [game, selected, legalTargets, applyMove, clearSelection],
  );

  const resetGame = () => {
    setGame(new Chess());
    setHistory([]);
    setLastMove(null);
    clearSelection();
    setStatus(t.chess.whiteToMoveReady);
    setMeshNote(t.chess.noteNewGame);
  };

  const undoMove = () => {
    const next = new Chess(game.fen());
    if (!next.undo()) return;
    setGame(next);
    const hist = next.history({ verbose: true });
    setHistory(hist);
    const prev = hist[hist.length - 1];
    setLastMove(prev ? { from: prev.from, to: prev.to } : null);
    setStatus(describeStatus(next, t.chess));
    clearSelection();
    setMeshNote(t.chess.noteUndone);
  };

  const formatHistory = () => {
    const pairs: string[] = [];
    for (let i = 0; i < history.length; i += 2) {
      const n = Math.floor(i / 2) + 1;
      const w = history[i]?.san ?? "";
      const b = history[i + 1]?.san;
      pairs.push(b ? `${n}. ${w} ${b}` : `${n}. ${w}`);
    }
    return pairs;
  };

  return (
    <div className="chessboard-container">
      <div className="chess-meta">
        <div className={`game-status ${game.isCheck() ? "in-check" : ""} ${game.isGameOver() ? "game-over" : ""}`}>
          {status}
          <span
            className="engine-eval"
            title={engineEval.source === "daemon" ? t.chess.evalDaemonTitle : t.chess.evalLocalTitle}
            aria-label={t.chess.evalAria(engineEval.score === null ? t.chess.evalLoading : formatEval(engineEval.score))}
            style={{ marginLeft: 8, opacity: 0.85, fontVariantNumeric: "tabular-nums" }}
          >
            {engineEval.score === null ? "…" : t.chess.evalText(formatEval(engineEval.score))}
          </span>
        </div>
        <div className="mesh-note">{meshNote}</div>
      </div>
      <div className="chessboard" role="grid" aria-label="Chessboard">
        {RANKS.map((rank, rankIdx) =>
          FILES.map((_file, fileIdx) => {
            const sq = squareName(fileIdx, rank);
            // rankIdx and fileIdx are both 0-indexed here (RANKS[0] is rank 8).
            // a1 is dark: rankIdx 7 + fileIdx 0 = 7, odd -> dark. h1 (7 + 7)
            // is even -> light. Regression guard: keep this parity, a1 dark.
            const isDark = (rankIdx + fileIdx) % 2 === 1;
            const piece = board[rankIdx][fileIdx];
            const glyph = piece ? PIECE_GLYPH[`${piece.color}${piece.type.toUpperCase()}`] : "";
            const isSelected = selected === sq;
            const isLegal = legalTargets.includes(sq);
            const isLast = !!(lastMove && (lastMove.from === sq || lastMove.to === sq));
            const isCaptureHint = isLegal && !!piece;
            return (
              <button
                type="button"
                key={sq}
                className={[
                  "square",
                  isDark ? "dark-square" : "light-square",
                  isSelected ? "selected" : "",
                  isLegal ? "legal" : "",
                  isCaptureHint ? "capture" : "",
                  isLast ? "last-move" : "",
                ].filter(Boolean).join(" ")}
                onClick={() => onSquareClick(sq)}
                aria-label={glyph ? `${sq} ${glyph}` : sq}
              >
                {glyph && (
                  <span className={`piece ${piece?.color === "w" ? "white" : "black"}`}>{glyph}</span>
                )}
                {isLegal && !piece && <span className="legal-dot" />}
              </button>
            );
          }),
        )}
      </div>
      <div className="chess-history" aria-live="polite">
        <div className="history-label">{t.chess.moveHistory}</div>
        <div className="history-scroll">
          {history.length === 0 ? (
            <span className="history-empty">{t.chess.noMoves}</span>
          ) : (
            formatHistory().map((line) => (
              <span key={line} className="history-entry">{line}</span>
            ))
          )}
        </div>
      </div>
      <div className="chessboard-controls">
        <button type="button" className={`mesh-btn ${mode === "offline" ? "primary" : "secondary"}`}
          onClick={() => { setMode("offline"); setMeshNote(t.chess.notePlayOffline); }}>
          {t.chess.playOffline}
        </button>
        <button type="button" className={`mesh-btn ${mode === "lora" ? "primary" : "secondary"}`}
          onClick={() => { setMode("lora"); setMeshNote(t.chess.noteSeeking); }}>
          {t.chess.seekPeer}
        </button>
        <button type="button" className="mesh-btn secondary" onClick={undoMove} disabled={history.length === 0}>{t.chess.undo}</button>
        <button type="button" className="mesh-btn secondary" onClick={resetGame}>{t.chess.newGame}</button>
      </div>
    </div>
  );
};