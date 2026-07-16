//! # Deterministic compute backends
//!
//! Vulkan-shaped compute abstraction with a **real** CPU path and an honest
//! **Vulkan stub**.
//!
//! ## Feature flags (see `ai/Cargo.toml`)
//!
//! | Feature | Reality |
//! | --- | --- |
//! | *(default)* | Pure-Rust only; [`VulkanBackend`] is unavailable |
//! | `vulkan-stub` | Documents stub usage at compile time — **still no ICD / shaders** |
//!
//! There is intentionally **no** `vulkan` feature that links `ash` or loads a
//! driver. A future real GPU backend must introduce new optional deps and must
//! not silently claim availability when only the stub is present.
//!
//! ## Ops
//!
//! - FP32 matmul: fixed left-to-right accumulation order (bit-stable)
//! - Toy single-head attention: same fixed-order reductions + stable softmax
//! - INT8 elementwise add: saturating arithmetic
//!
//! No network. No nondeterministic threads in the hot path.

use std::fmt;

use crate::sitf::{DType, SitfError, SitfTensor};

/// Errors from compute backends.
#[derive(Debug, Clone, PartialEq)]
pub enum ComputeError {
    /// Named op is not implemented on this backend.
    UnsupportedOp(&'static str),
    /// Shape / dtype validation failed.
    Shape(String),
    /// Nested SITF error.
    Sitf(SitfError),
    /// Backend is not present on this host (expected for the Vulkan stub).
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

/// Matrix multiply descriptor: `C[M,N] = A[M,K] × B[K,N]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MatMulOp {
    pub m: u32,
    pub k: u32,
    pub n: u32,
}

/// Toy scaled-dot-product attention over a single head:
/// `out = softmax(Q K^T / sqrt(d)) V`
///
/// Q, K, V: `[seq, d]` → out: `[seq, d]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttentionOp {
    pub seq: u32,
    pub d: u32,
}

/// Vulkan-shaped compute trait: submit named ops over SITF buffers.
///
/// A real Vulkan backend would map tensors to `VkBuffer`s and dispatch
/// compute shaders; [`CpuBackend`] stays pure Rust and deterministic.
/// [`VulkanBackend`] is a **stub** — see type docs and the `vulkan-stub` feature.
pub trait ComputeBackend {
    /// Human-readable backend name (e.g. `"cpu"`, `"vulkan"`).
    fn name(&self) -> &'static str;

    /// Whether this backend is currently usable on the host.
    ///
    /// For [`VulkanBackend`], this is **false** unless tests set
    /// `force_available` — never treat a true result as proof of a real ICD
    /// unless a future non-stub feature is added.
    fn is_available(&self) -> bool;

    /// FP32 matrix multiply. `a` shape `[m,k]`, `b` shape `[k,n]` → `[m,n]`.
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

/// Pure-Rust deterministic CPU backend (the production offline path).
///
/// Also used as the fallback when the Vulkan stub is unavailable, and as the
/// delegate when the stub is force-enabled for parity tests.
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

/// **Stub** Vulkan backend — not a real GPU path.
///
/// # Honesty contract
///
/// - Default [`VulkanBackend::new`] → `is_available() == false`
/// - Ops return [`ComputeError::BackendUnavailable`] when unavailable
/// - `force_available = true` is **test-only**: still runs on [`CpuBackend`],
///   never allocates `VkBuffer` or loads an ICD
/// - Crate feature `vulkan-stub` documents intent only; enabling it does not
///   change runtime behavior or link Vulkan libraries
///
/// Prefer [`PreferredBackend::auto`] so production stays on [`CpuBackend`].
#[derive(Debug, Default, Clone, Copy)]
pub struct VulkanBackend {
    /// Test-only switch that *pretends* a Vulkan ICD exists.
    ///
    /// Even when true, work is delegated to [`CpuBackend`] (CPU parity for
    /// shader-shaped APIs). This is **not** GPU acceleration.
    ///
    /// Gated documentation: the `vulkan-stub` Cargo feature exists so call
    /// sites can `cfg!(feature = "vulkan-stub")` when they intentionally use
    /// this field; the field itself is always present so default builds keep
    /// a single API surface.
    pub force_available: bool,
}

impl VulkanBackend {
    /// Construct an unavailable stub (`is_available() == false`).
    pub fn new() -> Self {
        Self {
            force_available: false,
        }
    }
}

impl ComputeBackend for VulkanBackend {
    fn name(&self) -> &'static str {
        // Name stays "vulkan" for API shape parity; availability is the truth signal.
        "vulkan"
    }

    fn is_available(&self) -> bool {
        // Honest default: no ICD. Only force_available (tests) flips this.
        // Future real integration must not set this true without a driver probe.
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
        // Stub path: CPU delegate for identical bits (not a shader dispatch).
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
    /// Only selected when the stub reports available (normally never).
    Vulkan(VulkanBackend),
}

impl PreferredBackend {
    /// Prefer Vulkan when marked available; otherwise CPU.
    ///
    /// With the default stub, this always returns [`PreferredBackend::Cpu`].
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

    /// Golden LE payload bits for a small matmul (deterministic tensor ops).
    #[test]
    fn matmul_golden_payload_bits() {
        let cpu = CpuBackend::new();
        let a = SitfTensor::from_f32(vec![2, 2], &[1.0, 2.0, 3.0, 4.0]).unwrap();
        let b = SitfTensor::from_f32(vec![2, 2], &[5.0, 6.0, 7.0, 8.0]).unwrap();
        let c = cpu
            .matmul_f32(MatMulOp { m: 2, k: 2, n: 2 }, &a, &b)
            .unwrap();
        let expected: Vec<u8> = [19.0f32, 22.0, 43.0, 50.0]
            .into_iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        assert_eq!(c.data, expected);
        // SITF encode of the result is also stable.
        let wire1 = c.to_bytes().unwrap();
        let wire2 = c.to_bytes().unwrap();
        assert_eq!(wire1, wire2);
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

    /// Attention is bit-stable across independent backend instances.
    #[test]
    fn attention_bit_stable_fresh_backends() {
        let seq = 3u32;
        let d = 2u32;
        let q = SitfTensor::from_f32(
            vec![seq, d],
            &[0.5, 0.0, 0.0, 0.5, 0.25, 0.25],
        )
        .unwrap();
        let k = q.clone();
        let v = SitfTensor::from_f32(
            vec![seq, d],
            &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0],
        )
        .unwrap();
        let op = AttentionOp { seq, d };
        let o1 = CpuBackend::new().attention_f32(op, &q, &k, &v).unwrap();
        let o2 = CpuBackend::new().attention_f32(op, &q, &k, &v).unwrap();
        assert_eq!(o1.data, o2.data);
        assert_eq!(o1.to_bytes().unwrap(), o2.to_bytes().unwrap());
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
    fn add_i8_deterministic_bits() {
        let cpu = CpuBackend::new();
        let a = SitfTensor::from_i8(vec![4], &[1, 2, 3, 4]).unwrap();
        let b = SitfTensor::from_i8(vec![4], &[10, 20, 30, 40]).unwrap();
        let c1 = cpu.add_i8(&a, &b).unwrap();
        let c2 = cpu.add_i8(&a, &b).unwrap();
        assert_eq!(c1.data, c2.data);
        assert_eq!(c1.as_i8_slice().unwrap(), &[11i8, 22, 33, 44]);
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
    fn vulkan_stub_ops_error_when_unavailable() {
        let vk = VulkanBackend::new();
        let a = SitfTensor::from_f32(vec![1, 1], &[1.0]).unwrap();
        let err = vk
            .matmul_f32(MatMulOp { m: 1, k: 1, n: 1 }, &a, &a)
            .unwrap_err();
        assert_eq!(err, ComputeError::BackendUnavailable("vulkan"));
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

    #[test]
    fn vulkan_stub_feature_is_documentation_only() {
        // Compiles whether or not `vulkan-stub` is enabled; documents that the
        // feature does not imply a real ICD.
        let _ = cfg!(feature = "vulkan-stub");
        let vk = VulkanBackend::new();
        assert!(!vk.is_available());
        assert_eq!(vk.name(), "vulkan");
    }
}
