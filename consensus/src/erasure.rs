//! Reed-Solomon erasure coding for regional SSD hub data availability.
//!
//! Splits append-only log blobs into `data_shards` + `parity_shards` so a hub
//! can recover the original payload after losing up to `parity_shards` shards.
//!
//! Uses the pure-Rust `reed-solomon-erasure` crate (GF(2^8)).

use reed_solomon_erasure::galois_8::ReedSolomon;
use reed_solomon_erasure::Error as RsError;

/// Default layout for regional hubs: 4 data + 2 parity (tolerate 2 lost shards).
pub const DEFAULT_DATA_SHARDS: usize = 4;
pub const DEFAULT_PARITY_SHARDS: usize = 2;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ErasureError {
    /// Invalid shard configuration.
    Config(String),
    /// Not enough shards remain to reconstruct.
    TooManyLost { lost: usize, parity: usize },
    /// Underlying RS library error.
    Rs(String),
    /// Empty payload.
    EmptyPayload,
}

impl std::fmt::Display for ErasureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ErasureError::Config(m) => write!(f, "config: {m}"),
            ErasureError::TooManyLost { lost, parity } => {
                write!(f, "too many lost shards ({lost} > parity {parity})")
            }
            ErasureError::Rs(m) => write!(f, "reed-solomon: {m}"),
            ErasureError::EmptyPayload => write!(f, "empty payload"),
        }
    }
}

impl std::error::Error for ErasureError {}

impl From<RsError> for ErasureError {
    fn from(e: RsError) -> Self {
        ErasureError::Rs(format!("{e:?}"))
    }
}

/// A single RS shard plus its index in the original layout.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shard {
    pub index: usize,
    pub data: Vec<u8>,
}

/// Encoded payload with padding metadata for exact recovery.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EncodedBlob {
    pub data_shards: usize,
    pub parity_shards: usize,
    /// Original unpadded payload length in bytes.
    pub original_len: usize,
    /// Equal-sized shards: first `data_shards` are data, rest are parity.
    pub shards: Vec<Vec<u8>>,
}

impl EncodedBlob {
    pub fn total_shards(&self) -> usize {
        self.data_shards + self.parity_shards
    }

    pub fn shard_len(&self) -> usize {
        self.shards.first().map(|s| s.len()).unwrap_or(0)
    }
}

/// Encode `payload` into data + parity shards.
pub fn encode(
    payload: &[u8],
    data_shards: usize,
    parity_shards: usize,
) -> Result<EncodedBlob, ErasureError> {
    if data_shards == 0 {
        return Err(ErasureError::Config("data_shards must be >= 1".into()));
    }
    if parity_shards == 0 {
        return Err(ErasureError::Config("parity_shards must be >= 1".into()));
    }
    if payload.is_empty() {
        return Err(ErasureError::EmptyPayload);
    }

    let rs = ReedSolomon::new(data_shards, parity_shards)?;
    let original_len = payload.len();

    // Pad to a multiple of data_shards so each data shard has equal length.
    let shard_len = (original_len + data_shards - 1) / data_shards;
    let padded_len = shard_len * data_shards;
    let mut padded = vec![0u8; padded_len];
    padded[..original_len].copy_from_slice(payload);

    let mut shards: Vec<Vec<u8>> = Vec::with_capacity(data_shards + parity_shards);
    for i in 0..data_shards {
        let start = i * shard_len;
        shards.push(padded[start..start + shard_len].to_vec());
    }
    for _ in 0..parity_shards {
        shards.push(vec![0u8; shard_len]);
    }

    {
        let mut refs: Vec<&mut [u8]> = shards.iter_mut().map(|s| s.as_mut_slice()).collect();
        rs.encode(&mut refs)?;
    }

    Ok(EncodedBlob {
        data_shards,
        parity_shards,
        original_len,
        shards,
    })
}

/// Encode with default 4+2 layout.
pub fn encode_default(payload: &[u8]) -> Result<EncodedBlob, ErasureError> {
    encode(payload, DEFAULT_DATA_SHARDS, DEFAULT_PARITY_SHARDS)
}

/// Reconstruct original payload from a partial set of shards.
///
/// `present` is a slice of length `data + parity` where missing shards are `None`.
pub fn recover(
    present: &[Option<Vec<u8>>],
    data_shards: usize,
    parity_shards: usize,
    original_len: usize,
) -> Result<Vec<u8>, ErasureError> {
    let total = data_shards + parity_shards;
    if present.len() != total {
        return Err(ErasureError::Config(format!(
            "expected {total} shard slots, got {}",
            present.len()
        )));
    }

    let lost = present.iter().filter(|s| s.is_none()).count();
    if lost > parity_shards {
        return Err(ErasureError::TooManyLost {
            lost,
            parity: parity_shards,
        });
    }

    // All present: just concatenate data shards.
    if lost == 0 {
        return concat_data(present, data_shards, original_len);
    }

    let rs = ReedSolomon::new(data_shards, parity_shards)?;
    let mut shards: Vec<Option<Vec<u8>>> = present.iter().map(|s| s.as_ref().cloned()).collect();
    rs.reconstruct(&mut shards)?;
    concat_data(&shards, data_shards, original_len)
}

/// Recover from an `EncodedBlob` with some shards removed.
pub fn recover_blob(blob: &EncodedBlob, lost_indices: &[usize]) -> Result<Vec<u8>, ErasureError> {
    let mut present: Vec<Option<Vec<u8>>> = blob.shards.iter().cloned().map(Some).collect();
    for &idx in lost_indices {
        if idx < present.len() {
            present[idx] = None;
        }
    }
    recover(
        &present,
        blob.data_shards,
        blob.parity_shards,
        blob.original_len,
    )
}

fn concat_data(
    present: &[Option<Vec<u8>>],
    data_shards: usize,
    original_len: usize,
) -> Result<Vec<u8>, ErasureError> {
    let mut out = Vec::with_capacity(original_len);
    for i in 0..data_shards {
        let shard = present[i]
            .as_ref()
            .ok_or_else(|| ErasureError::Config(format!("data shard {i} still missing")))?;
        out.extend_from_slice(shard);
    }
    out.truncate(original_len);
    Ok(out)
}

/// Convenience: drop specific shards from a blob (for tests / fault injection).
pub fn drop_shards(blob: &EncodedBlob, indices: &[usize]) -> Vec<Option<Vec<u8>>> {
    blob.shards
        .iter()
        .enumerate()
        .map(|(i, s)| {
            if indices.contains(&i) {
                None
            } else {
                Some(s.clone())
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rs_recover_from_lost_shard() {
        let payload = b"MossyMesh regional SSD hub append-only log block #42";
        let encoded = encode(payload, 4, 2).expect("encode");
        assert_eq!(encoded.shards.len(), 6);

        // Lose one data shard and one parity shard.
        let recovered = recover_blob(&encoded, &[1, 5]).expect("recover");
        assert_eq!(recovered, payload);
    }

    #[test]
    fn rs_recover_from_two_data_losses() {
        let payload = (0u8..100).collect::<Vec<_>>();
        let encoded = encode_default(&payload).expect("encode");
        let recovered = recover_blob(&encoded, &[0, 3]).expect("recover two data");
        assert_eq!(recovered, payload);
    }

    #[test]
    fn rs_fails_when_too_many_lost() {
        let payload = b"too many missing shards";
        let encoded = encode(payload, 3, 1).expect("encode");
        // parity=1 but lose 2 → fail
        let err = recover_blob(&encoded, &[0, 1]).unwrap_err();
        match err {
            ErasureError::TooManyLost { lost: 2, parity: 1 } => {}
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn rs_roundtrip_no_loss() {
        let payload = b"perfectly intact";
        let encoded = encode(payload, 2, 1).unwrap();
        let recovered = recover_blob(&encoded, &[]).unwrap();
        assert_eq!(recovered.as_slice(), payload);
    }

    #[test]
    fn empty_payload_rejected() {
        assert!(matches!(
            encode(b"", 2, 1),
            Err(ErasureError::EmptyPayload)
        ));
    }
}
