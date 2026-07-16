//! Optional WAMR FFI integration surface (`--features wamr`).
//!
//! # Design
//! Real WebAssembly Micro Runtime (WAMR) linkage is **feature-gated** so the
//! default pure-Rust host simulation remains the CI/test path:
//!
//! ```text
//! cargo test -p sandbox              # host simulation, no native libs
//! cargo test -p sandbox --features wamr
//! ```
//!
//! This module documents the intended hooks (`wasm_runtime_full_init`, heap
//! size = [`crate::MEM_LIMIT`], bounded aux stack) without pulling in system
//! WAMR libraries. A future `wamr-sys` / `libiwasm` binding can replace the
//! stub bodies while keeping the same function signatures.

use crate::host::AUX_STACK_SIZE;
use crate::MEM_LIMIT;

/// Configuration mirror of `wasm_runtime_full_init` constraints for MossyMesh.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WamrInitConfig {
    /// Hard heap ceiling in bytes (must equal [`MEM_LIMIT`] on edge nodes).
    pub heap_size: usize,
    /// Bounded auxiliary stack size (`-z stack-size=N` intent).
    pub aux_stack_size: usize,
}

impl Default for WamrInitConfig {
    fn default() -> Self {
        Self {
            heap_size: MEM_LIMIT,
            aux_stack_size: AUX_STACK_SIZE,
        }
    }
}

/// Result of a feature-gated init attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WamrStatus {
    /// Native WAMR is not linked; callers should use [`crate::host::HostRuntime`].
    SimulationFallback,
    /// Reserved for a future successful native init.
    NativeReady,
}

/// Intended entry point corresponding to WAMR `wasm_runtime_full_init`.
///
/// Currently always returns [`WamrStatus::SimulationFallback`] so enabling the
/// `wamr` feature never breaks builds that lack `libiwasm`.
pub fn wasm_runtime_full_init(config: WamrInitConfig) -> WamrStatus {
    debug_assert_eq!(config.heap_size, MEM_LIMIT);
    debug_assert!(config.aux_stack_size > 0);
    // Native path would call:
    //   wasm_runtime_init / RuntimeInitArgs { mem_alloc_type, ... heap_size }
    // and configure aux stack via stack-size link flags.
    let _ = config;
    WamrStatus::SimulationFallback
}

/// Returns true when a native WAMR backend is active (always false today).
pub fn is_native_backend() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_falls_back_without_native_lib() {
        let status = wasm_runtime_full_init(WamrInitConfig::default());
        assert_eq!(status, WamrStatus::SimulationFallback);
        assert!(!is_native_backend());
    }
}
