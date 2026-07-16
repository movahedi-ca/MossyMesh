//! Deterministic GPU/Vulkan compute abstraction with CPU fallback.
//!
//! The `ComputeBackend` trait mirrors a Vulkan compute dispatch surface
//! (buffers in, buffers out, named ops) without requiring a GPU driver.
//! `CpuBackend` implements all ops with pure Rust, bit-stable results:
//! - FP32 matmul uses fixed left-to-right accumulation order
//! - Attention toy op uses the same fixed-order reductions
//!
//! No network. No nondeterministic threads in the hot path.

use std::fmt;

use crate::sitf::{DType, SitfError, SitfTensor};

/// Errors from compute backends.
#[derive(Debug, Clone, PartialEq)]
pub enum ComputeError {
    UnsupportedOp(&'static str),
    Shape(String),
    Sitf(SitfError),
    BackendUnavailable(&'static str),
}

impl fmt::Display for ComputeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ComputeError::UnsupportedOp(op) => write!(f, "unsupported op: {op}"),
            ComputeError::Shape(s) => write!(f, "shape error: {s}"),
            ComputeError::Sitf(e) => write!(f, "sitf: {e}"),
            ComputeError::BackendUnavailable(b) => write!(f, "backend unavailable: {b}"),
        }
    }
}

impl std::error::Error for ComputeError {}

impl From<SitfError> for ComputeError {
    fn from(e: SitfError) -> Self {
        ComputeError::Sitf(e)
    }
}

/// Matrix multiply descriptor: C[M,N] = A[M,K] × B[K,N]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MatMulOp {
    pub m: u32,
    pub k: u32,
    pub n: u32,
}

/// Toy scaled-dot-product attention over a single head:
/// out = softmax(Q K^T / sqrt(d)) V
///
/// Q: [seq, d], K: [seq, d], V: [seq, d] → out: [seq, d]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttentionOp {
    pub seq: u32,
    pub d: u32,
}

/// Vulkan-shaped compute trait: submit named ops over SITF buffers.
///
/// A real Vulkan backend would map tensors to `VkBuffer`s and dispatch
/// compute shaders; the CPU fallback stays pure Rust and deterministic.
pub trait ComputeBackend {
    /// Human-readable backend name (e.g. "cpu", "vulkan").
    fn name(&self) -> &'static str;

    /// Whether this backend is currently usable on the host.
    fn is_available(&self) -> bool;

    /// FP32 matrix multiply. `a` shape [m,k], `b` shape [k,n] → [m,n].
    fn matmul_f32(
        &self,
        op: MatMulOp,
        a: &SitfTensor,
        b: &SitfTensor,
    ) -> Result<SitfTensor, ComputeError>;

    /// Toy single-head attention (FP32).
    fn attention_f32(
        &self,
        op: AttentionOp,
        q: &SitfTensor,
        k: &SitfTensor,
        v: &SitfTensor,
    ) -> Result<SitfTensor, ComputeError>;

    /// INT8 elementwise add (saturating). Shapes must match.
    fn add_i8(&self, a: &SitfTensor, b: &SitfTensor) -> Result<SitfTensor, ComputeError>;
}

/// Pure-Rust deterministic CPU backend (Vulkan compute fallback).
#[derive(Debug, Default, Clone, Copy)]
pub struct CpuBackend;

impl CpuBackend {
    pub fn new() -> Self {
        Self
    }
}

impl ComputeBackend for CpuBackend {
    fn name(&self) -> &'static str {
        "cpu"
    }

    fn is_available(&self) -> bool {
        true
    }

    fn matmul_f32(
        &self,
        op: MatMulOp,
        a: &SitfTensor,
        b: &SitfTensor,
    ) -> Result<SitfTensor, ComputeError> {
        validate_matrix(a, op.m, op.k, "A")?;
        validate_matrix(b, op.k, op.n, "B")?;
        let av = a
            .as_f32_vec()
            .ok_or_else(|| ComputeError::Shape("A must be FP32".into()))?;
        let bv = b
            .as_f32_vec()
            .ok_or_else(|| ComputeError::Shape("B must be FP32".into()))?;
        let m = op.m as usize;
        let k = op.k as usize;
        let n = op.n as usize;
        let mut out = vec![0f32; m * n];
        // Fixed i→j→p accumulation order for cross-run bit stability.
        for i in 0..m {
            for j in 0..n {
                let mut acc = 0f32;
                for p in 0..k {
                    acc = acc + av[i * k + p] * bv[p * n + j];
                }
                out[i * n + j] = acc;
            }
        }
        SitfTensor::from_f32(vec![op.m, op.n], &out).map_err(Into::into)
    }

    fn attention_f32(
        &self,
        op: AttentionOp,
        q: &SitfTensor,
        k: &SitfTensor,
        v: &SitfTensor,
    ) -> Result<SitfTensor, ComputeError> {
        let seq = op.seq as usize;
        let d = op.d as usize;
        validate_matrix(q, op.seq, op.d, "Q")?;
        validate_matrix(k, op.seq, op.d, "K")?;
        validate_matrix(v, op.seq, op.d, "V")?;
        let qv = q
            .as_f32_vec()
            .ok_or_else(|| ComputeError::Shape("Q FP32".into()))?;
        let kv = k
            .as_f32_vec()
            .ok_or_else(|| ComputeError::Shape("K FP32".into()))?;
        let vv = v
            .as_f32_vec()
            .ok_or_else(|| ComputeError::Shape("V FP32".into()))?;

        let scale = inv_sqrt_f32(d as f32);
        // scores[seq, seq]
        let mut scores = vec![0f32; seq * seq];
        for i in 0..seq {
            for j in 0..seq {
                let mut dot = 0f32;
                for t in 0..d {
                    dot = dot + qv[i * d + t] * kv[j * d + t];
                }
                scores[i * seq + j] = dot * scale;
            }
        }
        // row-wise softmax (stable: subtract max)
        let mut weights = vec![0f32; seq * seq];
        for i in 0..seq {
            let row = &scores[i * seq..(i + 1) * seq];
            let mut max_v = row[0];
            for &x in row.iter().skip(1) {
                if x > max_v {
                    max_v = x;
                }
            }
            let mut sum = 0f32;
            for j in 0..seq {
                // Same-process determinism: fixed reduction order + IEEE f32.
                let e = (row[j] - max_v).exp();
                weights[i * seq + j] = e;
                sum = sum + e;
            }
            let inv = if sum == 0.0 { 0.0 } else { 1.0 / sum };
            for j in 0..seq {
                weights[i * seq + j] = weights[i * seq + j] * inv;
            }
        }
        // out = weights × V
        let mut out = vec![0f32; seq * d];
        for i in 0..seq {
            for t in 0..d {
                let mut acc = 0f32;
                for j in 0..seq {
                    acc = acc + weights[i * seq + j] * vv[j * d + t];
                }
                out[i * d + t] = acc;
            }
        }
        SitfTensor::from_f32(vec![op.seq, op.d], &out).map_err(Into::into)
    }

    fn add_i8(&self, a: &SitfTensor, b: &SitfTensor) -> Result<SitfTensor, ComputeError> {
        if a.dtype != DType::Int8 || b.dtype != DType::Int8 {
            return Err(ComputeError::Shape("add_i8 requires INT8 tensors".into()));
        }
        if a.shape != b.shape {
            return Err(ComputeError::Shape(format!(
                "shape mismatch {:?} vs {:?}",
                a.shape, b.shape
            )));
        }
        let aa = a.as_i8_slice().unwrap();
        let bb = b.as_i8_slice().unwrap();
        let mut out = Vec::with_capacity(aa.len());
        for i in 0..aa.len() {
            out.push(aa[i].saturating_add(bb[i]));
        }
        SitfTensor::from_i8(a.shape.clone(), &out).map_err(Into::into)
    }
}

/// Stub Vulkan backend: reports availability false; all ops fall through error
/// unless `force_available` is set (then delegates to CPU for shader parity).
#[derive(Debug, Default, Clone, Copy)]
pub struct VulkanBackend {
    /// Simulated presence of a Vulkan ICD (always false in pure-Rust offline builds).
    pub force_available: bool,
}

impl VulkanBackend {
    pub fn new() -> Self {
        Self {
            force_available: false,
        }
    }
}

impl ComputeBackend for VulkanBackend {
    fn name(&self) -> &'static str {
        "vulkan"
    }

    fn is_available(&self) -> bool {
        self.force_available
    }

    fn matmul_f32(
        &self,
        op: MatMulOp,
        a: &SitfTensor,
        b: &SitfTensor,
    ) -> Result<SitfTensor, ComputeError> {
        if !self.is_available() {
            return Err(ComputeError::BackendUnavailable("vulkan"));
        }
        // When forced available, delegate to CPU for identical results (shader parity).
        CpuBackend.matmul_f32(op, a, b)
    }

    fn attention_f32(
        &self,
        op: AttentionOp,
        q: &SitfTensor,
        k: &SitfTensor,
        v: &SitfTensor,
    ) -> Result<SitfTensor, ComputeError> {
        if !self.is_available() {
            return Err(ComputeError::BackendUnavailable("vulkan"));
        }
        CpuBackend.attention_f32(op, q, k, v)
    }

    fn add_i8(&self, a: &SitfTensor, b: &SitfTensor) -> Result<SitfTensor, ComputeError> {
        if !self.is_available() {
            return Err(ComputeError::BackendUnavailable("vulkan"));
        }
        CpuBackend.add_i8(a, b)
    }
}

/// Owned backend choice used by the high-compute tier.
#[derive(Debug, Clone, Copy)]
pub enum PreferredBackend {
    Cpu(CpuBackend),
    Vulkan(VulkanBackend),
}

impl PreferredBackend {
    /// Prefer Vulkan when marked available; otherwise CPU.
    pub fn auto(vulkan: VulkanBackend, cpu: CpuBackend) -> Self {
        if vulkan.is_available() {
            PreferredBackend::Vulkan(vulkan)
        } else {
            PreferredBackend::Cpu(cpu)
        }
    }

    pub fn as_dyn(&self) -> &dyn ComputeBackend {
        match self {
            PreferredBackend::Cpu(c) => c,
            PreferredBackend::Vulkan(v) => v,
        }
    }
}

fn validate_matrix(
    t: &SitfTensor,
    rows: u32,
    cols: u32,
    name: &str,
) -> Result<(), ComputeError> {
    if t.dtype != DType::Fp32 {
        return Err(ComputeError::Shape(format!("{name} must be FP32")));
    }
    if t.shape != vec![rows, cols] {
        return Err(ComputeError::Shape(format!(
            "{name} expected shape [{rows}, {cols}], got {:?}",
            t.shape
        )));
    }
    Ok(())
}

/// Bit-stable inverse square root via IEEE f32 operations (same inputs → same bits).
fn inv_sqrt_f32(x: f32) -> f32 {
    if x <= 0.0 {
        return 0.0;
    }
    1.0 / x.sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matmul_identity() {
        let cpu = CpuBackend::new();
        let a = SitfTensor::from_f32(vec![2, 2], &[1.0, 2.0, 3.0, 4.0]).unwrap();
        let i = SitfTensor::from_f32(vec![2, 2], &[1.0, 0.0, 0.0, 1.0]).unwrap();
        let c = cpu
            .matmul_f32(MatMulOp { m: 2, k: 2, n: 2 }, &a, &i)
            .unwrap();
        assert_eq!(c.as_f32_vec().unwrap(), vec![1.0, 2.0, 3.0, 4.0]);
    }

    #[test]
    fn matmul_known() {
        let cpu = CpuBackend::new();
        // [1 2; 3 4] × [5 6; 7 8] = [19 22; 43 50]
        let a = SitfTensor::from_f32(vec![2, 2], &[1.0, 2.0, 3.0, 4.0]).unwrap();
        let b = SitfTensor::from_f32(vec![2, 2], &[5.0, 6.0, 7.0, 8.0]).unwrap();
        let c = cpu
            .matmul_f32(MatMulOp { m: 2, k: 2, n: 2 }, &a, &b)
            .unwrap();
        assert_eq!(c.as_f32_vec().unwrap(), vec![19.0, 22.0, 43.0, 50.0]);
    }

    #[test]
    fn matmul_deterministic_across_runs() {
        let cpu = CpuBackend::new();
        let a = SitfTensor::from_f32(
            vec![3, 3],
            &[0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9],
        )
        .unwrap();
        let b = SitfTensor::from_f32(vec![3, 2], &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0]).unwrap();
        let op = MatMulOp { m: 3, k: 3, n: 2 };
        let r1 = cpu.matmul_f32(op, &a, &b).unwrap();
        let r2 = cpu.matmul_f32(op, &a, &b).unwrap();
        // Exact bit equality of payload
        assert_eq!(r1.data, r2.data);
        let r3 = cpu.matmul_f32(op, &a, &b).unwrap();
        assert_eq!(r1.data, r3.data);
    }

    #[test]
    fn attention_toy_deterministic() {
        let cpu = CpuBackend::new();
        let seq = 2u32;
        let d = 2u32;
        let q = SitfTensor::from_f32(vec![seq, d], &[1.0, 0.0, 0.0, 1.0]).unwrap();
        let k = SitfTensor::from_f32(vec![seq, d], &[1.0, 0.0, 0.0, 1.0]).unwrap();
        let v = SitfTensor::from_f32(vec![seq, d], &[1.0, 2.0, 3.0, 4.0]).unwrap();
        let op = AttentionOp { seq, d };
        let o1 = cpu.attention_f32(op, &q, &k, &v).unwrap();
        let o2 = cpu.attention_f32(op, &q, &k, &v).unwrap();
        assert_eq!(o1.data, o2.data);
        assert_eq!(o1.shape, vec![2, 2]);
        // Output should be finite
        for x in o1.as_f32_vec().unwrap() {
            assert!(x.is_finite());
        }
    }

    #[test]
    fn add_i8_saturating() {
        let cpu = CpuBackend::new();
        let a = SitfTensor::from_i8(vec![3], &[100, -100, 0]).unwrap();
        let b = SitfTensor::from_i8(vec![3], &[100, -100, 5]).unwrap();
        let c = cpu.add_i8(&a, &b).unwrap();
        assert_eq!(c.as_i8_slice().unwrap(), &[127i8, -128, 5]);
    }

    #[test]
    fn vulkan_unavailable_falls_back_policy() {
        let vk = VulkanBackend::new();
        let cpu = CpuBackend::new();
        assert!(!vk.is_available());
        let pref = PreferredBackend::auto(vk, cpu);
        assert_eq!(pref.as_dyn().name(), "cpu");
    }

    #[test]
    fn vulkan_force_matches_cpu() {
        let mut vk = VulkanBackend::new();
        vk.force_available = true;
        let cpu = CpuBackend::new();
        let a = SitfTensor::from_f32(vec![2, 2], &[1.0, 0.0, 0.0, 1.0]).unwrap();
        let b = SitfTensor::from_f32(vec![2, 2], &[2.0, 3.0, 4.0, 5.0]).unwrap();
        let op = MatMulOp { m: 2, k: 2, n: 2 };
        let r_vk = vk.matmul_f32(op, &a, &b).unwrap();
        let r_cpu = cpu.matmul_f32(op, &a, &b).unwrap();
        assert_eq!(r_vk.data, r_cpu.data);
    }
}
