//! # SITF — Simple Intermediate Tensor Format
//!
//! Compact, endian-aware on-wire tensor blob for edge AI transfer and
//! deterministic replay. Fully offline; no external dependencies.
//!
//! ## On-wire layout (little-endian)
//!
//! ```text
//! magic:      4 bytes  b"SITF"
//! version:    u16      (currently [`SITF_VERSION`] = 1)
//! dtype:      u8       (0 = INT8, 1 = FP32)
//! rank:       u8       (number of dimensions, 0..=[`MAX_RANK`])
//! shape:      rank × u32
//! nbytes:     u64      (payload length in bytes)
//! data:       nbytes bytes (row-major, little-endian elements)
//! ```
//!
//! ## Public API
//!
//! | Construct | Wire | Inspect |
//! | --- | --- | --- |
//! | [`SitfTensor::new`], [`zeros`](SitfTensor::zeros), [`from_f32`](SitfTensor::from_f32), [`from_i8`](SitfTensor::from_i8) | [`to_bytes`](SitfTensor::to_bytes) / [`from_bytes`](SitfTensor::from_bytes) | [`numel`](SitfTensor::numel), [`as_f32_vec`](SitfTensor::as_f32_vec), [`as_i8_slice`](SitfTensor::as_i8_slice) |
//!
//! Encoding the same tensor twice yields **identical** byte sequences (deterministic).

use std::fmt;

/// Supported element types for SITF tensors.
///
/// Tags are stable on the wire (`as_u8` / `from_u8`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum DType {
    /// Symmetric / static INT8 quantization path (1 byte per element).
    Int8 = 0,
    /// IEEE-754 binary32, little-endian on the wire (4 bytes per element).
    Fp32 = 1,
}

impl DType {
    /// Parse a wire dtype tag. Returns `None` for unknown tags.
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(DType::Int8),
            1 => Some(DType::Fp32),
            _ => None,
        }
    }

    /// Size in bytes of one scalar element.
    pub fn element_size(self) -> usize {
        match self {
            DType::Int8 => 1,
            DType::Fp32 => 4,
        }
    }

    /// Wire tag for this dtype.
    pub fn as_u8(self) -> u8 {
        self as u8
    }
}

/// Maximum rank supported by the format (keeps header fixed-width friendly).
pub const MAX_RANK: usize = 8;

/// Format version written by this crate.
pub const SITF_VERSION: u16 = 1;

/// Magic bytes identifying a SITF blob (`b"SITF"`).
pub const SITF_MAGIC: &[u8; 4] = b"SITF";

/// Errors produced while encoding or decoding SITF tensors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SitfError {
    /// Buffer does not start with [`SITF_MAGIC`].
    BadMagic,
    /// Header version is not [`SITF_VERSION`].
    UnsupportedVersion(u16),
    /// Dtype tag is not a known [`DType`].
    UnknownDType(u8),
    /// Rank exceeds [`MAX_RANK`].
    RankTooLarge(usize),
    /// Payload length does not match `∏shape × element_size`.
    ShapeMismatch { expected_elems: usize, data_len: usize },
    /// Buffer shorter than header + claimed payload.
    Truncated { needed: usize, got: usize },
    /// Shape product or length arithmetic overflowed.
    Overflow,
}

impl fmt::Display for SitfError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SitfError::BadMagic => write!(f, "invalid SITF magic"),
            SitfError::UnsupportedVersion(v) => write!(f, "unsupported SITF version {v}"),
            SitfError::UnknownDType(d) => write!(f, "unknown dtype tag {d}"),
            SitfError::RankTooLarge(r) => write!(f, "rank {r} exceeds MAX_RANK={MAX_RANK}"),
            SitfError::ShapeMismatch {
                expected_elems,
                data_len,
            } => write!(
                f,
                "data length {data_len} does not match shape element count {expected_elems}"
            ),
            SitfError::Truncated { needed, got } => {
                write!(f, "truncated buffer: need {needed} bytes, got {got}")
            }
            SitfError::Overflow => write!(f, "shape product overflowed"),
        }
    }
}

impl std::error::Error for SitfError {}

/// Owned SITF tensor: shape + dtype + contiguous row-major payload.
///
/// Invariants (enforced by constructors):
/// - `shape.len() ≤ MAX_RANK`
/// - `data.len() == element_count(shape) * dtype.element_size()`
///
/// Fields are public for inspection and mesh hand-off; prefer constructors
/// over manual field assembly so size checks stay consistent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SitfTensor {
    /// Dimension sizes (row-major; empty shape = one scalar element).
    pub shape: Vec<u32>,
    /// Element type (INT8 or FP32).
    pub dtype: DType,
    /// Raw payload bytes (LE elements for FP32).
    pub data: Vec<u8>,
}

impl SitfTensor {
    /// Build a tensor from shape, dtype, and raw bytes. Validates size consistency.
    pub fn new(shape: Vec<u32>, dtype: DType, data: Vec<u8>) -> Result<Self, SitfError> {
        if shape.len() > MAX_RANK {
            return Err(SitfError::RankTooLarge(shape.len()));
        }
        let elems = element_count(&shape)?;
        let expected = elems
            .checked_mul(dtype.element_size())
            .ok_or(SitfError::Overflow)?;
        if data.len() != expected {
            return Err(SitfError::ShapeMismatch {
                expected_elems: elems,
                data_len: data.len(),
            });
        }
        Ok(Self { shape, dtype, data })
    }

    /// Zero-filled tensor with the given shape and dtype.
    pub fn zeros(shape: Vec<u32>, dtype: DType) -> Result<Self, SitfError> {
        let elems = element_count(&shape)?;
        let nbytes = elems
            .checked_mul(dtype.element_size())
            .ok_or(SitfError::Overflow)?;
        Self::new(shape, dtype, vec![0u8; nbytes])
    }

    /// Number of scalar elements (`∏ shape`, or `1` for rank-0).
    pub fn numel(&self) -> usize {
        element_count(&self.shape).unwrap_or(0)
    }

    /// Serialize to the SITF binary layout (little-endian).
    ///
    /// Deterministic: same tensor fields always produce the same byte vector.
    pub fn to_bytes(&self) -> Result<Vec<u8>, SitfError> {
        if self.shape.len() > MAX_RANK {
            return Err(SitfError::RankTooLarge(self.shape.len()));
        }
        let elems = element_count(&self.shape)?;
        let expected = elems
            .checked_mul(self.dtype.element_size())
            .ok_or(SitfError::Overflow)?;
        if self.data.len() != expected {
            return Err(SitfError::ShapeMismatch {
                expected_elems: elems,
                data_len: self.data.len(),
            });
        }

        // header: magic(4) + ver(2) + dtype(1) + rank(1) + shape(rank*4) + nbytes(8)
        let header_len = 4 + 2 + 1 + 1 + self.shape.len() * 4 + 8;
        let mut out = Vec::with_capacity(header_len + self.data.len());
        out.extend_from_slice(SITF_MAGIC);
        out.extend_from_slice(&SITF_VERSION.to_le_bytes());
        out.push(self.dtype.as_u8());
        out.push(self.shape.len() as u8);
        for &d in &self.shape {
            out.extend_from_slice(&d.to_le_bytes());
        }
        out.extend_from_slice(&(self.data.len() as u64).to_le_bytes());
        out.extend_from_slice(&self.data);
        Ok(out)
    }

    /// Parse a SITF binary blob.
    pub fn from_bytes(buf: &[u8]) -> Result<Self, SitfError> {
        // Minimum header without shape: 4+2+1+1+8 = 16
        if buf.len() < 16 {
            return Err(SitfError::Truncated {
                needed: 16,
                got: buf.len(),
            });
        }
        if &buf[0..4] != SITF_MAGIC.as_slice() {
            return Err(SitfError::BadMagic);
        }
        let version = u16::from_le_bytes([buf[4], buf[5]]);
        if version != SITF_VERSION {
            return Err(SitfError::UnsupportedVersion(version));
        }
        let dtype = DType::from_u8(buf[6]).ok_or(SitfError::UnknownDType(buf[6]))?;
        let rank = buf[7] as usize;
        if rank > MAX_RANK {
            return Err(SitfError::RankTooLarge(rank));
        }
        let shape_end = 8 + rank * 4;
        let nbytes_end = shape_end + 8;
        if buf.len() < nbytes_end {
            return Err(SitfError::Truncated {
                needed: nbytes_end,
                got: buf.len(),
            });
        }
        let mut shape = Vec::with_capacity(rank);
        for i in 0..rank {
            let off = 8 + i * 4;
            shape.push(u32::from_le_bytes([
                buf[off],
                buf[off + 1],
                buf[off + 2],
                buf[off + 3],
            ]));
        }
        let nbytes = u64::from_le_bytes(buf[shape_end..nbytes_end].try_into().unwrap()) as usize;
        let total = nbytes_end
            .checked_add(nbytes)
            .ok_or(SitfError::Overflow)?;
        if buf.len() < total {
            return Err(SitfError::Truncated {
                needed: total,
                got: buf.len(),
            });
        }
        let data = buf[nbytes_end..total].to_vec();
        Self::new(shape, dtype, data)
    }

    /// View payload as `i8` when dtype is INT8.
    pub fn as_i8_slice(&self) -> Option<&[i8]> {
        if self.dtype != DType::Int8 {
            return None;
        }
        // SAFETY: i8 and u8 have identical layout; we only re-interpret signedness.
        Some(bytemuck_i8(&self.data))
    }

    /// Decode FP32 payload into owned `f32` values (little-endian).
    pub fn as_f32_vec(&self) -> Option<Vec<f32>> {
        if self.dtype != DType::Fp32 {
            return None;
        }
        if self.data.len() % 4 != 0 {
            return None;
        }
        Some(
            self.data
                .chunks_exact(4)
                .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                .collect(),
        )
    }

    /// Build an FP32 tensor from `f32` values (encoded little-endian).
    pub fn from_f32(shape: Vec<u32>, values: &[f32]) -> Result<Self, SitfError> {
        let elems = element_count(&shape)?;
        if values.len() != elems {
            return Err(SitfError::ShapeMismatch {
                expected_elems: elems,
                data_len: values.len() * 4,
            });
        }
        let mut data = Vec::with_capacity(values.len() * 4);
        for v in values {
            data.extend_from_slice(&v.to_le_bytes());
        }
        Self::new(shape, DType::Fp32, data)
    }

    /// Build an INT8 tensor from `i8` values.
    pub fn from_i8(shape: Vec<u32>, values: &[i8]) -> Result<Self, SitfError> {
        let elems = element_count(&shape)?;
        if values.len() != elems {
            return Err(SitfError::ShapeMismatch {
                expected_elems: elems,
                data_len: values.len(),
            });
        }
        let data: Vec<u8> = values.iter().map(|v| *v as u8).collect();
        Self::new(shape, DType::Int8, data)
    }
}

/// Product of shape dimensions (0-rank tensor = 1 scalar element).
pub fn element_count(shape: &[u32]) -> Result<usize, SitfError> {
    if shape.is_empty() {
        return Ok(1);
    }
    let mut n: usize = 1;
    for &d in shape {
        n = n
            .checked_mul(d as usize)
            .ok_or(SitfError::Overflow)?;
    }
    Ok(n)
}

fn bytemuck_i8(data: &[u8]) -> &[i8] {
    // Reinterpret u8 buffer as i8 without allocation.
    unsafe { std::slice::from_raw_parts(data.as_ptr() as *const i8, data.len()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_fp32() {
        let t = SitfTensor::from_f32(vec![2, 3], &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0]).unwrap();
        let bytes = t.to_bytes().unwrap();
        let parsed = SitfTensor::from_bytes(&bytes).unwrap();
        assert_eq!(parsed, t);
        let vals = parsed.as_f32_vec().unwrap();
        assert_eq!(vals, vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
    }

    #[test]
    fn roundtrip_int8() {
        let t = SitfTensor::from_i8(vec![4], &[-1, 0, 1, 127]).unwrap();
        let bytes = t.to_bytes().unwrap();
        let parsed = SitfTensor::from_bytes(&bytes).unwrap();
        assert_eq!(parsed, t);
        let s = parsed.as_i8_slice().unwrap();
        assert_eq!(s, &[-1i8, 0, 1, 127]);
    }

    #[test]
    fn bad_magic() {
        let mut bytes = SitfTensor::zeros(vec![1], DType::Int8)
            .unwrap()
            .to_bytes()
            .unwrap();
        bytes[0] = b'X';
        assert_eq!(SitfTensor::from_bytes(&bytes), Err(SitfError::BadMagic));
    }

    #[test]
    fn shape_mismatch() {
        let err = SitfTensor::new(vec![2, 2], DType::Int8, vec![1, 2, 3]);
        assert!(matches!(err, Err(SitfError::ShapeMismatch { .. })));
    }

    #[test]
    fn zeros_fp32() {
        let t = SitfTensor::zeros(vec![2, 2], DType::Fp32).unwrap();
        assert_eq!(t.data.len(), 16);
        assert!(t.data.iter().all(|&b| b == 0));
    }

    #[test]
    fn truncated_buffer() {
        let bytes = SitfTensor::from_i8(vec![3], &[1, 2, 3])
            .unwrap()
            .to_bytes()
            .unwrap();
        let err = SitfTensor::from_bytes(&bytes[..10]);
        assert!(matches!(err, Err(SitfError::Truncated { .. })));
    }

    /// Encoding is bit-stable across repeated calls (deterministic tensor ops).
    #[test]
    fn encode_deterministic_across_runs() {
        let t = SitfTensor::from_f32(vec![2, 2], &[1.25, -0.5, 3.0, 0.0]).unwrap();
        let b1 = t.to_bytes().unwrap();
        let b2 = t.to_bytes().unwrap();
        assert_eq!(b1, b2);
        // Magic + version header is fixed.
        assert_eq!(&b1[0..4], b"SITF");
        assert_eq!(u16::from_le_bytes([b1[4], b1[5]]), SITF_VERSION);
        assert_eq!(b1[6], DType::Fp32.as_u8());
        assert_eq!(b1[7], 2); // rank
    }

    /// Golden on-wire header for a known INT8 vector (layout regression).
    #[test]
    fn golden_int8_wire_header() {
        let t = SitfTensor::from_i8(vec![2], &[10, -20]).unwrap();
        let bytes = t.to_bytes().unwrap();
        // magic + ver + dtype + rank + shape(2) + nbytes(2) + data
        assert_eq!(&bytes[0..4], b"SITF");
        assert_eq!(bytes[6], 0); // Int8
        assert_eq!(bytes[7], 1); // rank 1
        assert_eq!(u32::from_le_bytes(bytes[8..12].try_into().unwrap()), 2);
        assert_eq!(u64::from_le_bytes(bytes[12..20].try_into().unwrap()), 2);
        assert_eq!(bytes[20], 10u8);
        assert_eq!(bytes[21], (-20i8) as u8);
    }

    #[test]
    fn scalar_rank0_numel() {
        let t = SitfTensor::from_i8(vec![], &[42]).unwrap();
        assert_eq!(t.numel(), 1);
        assert_eq!(element_count(&[]).unwrap(), 1);
    }

    #[test]
    fn fp32_le_payload_bits() {
        let t = SitfTensor::from_f32(vec![1], &[1.0f32]).unwrap();
        assert_eq!(t.data, 1.0f32.to_le_bytes().to_vec());
    }
}
