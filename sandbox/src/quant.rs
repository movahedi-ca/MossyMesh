//! Symmetric Static INT8 quantization helpers for tensor payloads (edge AI).
//!
//! Symmetric static quantization uses a single positive scale `s` and a fixed
//! zero-point of 0:
//!
//! ```text
//! q = clamp(round(x / s), -127, 127)
//! x̂ = q * s
//! s = max(|x|) / 127   (or a caller-supplied static scale)
//! ```
//!
//! The scheme is deterministic across devices (no per-channel dynamic stats
//! beyond the agreed static scale), which keeps mesh verification simple.

/// Inclusive INT8 magnitude used for symmetric mapping (excludes -128 so the
/// representable range is perfectly symmetric around zero).
pub const INT8_ABS_MAX: i8 = 127;

/// Parameters for symmetric static INT8 quantization.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SymmetricInt8Params {
    /// Positive scale such that `dequant ≈ quant * scale`.
    pub scale: f32,
}

impl SymmetricInt8Params {
    /// Build params from an explicit positive scale.
    pub fn from_scale(scale: f32) -> Result<Self, QuantError> {
        if !scale.is_finite() || scale <= 0.0 {
            return Err(QuantError::InvalidScale);
        }
        Ok(Self { scale })
    }

    /// Derive a static scale from a floating-point tensor: `max(|x|) / 127`.
    /// Empty tensors and all-zero tensors use scale `1.0` (identity mapping).
    pub fn from_tensor(data: &[f32]) -> Result<Self, QuantError> {
        if data.iter().any(|v| !v.is_finite()) {
            return Err(QuantError::NonFiniteValue);
        }
        let max_abs = data.iter().fold(0.0f32, |acc, &v| acc.max(v.abs()));
        if max_abs == 0.0 {
            return Ok(Self { scale: 1.0 });
        }
        Self::from_scale(max_abs / (INT8_ABS_MAX as f32))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuantError {
    InvalidScale,
    NonFiniteValue,
}

impl QuantError {
    pub fn as_str(self) -> &'static str {
        match self {
            QuantError::InvalidScale => "Scale must be finite and > 0.",
            QuantError::NonFiniteValue => "Tensor contains NaN or Inf.",
        }
    }
}

impl core::fmt::Display for QuantError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Quantize a single float with symmetric static INT8.
#[inline]
pub fn quantize_value(x: f32, scale: f32) -> i8 {
    debug_assert!(scale > 0.0 && scale.is_finite());
    let q = (x / scale).round();
    q.clamp(-(INT8_ABS_MAX as f32), INT8_ABS_MAX as f32) as i8
}

/// Dequantize a single INT8 value.
#[inline]
pub fn dequantize_value(q: i8, scale: f32) -> f32 {
    (q as f32) * scale
}

/// Quantize a tensor slice into INT8 using the provided static params.
pub fn quantize_symmetric(data: &[f32], params: &SymmetricInt8Params) -> Result<Vec<i8>, QuantError> {
    if data.iter().any(|v| !v.is_finite()) {
        return Err(QuantError::NonFiniteValue);
    }
    Ok(data
        .iter()
        .map(|&x| quantize_value(x, params.scale))
        .collect())
}

/// Dequantize INT8 payload back to f32.
pub fn dequantize_symmetric(data: &[i8], params: &SymmetricInt8Params) -> Vec<f32> {
    data.iter()
        .map(|&q| dequantize_value(q, params.scale))
        .collect()
}

/// Convenience: derive scale from the tensor, quantize, and return both.
pub fn quantize_tensor(data: &[f32]) -> Result<(Vec<i8>, SymmetricInt8Params), QuantError> {
    let params = SymmetricInt8Params::from_tensor(data)?;
    let q = quantize_symmetric(data, &params)?;
    Ok((q, params))
}

/// Maximum absolute reconstruction error bound for values within the scale range.
/// For `|x| <= scale * 127`, error is at most `scale / 2` (rounding).
pub fn max_roundtrip_error(params: &SymmetricInt8Params) -> f32 {
    params.scale / 2.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_stays_within_half_scale() {
        let data = vec![-3.5f32, -1.0, 0.0, 0.5, 2.25, 3.5];
        let (q, params) = quantize_tensor(&data).unwrap();
        assert!(q.iter().all(|&v| v >= -INT8_ABS_MAX && v <= INT8_ABS_MAX));
        let recon = dequantize_symmetric(&q, &params);
        let bound = max_roundtrip_error(&params) + f32::EPSILON;
        for (a, b) in data.iter().zip(recon.iter()) {
            assert!(
                (a - b).abs() <= bound,
                "roundtrip error {} exceeds bound {} (scale={})",
                (a - b).abs(),
                bound,
                params.scale
            );
        }
    }

    #[test]
    fn clamps_out_of_range_values() {
        let params = SymmetricInt8Params::from_scale(1.0).unwrap();
        assert_eq!(quantize_value(500.0, params.scale), INT8_ABS_MAX);
        assert_eq!(quantize_value(-500.0, params.scale), -INT8_ABS_MAX);
    }
}
