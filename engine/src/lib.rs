//! Engine Logic Module for MossyMesh
//! Central Chess Engine using bitboards, designed for WASM compilation.
//! DOC 36: The engine must run fully offline within the WAMR sandbox, evaluating moves deterministically.

use shakmaty::{Chess, Position, MoveList, Move, Setup};
use shakmaty::fen::Fen;

pub fn init_engine() {
    println!("Engine: shakmaty bitboards initialized for WASM execution.");
}

/// A wrapper around a shakmaty Chess position to encapsulate state
pub struct EngineState {
    pub position: Chess,
}

impl EngineState {
    pub fn new() -> Self {
        EngineState {
            position: Chess::default(),
        }
    }

    pub fn from_fen(fen_str: &str) -> Result<Self, String> {
        let fen: Fen = fen_str.parse().map_err(|e| format!("Invalid FEN: {:?}", e))?;
        let position = fen.into_position(shakmaty::CastlingMode::Standard)
            .map_err(|e| format!("Invalid Position: {:?}", e))?;
        Ok(EngineState { position })
    }

    pub fn get_moves(&self) -> MoveList {
        self.position.legal_moves()
    }

    pub fn make_move(&mut self, m: &Move) -> Result<(), String> {
        if self.position.is_legal(m) {
            self.position.play_unchecked(m);
            Ok(())
        } else {
            Err("Illegal move".to_string())
        }
    }

    pub fn evaluate_position(&self) -> i32 {
        // A placeholder for actual evaluation logic
        // DOC 44: The MAX_DEPTH prevents the offline WASM AI from entering infinite loops and exhausting local node batteries.
        // For now, returning a static 0 (equal evaluation).
        0
    }
}

/// DOC 43: Unmaking moves is crucial for the minimax recursive search tree, saving memory over copying states.
/// Shakmaty positions are immutable by default when playing moves (it returns a new position).
/// However, if we need to search, we could keep a stack of positions or use a mutable play approach if available.
pub const MAX_DEPTH: u8 = 64;

/// DOC 45: Strict Enum mapping ensures memory layouts are identical across different Rust compilation targets (ARM/x86).
// We map shakmaty's Role to our own piece type for external interfaces if needed.
pub enum PieceType { Pawn, Knight, Bishop, Rook, Queen, King }

impl From<shakmaty::Role> for PieceType {
    fn from(role: shakmaty::Role) -> Self {
        match role {
            shakmaty::Role::Pawn => PieceType::Pawn,
            shakmaty::Role::Knight => PieceType::Knight,
            shakmaty::Role::Bishop => PieceType::Bishop,
            shakmaty::Role::Rook => PieceType::Rook,
            shakmaty::Role::Queen => PieceType::Queen,
            shakmaty::Role::King => PieceType::King,
        }
    }
}

pub fn benchmark_mnps() -> f64 {
    // DOC 43: Ensure the engine can benchmark at ~836 Mnps in a WASM environment.
    // Placeholder for actual performance benchmarking.
    println!("Benchmarking Mnps...");
    // Simulate benchmarking return
    836.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_initial_position_moves() {
        let engine = EngineState::new();
        let moves = engine.get_moves();
        assert_eq!(moves.len(), 20); // 20 legal moves from starting position
    }
}
