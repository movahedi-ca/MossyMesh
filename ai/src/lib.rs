//! # MossyMesh AI Processing
//!
//! High-compute tier primitives that run fully **offline** and **bit-stable**:
//!
//! | Module | Public surface | Role |
//! | --- | --- | --- |
//! | [`sitf`] | [`SitfTensor`], [`DType`], [`SitfError`] | Compact LE tensor container (encode / decode / views) |
//! | [`paged_attention`] | [`PagedAttention`], [`PageTable`], [`PageId`] | vLLM-style page-table KV / context windows (RAM or file) |
//! | [`compute`] | [`ComputeBackend`], [`CpuBackend`], [`VulkanBackend`] | Named ops over SITF buffers; CPU is the real path |
//!
//! ## SITF (Simple Intermediate Tensor Format)
//!
//! [`SitfTensor`] is the interchange type for edge AI payloads:
//!
//! - **DTypes**: [`DType::Int8`] (quantized), [`DType::Fp32`] (compute / replay)
//! - **Layout**: little-endian header + row-major payload — see [`sitf`] module docs
//! - **API**: construct with [`SitfTensor::new`] / [`SitfTensor::from_f32`] /
//!   [`SitfTensor::from_i8`], wire with [`SitfTensor::to_bytes`] /
//!   [`SitfTensor::from_bytes`], view with [`SitfTensor::as_f32_vec`] /
//!   [`SitfTensor::as_i8_slice`]
//!
//! ## Edge PagedAttention
//!
//! [`PagedAttention`] stores per-token KV blobs in a [`PageTable`]:
//!
//! - Logical slots map 1:1 to fixed-size physical pages
//! - Oversubscription uses **deterministic LRU** (generation counter, not wall-clock)
//! - File-backed tables simulate disk-mapped context without OS `mmap` APIs
//! - Typical flow: [`PagedAttention::append_token`] / [`PagedAttention::append_tensor`]
//!   → [`PagedAttention::gather`] / [`PagedAttention::gather_fp32_tensor`]
//!
//! Lower-level slot ops live on [`PageTable`] (`map_slot`, `write_slot`, `read_slot`).
//!
//! ## Compute backends
//!
//! - [`CpuBackend`]: pure-Rust matmul, toy attention, saturating INT8 add — **deterministic**
//! - [`VulkanBackend`]: **stub only** (see crate feature `vulkan-stub` and module docs).
//!   Prefer [`PreferredBackend::auto`] so hosts without a real GPU path stay on CPU.
//!
//! No network or cloud calls. Hot-path ops avoid nondeterministic threading.

pub mod sitf;
pub mod paged_attention;
pub mod compute;

pub use sitf::{DType, SitfError, SitfTensor, MAX_RANK, SITF_MAGIC, SITF_VERSION};
pub use paged_attention::{
    PageBackendKind, PageId, PageTable, PagedAttention, PagedAttentionError,
};
pub use compute::{
    AttentionOp, ComputeBackend, ComputeError, CpuBackend, MatMulOp, PreferredBackend,
    VulkanBackend,
};

/// Initialize the AI processing subsystem (offline, side-effect free for determinism).
///
/// Currently a no-op reserved for future offline registry / capability probes.
pub fn init_ai() {}
