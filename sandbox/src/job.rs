//! Job execution API: load module bytes, invoke exports, deterministic faults.
//!
//! A [`Job`] is the unit of sandboxed work on the mesh. It owns a
//! [`crate::host::HostRuntime`] (pure-Rust simulation by default) and exposes
//! a small, deterministic surface suitable for VRF-assigned workers.

use crate::host::{HostError, HostRuntime};
use crate::{DEFAULT_BLOCK_SIZE, MEM_LIMIT};
use serde::{Deserialize, Serialize};

/// Errors returned by the job API. Serialized forms stay stable for mesh logs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum JobError {
    OutOfMemory,
    ExportNotFound,
    InvalidModule,
    AuxStackOverflow,
    Runtime(String),
}

impl JobError {
    pub fn as_str(&self) -> &str {
        match self {
            JobError::OutOfMemory => "Allocation failed: 10MB memory limit exceeded.",
            JobError::ExportNotFound => "FFI Error: Exported function not found in WASM module.",
            JobError::InvalidModule => "Host Error: Invalid or empty WASM module bytes.",
            JobError::AuxStackOverflow => "Host Error: Bounded aux stack overflow.",
            JobError::Runtime(s) => s.as_str(),
        }
    }
}

impl core::fmt::Display for JobError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::error::Error for JobError {}

impl From<HostError> for JobError {
    fn from(value: HostError) -> Self {
        match value {
            HostError::OutOfMemory => JobError::OutOfMemory,
            HostError::ExportNotFound => JobError::ExportNotFound,
            HostError::InvalidModule => JobError::InvalidModule,
            HostError::AuxStackOverflow => JobError::AuxStackOverflow,
            HostError::Pool(e) => JobError::Runtime(e.as_str().to_string()),
        }
    }
}

/// A loaded guest job ready for export invocation.
#[derive(Debug)]
pub struct Job {
    runtime: HostRuntime,
}

impl Job {
    /// Load guest module bytes under the global 10 MB cap and default block size.
    pub fn load(module_bytes: impl Into<Vec<u8>>) -> Result<Self, JobError> {
        Self::load_with_config(module_bytes, DEFAULT_BLOCK_SIZE, MEM_LIMIT)
    }

    /// Load with a custom fixed-block size (still capped by `mem_limit`).
    pub fn load_with_block_size(
        module_bytes: impl Into<Vec<u8>>,
        block_size: usize,
    ) -> Result<Self, JobError> {
        Self::load_with_config(module_bytes, block_size, MEM_LIMIT)
    }

    /// Full configuration entry point (tests may lower `mem_limit`).
    pub fn load_with_config(
        module_bytes: impl Into<Vec<u8>>,
        block_size: usize,
        mem_limit: usize,
    ) -> Result<Self, JobError> {
        let runtime = HostRuntime::load_with_config(module_bytes.into(), block_size, mem_limit)?;
        Ok(Self { runtime })
    }

    /// Invoke an exported guest function by name.
    pub fn invoke(&mut self, export: &str, args: &[u8]) -> Result<Vec<u8>, JobError> {
        self.runtime.invoke(export, args).map_err(JobError::from)
    }

    /// Allocate from the job's fixed-block pool (guest-linear offset).
    pub fn allocate(&mut self, size: usize) -> Result<usize, JobError> {
        self.runtime.allocate(size).map_err(JobError::from)
    }

    pub fn used_memory(&self) -> usize {
        self.runtime.used_memory()
    }

    pub fn mem_limit(&self) -> usize {
        self.runtime.mem_limit()
    }

    pub fn block_size(&self) -> usize {
        self.runtime.block_size()
    }

    pub fn exports(&self) -> &[String] {
        self.runtime.exports()
    }

    pub fn module_bytes(&self) -> &[u8] {
        &self.runtime.module_bytes
    }
}
