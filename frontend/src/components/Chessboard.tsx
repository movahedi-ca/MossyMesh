import { useCallback, useEffect, useMemo, useState } from "react";
import { Chess, type Square, type Move } from "chess.js";
import { submitJob } from "../lib/meshApi";
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

function describeStatus(game: Chess): string {
  if (game.isCheckmate()) {
    return game.turn() === "w" ? "Checkmate — Black wins" : "Checkmate — White wins";
  }
  if (game.isStalemate()) return "Stalemate — draw";
  if (game.isThreefoldRepetition()) return "Draw — threefold repetition";
  if (game.isInsufficientMaterial()) return "Draw — insufficient material";
  if (game.isDraw()) return "Draw";
  if (game.isCheck()) {
    return game.turn() === "w" ? "White in check" : "Black in check";
  }
  return game.turn() === "w" ? "White to move" : "Black to move";
}

export interface ChessboardProps {
  /** Optional FEN change callback (e.g. job submit panel). */
  onFenChange?: (fen: string) => void;
}

export const Chessboard = ({ onFenChange }: ChessboardProps = {}) => {
  const [game, setGame] = useState(() => new Chess());
  const [selected, setSelected] = useState<Square | null>(null);
  const [legalTargets, setLegalTargets] = useState<Square[]>([]);
  const [lastMove, setLastMove] = useState<{ from: Square; to: Square } | null>(null);
  const [history, setHistory] = useState<Move[]>([]);
  const [status, setStatus] = useState("White to move · offline engine ready");
  const [meshNote, setMeshNote] = useState("Moves stay on-device until a mesh peer is found.");
  const [mode, setMode] = useState<"offline" | "lora">("offline");

  const board = useMemo(() => game.board(), [game]);
  const fen = useMemo(() => game.fen(), [game]);

  useEffect(() => {
    onFenChange?.(fen);
  }, [fen, onFenChange]);

  const clearSelection = useCallback(() => {
    setSelected(null);
    setLegalTargets([]);
  }, []);

  const publishMove = useCallback(
    async (from: Square, to: Square) => {
      // Pure local play — never require WAN. Captive APs often set
      // navigator.onLine=false even when the mesh host answers /api.
      if (mode === "offline") {
        setMeshNote("Offline: move applied locally (no internet required)");
        return;
      }
      setMeshNote("Submitting move to mesh host…");
      const result = await submitJob({ action: "move", from, to, fen });
      if (result.ok) {
        setMeshNote(
          result.body
            ? `Move confirmed: ${result.body.slice(0, 80)}`
            : "Move confirmed by swarm",
        );
      } else if (result.status === 0) {
        setMeshNote("Mesh unreachable — queued local (DHT island mode)");
      } else {
        setMeshNote(`Host ${result.status} — kept local state`);
      }
    },
    [fen, mode],
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
        setStatus("Illegal move");
        clearSelection();
        return;
      }
      setGame(next);
      setHistory(next.history({ verbose: true }));
      setLastMove({ from, to });
      setStatus(describeStatus(next));
      clearSelection();
      void publishMove(from, to);
    },
    [game, clearSelection, publishMove],
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
    setStatus("White to move · offline engine ready");
    setMeshNote("New game — local FEN store reset");
  };

  const undoMove = () => {
    const next = new Chess(game.fen());
    if (!next.undo()) return;
    setGame(next);
    const hist = next.history({ verbose: true });
    setHistory(hist);
    const prev = hist[hist.length - 1];
    setLastMove(prev ? { from: prev.from, to: prev.to } : null);
    setStatus(describeStatus(next));
    clearSelection();
    setMeshNote("Undid last move (local only)");
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
        </div>
        <div className="mesh-note">{meshNote}</div>
      </div>
      <div className="chessboard" role="grid" aria-label="Chessboard">
        {RANKS.map((rank, rankIdx) =>
          FILES.map((_file, fileIdx) => {
            const sq = squareName(fileIdx, rank);
            const isDark = (rank + fileIdx) % 2 === 0;
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
        <div className="history-label">Move history</div>
        <div className="history-scroll">
          {history.length === 0 ? (
            <span className="history-empty">No moves yet — play offline</span>
          ) : (
            formatHistory().map((line) => (
              <span key={line} className="history-entry">{line}</span>
            ))
          )}
        </div>
      </div>
      <div className="chessboard-controls">
        <button type="button" className={`mesh-btn ${mode === "offline" ? "primary" : "secondary"}`}
          onClick={() => { setMode("offline"); setMeshNote("Play offline — pure local chess engine"); }}>
          Play offline
        </button>
        <button type="button" className={`mesh-btn ${mode === "lora" ? "primary" : "secondary"}`}
          onClick={() => { setMode("lora"); setMeshNote("Seeking peer via LoRa / mesh island…"); }}>
          Seek peer via LoRa
        </button>
        <button type="button" className="mesh-btn secondary" onClick={undoMove} disabled={history.length === 0}>Undo</button>
        <button type="button" className="mesh-btn secondary" onClick={resetGame}>New game</button>
      </div>
    </div>
  );
};
