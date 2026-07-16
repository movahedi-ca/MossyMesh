//! Pure-Rust host simulation of the WAMR/WASI execution surface.
//!
//! Real WAMR FFI is optional and feature-gated (`wamr`). Default builds use this
//! host so unit tests and edge nodes without native WAMR still observe the same
//! deterministic semantics: fixed-block heap under [`crate::MEM_LIMIT`], known
//! export dispatch, and stable error strings on OOM / missing exports.
//!
//! Bounded auxiliary stack: guest call frames are limited by
//! [`AUX_STACK_SIZE`] (mirrors WAMR `-z stack-size=N` intent).

use crate::pool::{FixedBlockPool, PoolError};
use crate::{DEFAULT_BLOCK_SIZE, MEM_LIMIT};

/// Default bounded aux-stack size for guest invocations (bytes).
/// Aligns with the blueprint's "bounded aux stack (`-z stack-size=N`)" guardrail.
pub const AUX_STACK_SIZE: usize = 64 * 1024; // 64 KiB

/// Errors raised by the simulated host / guest boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostError {
    OutOfMemory,
    ExportNotFound,
    InvalidModule,
    AuxStackOverflow,
    Pool(PoolError),
}

impl HostError {
    /// Deterministic, stable string for cross-node agreement.
    pub fn as_str(&self) -> &'static str {
        match self {
            HostError::OutOfMemory => "Allocation failed: 10MB memory limit exceeded.",
            HostError::ExportNotFound => "FFI Error: Exported function not found in WASM module.",
            HostError::InvalidModule => "Host Error: Invalid or empty WASM module bytes.",
            HostError::AuxStackOverflow => "Host Error: Bounded aux stack overflow.",
            HostError::Pool(e) => e.as_str(),
        }
    }
}

impl core::fmt::Display for HostError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<PoolError> for HostError {
    fn from(value: PoolError) -> Self {
        match value {
            PoolError::OutOfMemory => HostError::OutOfMemory,
            // Construction / config faults surface as pool errors (stable string).
            PoolError::LimitExceedsGlobal
            | PoolError::InvalidBlockSize
            | PoolError::ZeroSize
            | PoolError::InvalidHandle => HostError::Pool(value),
        }
    }
}

/// Simulated guest module loaded into the host.
#[derive(Debug)]
pub struct HostRuntime {
    pub module_bytes: Vec<u8>,
    pool: FixedBlockPool,
    /// Names the guest is treated as exporting (simulation registry).
    exports: Vec<String>,
    /// Tracks nested invoke depth against [`AUX_STACK_SIZE`] (frame estimate).
    call_depth: usize,
    /// Estimated bytes per simulated call frame on the aux stack.
    frame_size: usize,
}

impl HostRuntime {
    /// Load module bytes into a new host with default block size and MEM_LIMIT.
    pub fn load(module_bytes: Vec<u8>) -> Result<Self, HostError> {
        Self::load_with_config(module_bytes, DEFAULT_BLOCK_SIZE, MEM_LIMIT)
    }

    /// Load with configurable fixed-block size and memory ceiling.
    pub fn load_with_config(
        module_bytes: Vec<u8>,
        block_size: usize,
        mem_limit: usize,
    ) -> Result<Self, HostError> {
        if module_bytes.is_empty() {
            return Err(HostError::InvalidModule);
        }
        let pool = FixedBlockPool::with_limit(block_size, mem_limit)?;
        let exports = discover_exports(&module_bytes);
        Ok(Self {
            module_bytes,
            pool,
            exports,
            call_depth: 0,
            // 4 KiB pseudo-frame keeps many nested calls under AUX_STACK_SIZE.
            frame_size: 4 * 1024,
        })
    }

    pub fn used_memory(&self) -> usize {
        self.pool.used_bytes()
    }

    pub fn mem_limit(&self) -> usize {
        self.pool.mem_limit()
    }

    pub fn block_size(&self) -> usize {
        self.pool.block_size()
    }

    pub fn exports(&self) -> &[String] {
        &self.exports
    }

    /// Allocate `size` bytes from the fixed-block pool.
    /// Returns a guest pointer (byte offset) or a deterministic OOM error.
    pub fn allocate(&mut self, size: usize) -> Result<usize, HostError> {
        let handle = self.pool.allocate(size).map_err(HostError::from)?;
        Ok(handle.offset(self.pool.block_size()))
    }

    /// Invoke an exported function by name with raw argument bytes.
    pub fn invoke(&mut self, export: &str, args: &[u8]) -> Result<Vec<u8>, HostError> {
        if !self.exports.iter().any(|e| e == export) {
            return Err(HostError::ExportNotFound);
        }

        // Bounded aux stack: each invoke consumes a simulated frame.
        let next_depth = self.call_depth.saturating_add(1);
        if next_depth.saturating_mul(self.frame_size) > AUX_STACK_SIZE {
            return Err(HostError::AuxStackOverflow);
        }
        self.call_depth = next_depth;

        let result = self.dispatch(export, args);

        self.call_depth = self.call_depth.saturating_sub(1);
        result
    }

    fn dispatch(&mut self, export: &str, args: &[u8]) -> Result<Vec<u8>, HostError> {
        match export {
            "evaluate_move" => {
                // Simulate engine bitboard bridge: non-empty args → valid move.
                if args.is_empty() {
                    Ok(vec![0x00])
                } else {
                    Ok(vec![0x01])
                }
            }
            "get_best_move" => Ok(vec![0xE2, 0xE4]), // e2-e4
            "echo" => Ok(args.to_vec()),
            "alloc_probe" => {
                // Allocate `args[0]` pages of block_size (or 1 block if empty).
                let n_blocks = args.first().copied().unwrap_or(1) as usize;
                let size = n_blocks.saturating_mul(self.pool.block_size()).max(1);
                let ptr = self.allocate(size)?;
                // Return pointer as little-endian u32 for determinism.
                Ok((ptr as u32).to_le_bytes().to_vec())
            }
            "quant_scale_probe" => {
                // Host-side helper exposed for edge-AI prep tests: returns scale
                // bytes for a trivial constant tensor [1.0, -1.0].
                let data = [1.0f32, -1.0f32];
                let params = crate::quant::SymmetricInt8Params::from_tensor(&data)
                    .map_err(|_| HostError::InvalidModule)?;
                Ok(params.scale.to_le_bytes().to_vec())
            }
            _ => {
                // Custom exports discovered from the module marker still succeed
                // with an empty payload so loaders can feature-detect.
                Ok(Vec::new())
            }
        }
    }
}

/// Default simulated exports for the MessyMash chess / edge-AI PoC.
fn default_exports() -> Vec<String> {
    vec![
        "evaluate_move".into(),
        "get_best_move".into(),
        "echo".into(),
        "alloc_probe".into(),
        "quant_scale_probe".into(),
    ]
}

/// Discover exports for the simulated guest.
///
/// Real WASM export sections are not parsed here. Instead:
/// 1. Always register the default PoC exports.
/// 2. If module bytes contain the ASCII marker `MOSSYMESH_EXPORTS:` followed by
///    comma-separated names (until newline or end), those names are added.
///
/// This keeps tests free of a WASM decoder while still exercising custom exports.
fn discover_exports(module_bytes: &[u8]) -> Vec<String> {
    let mut exports = default_exports();
    const MARKER: &[u8] = b"MOSSYMESH_EXPORTS:";
    if let Some(pos) = find_subslice(module_bytes, MARKER) {
        let rest = &module_bytes[pos + MARKER.len()..];
        let end = rest
            .iter()
            .position(|&b| b == b'\n' || b == 0)
            .unwrap_or(rest.len());
        if let Ok(list) = std::str::from_utf8(&rest[..end]) {
            for name in list.split(',') {
                let n = name.trim();
                if !n.is_empty() && !exports.iter().any(|e| e == n) {
                    exports.push(n.to_string());
                }
            }
        }
    }
    exports
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}
