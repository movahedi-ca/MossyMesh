//! Engine Logic Module for MossyMesh
//!
//! Central chess application logic using shakmaty bitboards.
//! Designed for deterministic mesh PoC and wasm32-wasip1 readiness.
//!
//! DOC 36: The engine must run fully offline within the WAMR sandbox,
//! evaluating moves deterministically.
//!
//! # Modules
//! - [`eval`] — material + simple PST evaluation (no RNG)
//! - [`search`] — negamax/minimax with [`MAX_DEPTH`] and node counters
//! - [`tablebase`] — Syzygy hook (feature-gated / stub when tables absent)
//! - [`benchmark`] — measured Mnps over a fixed workload

use shakmaty::fen::Fen;
use shakmaty::{
    CastlingMode, Chess, Color, EnPassantMode, Move, MoveList, Position, Role, Setup, Square,
};

pub mod benchmark;
pub mod eval;
pub mod search;
pub mod tablebase;

pub use benchmark::{benchmark_mnps, benchmark_mnps_detailed, BenchmarkReport};
pub use eval::{evaluate_side_to_move, evaluate_white_perspective, MATE_SCORE};
pub use search::{negamax_search, perft, SearchResult};
pub use tablebase::{
    open_tablebase, FileBackedTablebase, StubTablebase, TablebaseProbe, TbWdl,
};

/// DOC 44 / DOC 43: Hard depth cap prevents infinite search loops (battery / WASM safety).
/// Practical callers may use much smaller depths; this is the absolute ceiling.
pub const MAX_DEPTH: u8 = 64;

/// Default search depth for convenience helpers (well below MAX_DEPTH).
pub const DEFAULT_SEARCH_DEPTH: u8 = 3;

/// Starting position FEN (standard chess).
pub const START_FEN: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";

/// Lightweight init hook (no global state; kept for API stability with sandbox/interop).
pub fn init_engine() {
    #[cfg(not(target_arch = "wasm32"))]
    {
        eprintln!("Engine: shakmaty bitboards ready (native). MAX_DEPTH={MAX_DEPTH}");
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = MAX_DEPTH;
    }
}

/// DOC 45: Strict enum mapping for stable cross-target layouts (ARM/x86/WASM).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum PieceType {
    Pawn = 1,
    Knight = 2,
    Bishop = 3,
    Rook = 4,
    Queen = 5,
    King = 6,
}

impl From<Role> for PieceType {
    fn from(role: Role) -> Self {
        match role {
            Role::Pawn => PieceType::Pawn,
            Role::Knight => PieceType::Knight,
            Role::Bishop => PieceType::Bishop,
            Role::Rook => PieceType::Rook,
            Role::Queen => PieceType::Queen,
            Role::King => PieceType::King,
        }
    }
}

/// Wrapper around a shakmaty [`Chess`] position with make/unmake stack support.
#[derive(Debug, Clone)]
pub struct EngineState {
    pub position: Chess,
    /// History stack for unmake (DOC 43: cheaper than unbounded clone-only recursion at UI layer).
    history: Vec<Chess>,
}

impl Default for EngineState {
    fn default() -> Self {
        Self::new()
    }
}

impl EngineState {
    pub fn new() -> Self {
        EngineState {
            position: Chess::default(),
            history: Vec::new(),
        }
    }

    /// Load position from FEN string.
    pub fn from_fen(fen_str: &str) -> Result<Self, String> {
        let fen: Fen = fen_str
            .parse()
            .map_err(|e| format!("Invalid FEN: {:?}", e))?;
        let position = fen
            .into_position(CastlingMode::Standard)
            .map_err(|e| format!("Invalid Position: {:?}", e))?;
        Ok(EngineState {
            position,
            history: Vec::new(),
        })
    }

    /// Export current position as FEN.
    pub fn to_fen(&self) -> String {
        Fen::from_position(self.position.clone(), EnPassantMode::Legal).to_string()
    }

    /// Setup dump (structural equality helper).
    pub fn to_setup(&self) -> Setup {
        self.position.clone().into_setup(EnPassantMode::Legal)
    }

    /// Legal move generation.
    pub fn get_moves(&self) -> MoveList {
        self.position.legal_moves()
    }

    /// Number of legal moves.
    pub fn legal_move_count(&self) -> usize {
        self.get_moves().len()
    }

    /// Play a legal move, pushing prior state for [`Self::unmake_move`].
    pub fn make_move(&mut self, m: &Move) -> Result<(), String> {
        if !self.position.is_legal(m) {
            return Err("Illegal move".to_string());
        }
        self.history.push(self.position.clone());
        self.position.play_unchecked(m);
        Ok(())
    }

    /// Unmake last move from the history stack.
    pub fn unmake_move(&mut self) -> Result<(), String> {
        let prev = self
            .history
            .pop()
            .ok_or_else(|| "No move to unmake".to_string())?;
        self.position = prev;
        Ok(())
    }

    /// Clear make/unmake history (keeps current position).
    pub fn clear_history(&mut self) {
        self.history.clear();
    }

    /// Side to move.
    pub fn turn(&self) -> Color {
        self.position.turn()
    }

    /// Whether the current side is in check.
    pub fn is_check(&self) -> bool {
        self.position.is_check()
    }

    /// Game-over detection (mate / stalemate / insufficient material / variant end).
    pub fn is_game_over(&self) -> bool {
        self.position.is_game_over()
    }

    /// Material + positional evaluation from White's perspective (centipawns).
    pub fn evaluate_position(&self) -> i32 {
        evaluate_white_perspective(&self.position)
    }

    /// Evaluation from side-to-move (negamax convention).
    pub fn evaluate_stm(&self) -> i32 {
        evaluate_side_to_move(&self.position)
    }

    /// Bounded negamax search from the current position.
    pub fn search(&self, depth: u8) -> SearchResult {
        negamax_search(&self.position, depth)
    }

    /// Optional tablebase probe (always `None` with stub / missing tables).
    pub fn probe_tablebase(&self, tb: &dyn TablebaseProbe) -> Option<TbWdl> {
        tb.probe_wdl(&self.position)
    }

    /// Piece at square, if any, as [`PieceType`] + color.
    pub fn piece_at(&self, sq: Square) -> Option<(PieceType, Color)> {
        self.position
            .board()
            .piece_at(sq)
            .map(|p| (PieceType::from(p.role), p.color))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shakmaty::{Role, Square};

    #[test]
    fn test_initial_position_moves() {
        let engine = EngineState::new();
        let moves = engine.get_moves();
        assert_eq!(moves.len(), 20); // 20 legal moves from starting position
    }

    #[test]
    fn fen_roundtrip_startpos() {
        let engine = EngineState::new();
        let fen = engine.to_fen();
        let loaded = EngineState::from_fen(&fen).expect("reload fen");
        assert_eq!(loaded.legal_move_count(), 20);
        assert_eq!(loaded.evaluate_position(), engine.evaluate_position());
    }

    #[test]
    fn from_fen_start_constant() {
        let e = EngineState::from_fen(START_FEN).unwrap();
        assert_eq!(e.legal_move_count(), 20);
        assert_eq!(e.turn(), Color::White);
    }

    #[test]
    fn make_unmake_restores_position() {
        let mut engine = EngineState::new();
        let fen_before = engine.to_fen();
        let moves = engine.get_moves();
        let m = moves[0].clone();
        engine.make_move(&m).unwrap();
        assert_ne!(engine.to_fen(), fen_before);
        engine.unmake_move().unwrap();
        assert_eq!(engine.to_fen(), fen_before);
        assert_eq!(engine.legal_move_count(), 20);
    }

    #[test]
    fn play_sequence_e4_e5() {
        let mut engine = EngineState::new();
        let e4 = moves_matching(&engine, Square::E2, Square::E4);
        engine.make_move(&e4).unwrap();
        let e5 = moves_matching(&engine, Square::E7, Square::E5);
        engine.make_move(&e5).unwrap();
        assert_eq!(engine.turn(), Color::White);
        assert!(engine.legal_move_count() > 0);
        engine.unmake_move().unwrap();
        engine.unmake_move().unwrap();
        assert_eq!(engine.legal_move_count(), 20);
    }

    #[test]
    fn eval_stability_startpos() {
        let engine = EngineState::new();
        let a = engine.evaluate_position();
        let b = engine.evaluate_position();
        assert_eq!(a, b);
        // Equal material + white tempo
        assert_eq!(a, 10);
    }

    #[test]
    fn eval_material_advantage_white_queen_up() {
        let fen = "4k3/8/8/8/8/8/8/4KQ2 w - - 0 1";
        let engine = EngineState::from_fen(fen).unwrap();
        let score = engine.evaluate_position();
        assert!(
            score > 800,
            "white queen-up should be strongly positive, got {score}"
        );
    }

    #[test]
    fn search_depth_clamped_and_nodes_increase() {
        let engine = EngineState::new();
        let d1 = engine.search(1);
        let d2 = engine.search(2);
        assert!(d2.nodes >= d1.nodes);
        assert!(d1.best_move.is_some());
        assert_eq!(d1.depth, 1);
    }

    #[test]
    fn piece_type_mapping() {
        assert_eq!(PieceType::from(Role::Queen), PieceType::Queen);
        assert_eq!(PieceType::King as u8, 6);
    }

    #[test]
    fn invalid_fen_errors() {
        assert!(EngineState::from_fen("not-a-fen").is_err());
    }

    #[test]
    fn tablebase_stub_integration() {
        let engine = EngineState::new();
        let tb = StubTablebase::new();
        assert!(engine.probe_tablebase(&tb).is_none());
    }

    #[test]
    fn benchmark_mnps_is_measured() {
        let mnps = benchmark_mnps();
        assert!(mnps.is_finite());
        assert!(mnps > 0.0);
    }

    fn moves_matching(engine: &EngineState, from: Square, to: Square) -> Move {
        engine
            .get_moves()
            .into_iter()
            .find(|m| m.from() == Some(from) && m.to() == to)
            .unwrap_or_else(|| panic!("no move {from:?}->{to:?}"))
    }
}
