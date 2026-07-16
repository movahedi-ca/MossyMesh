//! AI Processing Module for MossyMesh
//!
//! High-compute tier primitives that run fully offline:
//! - **SITF** — Simple Intermediate Tensor Format (shape, dtype, raw bytes)
//! - **Edge PagedAttention** — page-table context windows (memory or disk-mapped)
//! - **ComputeBackend** — Vulkan-shaped compute trait with a deterministic CPU fallback
//!
//! No network or cloud calls. All ops are pure and bit-stable across runs.

pub mod sitf;
pub mod paged_attention;
pub mod compute;

pub use sitf::{DType, SitfError, SitfTensor};
pub use paged_attention::{PageId, PageTable, PagedAttention, PagedAttentionError};
pub use compute::{
    AttentionOp, ComputeBackend, ComputeError, CpuBackend, MatMulOp, PreferredBackend,
    VulkanBackend,
};

/// Initialize the AI processing subsystem (offline, side-effect free for determinism).
pub fn init_ai() {}
