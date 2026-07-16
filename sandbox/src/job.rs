//! Job execution API: load module bytes, invoke exports, deterministic faults.
//!
//! A [`Job`] is the unit of sandboxed work on the mesh. It owns a
//! [`crate::host::HostRuntime`] (pure-Rust simulation by default) and exposes
//! a small, deterministic surface suitable for VRF-assigned workers.
//!
//! ## Admit gate
//! Production workers should call [`admit_and_load`] (or [`crate::admit::admit_job`]
//! then bind the DID) so guest modules only run after a verified VDF / Job DID
//! receipt. Legacy [`Job::load`] remains for tests and mesh-transport stubs.

use crate::admit::{admit_job, AdmitError, JobDid, VdfReceipt, VdfVerifier};
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
    /// Job was not admitted (missing / failed VDF gate).
    NotAdmitted,
    /// Admit gate rejected the VDF receipt / Job DID.
    Admit(AdmitError),
    Runtime(String),
}

impl JobError {
    pub fn as_str(&self) -> &str {
        match self {
            JobError::OutOfMemory => "Allocation failed: 10MB memory limit exceeded.",
            JobError::ExportNotFound => "FFI Error: Exported function not found in WASM module.",
            JobError::InvalidModule => "Host Error: Invalid or empty WASM module bytes.",
            JobError::AuxStackOverflow => "Host Error: Bounded aux stack overflow.",
            JobError::NotAdmitted => "Admit denied: job has no verified Job DID.",
            JobError::Admit(e) => e.as_str(),
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

impl From<AdmitError> for JobError {
    fn from(value: AdmitError) -> Self {
        JobError::Admit(value)
    }
}

/// A loaded guest job ready for export invocation.
#[derive(Debug)]
pub struct Job {
    runtime: HostRuntime,
    /// Set when the job passed the VDF / Job DID admit gate.
    admitted_did: Option<JobDid>,
}

impl Job {
    /// Load guest module bytes under the global 10 MB cap and default block size.
    ///
    /// Does **not** run the admit gate (legacy / test path). Prefer
    /// [`admit_and_load`] for production workers.
    pub fn load(module_bytes: impl Into<Vec<u8>>) -> Result<Self, JobError> {
        Self::load_with_config(module_bytes, DEFAULT_BLOCK_SIZE, MEM_LIMIT)
    }

    /// Load with a custom fixed-block size (still capped by `mem_limit` ≤ [`MEM_LIMIT`]).
    pub fn load_with_block_size(
        module_bytes: impl Into<Vec<u8>>,
        block_size: usize,
    ) -> Result<Self, JobError> {
        Self::load_with_config(module_bytes, block_size, MEM_LIMIT)
    }

    /// Full configuration entry point (tests may lower `mem_limit`, never above [`MEM_LIMIT`]).
    pub fn load_with_config(
        module_bytes: impl Into<Vec<u8>>,
        block_size: usize,
        mem_limit: usize,
    ) -> Result<Self, JobError> {
        let runtime = HostRuntime::load_with_config(module_bytes.into(), block_size, mem_limit)?;
        Ok(Self {
            runtime,
            admitted_did: None,
        })
    }

    /// Verify a VDF receipt, then load the module under the global MEM_LIMIT pool.
    ///
    /// This is the Phase-2 production entry: no guest work without a Job DID.
    pub fn admit_and_load(
        receipt: &VdfReceipt,
        verifier: &impl VdfVerifier,
        module_bytes: impl Into<Vec<u8>>,
    ) -> Result<Self, JobError> {
        Self::admit_and_load_with_config(receipt, verifier, module_bytes, DEFAULT_BLOCK_SIZE, MEM_LIMIT)
    }

    /// Admit + load with custom block size / mem ceiling (tests).
    pub fn admit_and_load_with_config(
        receipt: &VdfReceipt,
        verifier: &impl VdfVerifier,
        module_bytes: impl Into<Vec<u8>>,
        block_size: usize,
        mem_limit: usize,
    ) -> Result<Self, JobError> {
        let did = admit_job(receipt, verifier)?;
        let mut job = Self::load_with_config(module_bytes, block_size, mem_limit)?;
        job.admitted_did = Some(did);
        Ok(job)
    }

    /// Bind an already-verified Job DID (e.g. transport pre-checked MinRoot).
    pub fn bind_admitted_did(&mut self, did: JobDid) {
        self.admitted_did = Some(did);
    }

    /// `true` when this job carries a verified Job DID from the admit gate.
    pub fn is_admitted(&self) -> bool {
        self.admitted_did.is_some()
    }

    pub fn job_did(&self) -> Option<JobDid> {
        self.admitted_did
    }

    /// Invoke an exported guest function by name.
    pub fn invoke(&mut self, export: &str, args: &[u8]) -> Result<Vec<u8>, JobError> {
        self.runtime.invoke(export, args).map_err(JobError::from)
    }

    /// Invoke only if the job was admitted via VDF / Job DID gate.
    ///
    /// Use this from worker loops that must refuse unaudited guests.
    pub fn invoke_admitted(&mut self, export: &str, args: &[u8]) -> Result<Vec<u8>, JobError> {
        if self.admitted_did.is_none() {
            return Err(JobError::NotAdmitted);
        }
        self.invoke(export, args)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admit::DomainSeparatedHashVdfStub;

    #[test]
    fn admit_and_load_then_invoke() {
        let stub = DomainSeparatedHashVdfStub::new(8);
        let receipt = stub.issue(11, 16, 1, b"chess-eval");
        let mut job = Job::admit_and_load(&receipt, &stub, b"\0asm").unwrap();
        assert!(job.is_admitted());
        assert_eq!(job.job_did(), Some(receipt.job_did));
        let out = job.invoke_admitted("get_best_move", &[]).unwrap();
        assert_eq!(out, vec![0xE2, 0xE4]);
    }

    #[test]
    fn invoke_admitted_rejects_unadmitted_job() {
        let mut job = Job::load(b"\0asm").unwrap();
        assert!(!job.is_admitted());
        assert_eq!(
            job.invoke_admitted("echo", b"x").unwrap_err(),
            JobError::NotAdmitted
        );
        // Legacy invoke still works for stubs / tests.
        assert_eq!(job.invoke("echo", b"x").unwrap(), b"x");
    }

    #[test]
    fn bad_receipt_never_loads_module() {
        let stub = DomainSeparatedHashVdfStub::default();
        let mut receipt = stub.issue(3, 10, 0, b"m");
        receipt.final_x = receipt.final_x.wrapping_add(1);
        let err = Job::admit_and_load(&receipt, &stub, b"\0asm").unwrap_err();
        assert!(matches!(err, JobError::Admit(AdmitError::InvalidVdf)));
    }
}
