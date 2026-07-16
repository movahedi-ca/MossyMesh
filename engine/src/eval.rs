//! Deterministic material + simple positional evaluation (no RNG).
//! Scores are in centipawns from White's perspective unless noted.

use shakmaty::{Color, Piece, Position, Role, Square};

/// Material values in centipawns.
pub const PAWN_VALUE: i32 = 100;
pub const KNIGHT_VALUE: i32 = 320;
pub const BISHOP_VALUE: i32 = 330;
pub const ROOK_VALUE: i32 = 500;
pub const QUEEN_VALUE: i32 = 900;
pub const KING_VALUE: i32 = 20_000;

/// Absolute mate score (centipawns). Search uses mate distance tapering.
pub const MATE_SCORE: i32 = 30_000;

/// Piece-square tables indexed by square index (A1=0 … H8=63), White's POV.
/// Black uses vertically flipped squares.
const PAWN_PST: [i32; 64] = [
    0, 0, 0, 0, 0, 0, 0, 0, // rank 1
    5, 10, 10, -20, -20, 10, 10, 5, // rank 2
    5, -5, -10, 0, 0, -10, -5, 5, // rank 3
    0, 0, 0, 20, 20, 0, 0, 0, // rank 4
    5, 5, 10, 25, 25, 10, 5, 5, // rank 5
    10, 10, 20, 30, 30, 20, 10, 10, // rank 6
    50, 50, 50, 50, 50, 50, 50, 50, // rank 7
    0, 0, 0, 0, 0, 0, 0, 0, // rank 8
];

const KNIGHT_PST: [i32; 64] = [
    -50, -40, -30, -30, -30, -30, -40, -50, //
    -40, -20, 0, 0, 0, 0, -20, -40, //
    -30, 0, 10, 15, 15, 10, 0, -30, //
    -30, 5, 15, 20, 20, 15, 5, -30, //
    -30, 0, 15, 20, 20, 15, 0, -30, //
    -30, 5, 10, 15, 15, 10, 5, -30, //
    -40, -20, 0, 5, 5, 0, -20, -40, //
    -50, -40, -30, -30, -30, -30, -40, -50, //
];

const BISHOP_PST: [i32; 64] = [
    -20, -10, -10, -10, -10, -10, -10, -20, //
    -10, 5, 0, 0, 0, 0, 5, -10, //
    -10, 10, 10, 10, 10, 10, 10, -10, //
    -10, 0, 10, 10, 10, 10, 0, -10, //
    -10, 5, 5, 10, 10, 5, 5, -10, //
    -10, 0, 5, 10, 10, 5, 0, -10, //
    -10, 0, 0, 0, 0, 0, 0, -10, //
    -20, -10, -10, -10, -10, -10, -10, -20, //
];

const ROOK_PST: [i32; 64] = [
    0, 0, 0, 5, 5, 0, 0, 0, //
    -5, 0, 0, 0, 0, 0, 0, -5, //
    -5, 0, 0, 0, 0, 0, 0, -5, //
    -5, 0, 0, 0, 0, 0, 0, -5, //
    -5, 0, 0, 0, 0, 0, 0, -5, //
    -5, 0, 0, 0, 0, 0, 0, -5, //
    5, 10, 10, 10, 10, 10, 10, 5, //
    0, 0, 0, 0, 0, 0, 0, 0, //
];

const QUEEN_PST: [i32; 64] = [
    -20, -10, -10, -5, -5, -10, -10, -20, //
    -10, 0, 0, 0, 0, 0, 0, -10, //
    -10, 0, 5, 5, 5, 5, 0, -10, //
    -5, 0, 5, 5, 5, 5, 0, -5, //
    0, 0, 5, 5, 5, 5, 0, -5, //
    -10, 5, 5, 5, 5, 5, 0, -10, //
    -10, 0, 5, 0, 0, 0, 0, -10, //
    -20, -10, -10, -5, -5, -10, -10, -20, //
];

const KING_MID_PST: [i32; 64] = [
    20, 30, 10, 0, 0, 10, 30, 20, //
    20, 20, 0, 0, 0, 0, 20, 20, //
    -10, -20, -20, -20, -20, -20, -20, -10, //
    -20, -30, -30, -40, -40, -30, -30, -20, //
    -30, -40, -40, -50, -50, -40, -40, -30, //
    -30, -40, -40, -50, -50, -40, -40, -30, //
    -30, -40, -40, -50, -50, -40, -40, -30, //
    -30, -40, -40, -50, -50, -40, -40, -30, //
];

#[inline]
fn material_value(role: Role) -> i32 {
    match role {
        Role::Pawn => PAWN_VALUE,
        Role::Knight => KNIGHT_VALUE,
        Role::Bishop => BISHOP_VALUE,
        Role::Rook => ROOK_VALUE,
        Role::Queen => QUEEN_VALUE,
        Role::King => KING_VALUE,
    }
}

#[inline]
fn pst(role: Role, sq: Square) -> i32 {
    let idx = u32::from(sq) as usize;
    match role {
        Role::Pawn => PAWN_PST[idx],
        Role::Knight => KNIGHT_PST[idx],
        Role::Bishop => BISHOP_PST[idx],
        Role::Rook => ROOK_PST[idx],
        Role::Queen => QUEEN_PST[idx],
        Role::King => KING_MID_PST[idx],
    }
}

/// Evaluate `position` in centipawns from White's perspective.
/// Positive = White is better. Fully deterministic (no RNG, no clocks).
pub fn evaluate_white_perspective<P: Position>(position: &P) -> i32 {
    let board = position.board();
    let mut score = 0_i32;

    for sq in board.occupied() {
        if let Some(piece) = board.piece_at(sq) {
            score += piece_score(piece, sq);
        }
    }

    // Tempo: small bonus for side to move so equal positions aren't sticky draws
    // in shallow search (deterministic constant).
    match position.turn() {
        Color::White => score += 10,
        Color::Black => score -= 10,
    }

    score
}

#[inline]
fn piece_score(piece: Piece, sq: Square) -> i32 {
    let (mat, pst_sq) = match piece.color {
        Color::White => (material_value(piece.role), sq),
        Color::Black => (material_value(piece.role), sq.flip_vertical()),
    };
    let positional = pst(piece.role, pst_sq);
    match piece.color {
        Color::White => mat + positional,
        Color::Black => -(mat + positional),
    }
}

/// Evaluate from the side to move (negamax convention).
pub fn evaluate_side_to_move<P: Position>(position: &P) -> i32 {
    let white = evaluate_white_perspective(position);
    match position.turn() {
        Color::White => white,
        Color::Black => -white,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shakmaty::Chess;

    #[test]
    fn startpos_near_equal() {
        let pos = Chess::default();
        let s = evaluate_white_perspective(&pos);
        // Tempo +10 for white; material equal.
        assert_eq!(s, 10);
    }

    #[test]
    fn eval_stable_repeated() {
        let pos = Chess::default();
        let a = evaluate_white_perspective(&pos);
        let b = evaluate_white_perspective(&pos);
        assert_eq!(a, b);
    }
}
