//! Negamax search with hard depth cap and node counters for Mnps-style metrics.

use shakmaty::{Chess, Move, Position};

use crate::eval::{evaluate_side_to_move, MATE_SCORE};
use crate::MAX_DEPTH;

/// Result of a bounded search.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchResult {
    /// Best score from side-to-move perspective (centipawns / mate scores).
    pub score: i32,
    /// Principal variation first move, if any legal move exists.
    pub best_move: Option<Move>,
    /// Nodes visited (every recursive entry counts as one node).
    pub nodes: u64,
    /// Depth actually searched (after clamping to MAX_DEPTH).
    pub depth: u8,
}

/// Negamax with alpha-beta pruning, depth-limited by `depth` and hard-capped by [`MAX_DEPTH`].
///
/// Uses clone-based make/unmake: snapshot the position, `play_unchecked`, recurse, restore.
/// Fully deterministic for a fixed position and depth.
pub fn negamax_search(root: &Chess, depth: u8) -> SearchResult {
    let depth = depth.min(MAX_DEPTH);
    let mut nodes = 0_u64;
    let mut pos = root.clone();
    let (score, best_move) = negamax(&mut pos, depth, -MATE_SCORE, MATE_SCORE, &mut nodes);
    SearchResult {
        score,
        best_move,
        nodes,
        depth,
    }
}

fn negamax(
    pos: &mut Chess,
    depth: u8,
    mut alpha: i32,
    beta: i32,
    nodes: &mut u64,
) -> (i32, Option<Move>) {
    *nodes = nodes.saturating_add(1);

    // Terminal / horizon.
    if pos.is_checkmate() {
        // Prefer faster mates: score already from side-to-move (they are mated).
        return (-MATE_SCORE + 1, None);
    }
    if pos.is_stalemate() || pos.is_insufficient_material() {
        return (0, None);
    }
    if depth == 0 {
        return (evaluate_side_to_move(pos), None);
    }

    let moves = pos.legal_moves();
    if moves.is_empty() {
        // Should be covered by checkmate/stalemate, but keep safe.
        return (evaluate_side_to_move(pos), None);
    }

    let mut best_score = -MATE_SCORE;
    let mut best_move: Option<Move> = None;

    for m in moves {
        let snapshot = pos.clone();
        pos.play_unchecked(&m);
        let (child_score, _) = negamax(pos, depth - 1, -beta, -alpha, nodes);
        let score = -child_score;
        *pos = snapshot;

        if score > best_score {
            best_score = score;
            best_move = Some(m);
        }
        if score > alpha {
            alpha = score;
        }
        if alpha >= beta {
            break;
        }
    }

    // Mate distance: if we have a mating line, taper so shorter mates prefer.
    if best_score > MATE_SCORE - 1000 {
        best_score -= 1;
    } else if best_score < -MATE_SCORE + 1000 {
        best_score += 1;
    }

    (best_score, best_move)
}

/// Count nodes in a pure move-generation tree (perft-style) for throughput tests.
/// Each generated position counts as one node (including the root).
pub fn perft(pos: &Chess, depth: u8) -> u64 {
    perft_rec(&mut pos.clone(), depth)
}

fn perft_rec(pos: &mut Chess, depth: u8) -> u64 {
    if depth == 0 {
        return 1;
    }
    let moves = pos.legal_moves();
    if depth == 1 {
        return moves.len() as u64;
    }
    let mut nodes = 0_u64;
    for m in moves {
        let snapshot = pos.clone();
        pos.play_unchecked(&m);
        nodes += perft_rec(pos, depth - 1);
        *pos = snapshot;
    }
    nodes
}

#[cfg(test)]
mod tests {
    use super::*;
    use shakmaty::fen::Fen;

    #[test]
    fn perft_startpos_depth1() {
        let pos = Chess::default();
        assert_eq!(perft(&pos, 1), 20);
    }

    #[test]
    fn perft_startpos_depth2() {
        let pos = Chess::default();
        assert_eq!(perft(&pos, 2), 400);
    }

    #[test]
    fn search_returns_legal_move() {
        let pos = Chess::default();
        let res = negamax_search(&pos, 2);
        assert!(res.nodes > 0);
        assert!(res.best_move.is_some());
        assert!(pos.is_legal(res.best_move.as_ref().unwrap()));
    }

    #[test]
    fn search_finds_mate_in_one() {
        // Qxf7# (classic Q+B battery).
        let fen = "r1bqkb1r/pppp1ppp/2n2n2/4p2Q/2B1P3/8/PPPP1PPP/RNB1K1NR w KQkq - 4 4";
        let pos: Chess = fen
            .parse::<Fen>()
            .unwrap()
            .into_position(shakmaty::CastlingMode::Standard)
            .unwrap();
        let res = negamax_search(&pos, 1);
        assert!(res.best_move.is_some());
        let mut next = pos.clone();
        next.play_unchecked(res.best_move.as_ref().unwrap());
        assert!(next.is_checkmate(), "expected mate-in-one move");
    }
}
