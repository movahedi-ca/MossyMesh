//! Compact binary FEN codec for LoRa transmission (issue #20).
//!
//! A raw FEN string is 60-90 bytes of ASCII; with frame headers it risks
//! spilling into extra LoRa fragments on constrained links. This codec packs
//! any legal position into exactly [`COMPRESSED_FEN_LEN`] bytes:
//!
//! ```text
//! bytes  0..32  board: 64 squares x 4-bit piece nibbles
//! byte      32   flags: bit 0 = side to move (1 = black), bits 1-4 = KQkq
//! byte      33   en-passant file (0-7), or 0xFF when none
//! bytes  34..36 halfmove clock, u16 little-endian
//! bytes  36..38 fullmove number, u16 little-endian
//! ```
//!
//! Square `i = rank * 8 + file` with rank 0 as White's back rank. Piece
//! nibbles: 0 = empty, 1-6 = PNBRQK, 7-12 = pnbrqk. The en-passant rank is
//! implied by the side to move, so only the file is stored.

/// Compressed size of any encoded position: 38 bytes, well inside one
/// LoRa frame payload.
pub const COMPRESSED_FEN_LEN: usize = 38;

/// En-passant file value meaning "no en-passant square".
const NO_EP: u8 = 0xFF;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FenError {
    BadFieldCount,
    BadRank,
    BadPiece(char),
    BadSide,
    BadCastling,
    BadEnPassant,
    BadClock,
    BadLength(usize),
    BadNibble(u8),
    BadFlags,
}

fn piece_to_nibble(c: char) -> Result<u8, FenError> {
    Ok(match c {
        'P' => 1,
        'N' => 2,
        'B' => 3,
        'R' => 4,
        'Q' => 5,
        'K' => 6,
        'p' => 7,
        'n' => 8,
        'b' => 9,
        'r' => 10,
        'q' => 11,
        'k' => 12,
        _ => return Err(FenError::BadPiece(c)),
    })
}

fn nibble_to_piece(n: u8) -> Result<char, FenError> {
    Ok(match n {
        0 => ' ',
        1 => 'P',
        2 => 'N',
        3 => 'B',
        4 => 'R',
        5 => 'Q',
        6 => 'K',
        7 => 'p',
        8 => 'n',
        9 => 'b',
        10 => 'r',
        11 => 'q',
        12 => 'k',
        _ => return Err(FenError::BadNibble(n)),
    })
}

/// Encode a FEN string into 38 compact bytes for LoRa transmission.
pub fn encode_fen(fen: &str) -> Result<[u8; COMPRESSED_FEN_LEN], FenError> {
    let parts: Vec<&str> = fen.split_whitespace().collect();
    if parts.len() != 6 {
        return Err(FenError::BadFieldCount);
    }

    let mut out = [0u8; COMPRESSED_FEN_LEN];

    // Board: FEN lists rank 8 first; our rank 0 is White's back rank.
    let ranks: Vec<&str> = parts[0].split('/').collect();
    if ranks.len() != 8 {
        return Err(FenError::BadRank);
    }
    for (ri, rank) in ranks.iter().enumerate() {
        let rank_idx = 7 - ri;
        let mut file = 0usize;
        for c in rank.chars() {
            if let Some(d) = c.to_digit(10) {
                if d == 0 || d > 8 {
                    return Err(FenError::BadRank);
                }
                file += d as usize;
            } else {
                if file >= 8 {
                    return Err(FenError::BadRank);
                }
                let nib = piece_to_nibble(c)?;
                let sq = rank_idx * 8 + file;
                out[sq / 2] |= nib << ((sq % 2) * 4);
                file += 1;
            }
        }
        if file != 8 {
            return Err(FenError::BadRank);
        }
    }

    // Side to move.
    let black_to_move = match parts[1] {
        "w" => false,
        "b" => true,
        _ => return Err(FenError::BadSide),
    };
    let mut flags: u8 = if black_to_move { 0x01 } else { 0x00 };

    // Castling rights.
    if parts[2] != "-" {
        for c in parts[2].chars() {
            flags |= match c {
                'K' => 0x02,
                'Q' => 0x04,
                'k' => 0x08,
                'q' => 0x10,
                _ => return Err(FenError::BadCastling),
            };
        }
    }
    out[32] = flags;

    // En passant: file only; the rank is implied by the side to move.
    out[33] = if parts[3] == "-" {
        NO_EP
    } else {
        let b = parts[3].as_bytes();
        if b.len() != 2 || !(b'a'..=b'h').contains(&b[0]) || !(b'1'..=b'8').contains(&b[1]) {
            return Err(FenError::BadEnPassant);
        }
        b[0] - b'a'
    };

    // Clocks.
    let half: u16 = parts[4].parse().map_err(|_| FenError::BadClock)?;
    let full: u16 = parts[5].parse().map_err(|_| FenError::BadClock)?;
    out[34..36].copy_from_slice(&half.to_le_bytes());
    out[36..38].copy_from_slice(&full.to_le_bytes());

    Ok(out)
}

/// Decode 38 compact bytes back into a FEN string.
pub fn decode_fen(bytes: &[u8]) -> Result<String, FenError> {
    if bytes.len() != COMPRESSED_FEN_LEN {
        return Err(FenError::BadLength(bytes.len()));
    }
    let mut fen = String::with_capacity(90);

    // Board, emitted in FEN order (rank 8 down to rank 1).
    for rank_idx in (0..8).rev() {
        let mut empty = 0u32;
        for file in 0..8 {
            let sq = rank_idx * 8 + file;
            let nib = (bytes[sq / 2] >> ((sq % 2) * 4)) & 0x0F;
            let piece = nibble_to_piece(nib)?;
            if piece == ' ' {
                empty += 1;
            } else {
                if empty > 0 {
                    fen.push(char::from_digit(empty, 10).unwrap());
                    empty = 0;
                }
                fen.push(piece);
            }
        }
        if empty > 0 {
            fen.push(char::from_digit(empty, 10).unwrap());
        }
        if rank_idx > 0 {
            fen.push('/');
        }
    }

    let flags = bytes[32];
    if flags & 0xE0 != 0 {
        return Err(FenError::BadFlags);
    }
    fen.push(' ');
    fen.push(if flags & 0x01 != 0 { 'b' } else { 'w' });
    fen.push(' ');
    let mut castle = String::new();
    if flags & 0x02 != 0 {
        castle.push('K');
    }
    if flags & 0x04 != 0 {
        castle.push('Q');
    }
    if flags & 0x08 != 0 {
        castle.push('k');
    }
    if flags & 0x10 != 0 {
        castle.push('q');
    }
    if castle.is_empty() {
        castle.push('-');
    }
    fen.push_str(&castle);
    fen.push(' ');

    if bytes[33] == NO_EP {
        fen.push('-');
    } else {
        if bytes[33] > 7 {
            return Err(FenError::BadEnPassant);
        }
        // White to move means Black just double-pushed: ep square is on rank 6.
        let rank = if flags & 0x01 != 0 { '3' } else { '6' };
        fen.push((b'a' + bytes[33]) as char);
        fen.push(rank);
    }

    let half = u16::from_le_bytes([bytes[34], bytes[35]]);
    let full = u16::from_le_bytes([bytes[36], bytes[37]]);
    fen.push_str(&format!(" {half} {full}"));
    Ok(fen)
}

/// The compressed form always fits in a single LoRa frame payload.
pub fn fits_single_lora_frame() -> bool {
    COMPRESSED_FEN_LEN <= crate::lora_mac::LORA_MAX_PAYLOAD
}

#[cfg(test)]
mod tests {
    use super::*;

    const STARTPOS: &str =
        "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";

    #[test]
    fn startpos_roundtrip() {
        let enc = encode_fen(STARTPOS).expect("encode startpos");
        assert_eq!(decode_fen(&enc).expect("decode"), STARTPOS);
    }

    #[test]
    fn complex_position_roundtrip() {
        for fen in [
            "r1bqk2r/pppp1ppp/2n2n2/2b1p3/2B1P3/5N2/PPPP1PPP/RNBQK2R w KQkq - 4 4",
            "rnbqkbnr/ppp1pppp/8/3p4/4P3/8/PPPP1PPP/RNBQKBNR w KQkq d6 0 2",
            "8/8/4k3/8/8/4K3/8/8 b - - 12 60",
            "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
        ] {
            let enc = encode_fen(fen).expect("encode");
            assert_eq!(decode_fen(&enc).expect("decode"), fen, "roundtrip: {fen}");
        }
    }

    #[test]
    fn compressed_size_fits_lora() {
        assert_eq!(COMPRESSED_FEN_LEN, 38);
        assert!(COMPRESSED_FEN_LEN < STARTPOS.len());
        assert!(fits_single_lora_frame());
    }

    #[test]
    fn invalid_fen_rejected() {
        assert_eq!(
            encode_fen("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0"),
            Err(FenError::BadFieldCount)
        );
        assert!(matches!(
            encode_fen("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP w KQkq - 0 1"),
            Err(FenError::BadRank)
        ));
        assert_eq!(
            encode_fen("rnbqkXnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1"),
            Err(FenError::BadPiece('X'))
        );
        assert_eq!(
            encode_fen("8/8/8/8/8/8/8/8 x - - 0 1"),
            Err(FenError::BadSide)
        );
        assert_eq!(
            decode_fen(&[0u8; 10]),
            Err(FenError::BadLength(10))
        );
    }
}
