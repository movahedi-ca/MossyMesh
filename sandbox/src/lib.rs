//! # MossyMesh Execution Sandbox
//!
//! WAMR/WASI-oriented isolation for deterministic guest work (chess engine AI,
//! edge tensor probes) under a hard **10 MB** RAM ceiling.
//!
//! ## Guarantees
//! - [`MEM_LIMIT`] is an unyielding host-side boundary; pool growth never exceeds it
//!   (`FixedBlockPool` rejects ceilings above [`MEM_LIMIT`] and OOMs deterministically).
//! - Memory is served from a **fixed-block** pool ([`pool::FixedBlockPool`]); block
//!   size is configurable per job.
//! - **Job admit gate** ([`admit`]): require a verified [`admit::VdfReceipt`] /
//!   [`admit::JobDid`] before trusted invoke ([`job::Job::admit_and_load`],
//!   [`job::Job::invoke_admitted`]). Full MinRoot is owned by transport; the
//!   sandbox ships a domain-separated hash PoW **stub** for local tests.
//! - **Symmetric Static INT8** helpers ([`quant`]) prepare tensor payloads for edge AI.
//! - **Job API** ([`job::Job`]): load module bytes → invoke exports → deterministic
//!   errors on OOM / missing exports.
//! - **Default runtime** is a pure-Rust host simulation ([`host::HostRuntime`]) so
//!   `cargo test -p sandbox` works without native WAMR.
//! - Optional feature **`wamr`**: documents the FFI surface for real WAMR init
//!   (`wasm_runtime_full_init`, bounded aux stack). Enabling it does not link
//!   system libraries and does not break default builds.
//!
//! DOC 46: Isolates guest logic inside a strict WebAssembly-shaped container to
//! prevent rogue host operations.

#![deny(unsafe_code)]

pub mod admit;
pub mod host;
pub mod job;
pub mod pool;
pub mod quant;

#[cfg(feature = "wamr")]
pub mod wamr;

// Re-exports for a compact public surface.
pub use admit::{
    admit_job, AdmitError, DomainSeparatedHashVdfStub, JobDid, VdfReceipt, VdfVerifier,
    HASH_VDF_STUB_DEFAULT_MIN_STEPS, HASH_VDF_STUB_DOMAIN,
};
pub use host::{HostError, HostRuntime, AUX_STACK_SIZE};
pub use job::{Job, JobError};
pub use pool::{BlockHandle, FixedBlockPool, PoolError};
pub use quant::{
    dequantize_symmetric, dequantize_value, max_roundtrip_error, quantize_symmetric,
    quantize_tensor, quantize_value, QuantError, SymmetricInt8Params, INT8_ABS_MAX,
};

/// DOC 47: The MEM_LIMIT is an unyielding boundary. Any execution crossing 10MB
/// triggers a deterministic out-of-memory fault (WASM trap equivalent).
///
/// Value: `10 * 1024 * 1024` (10 MiB). All guest heap paths allocate only through
/// [`FixedBlockPool`], which refuses ceilings above this constant.
pub const MEM_LIMIT: usize = 10 * 1024 * 1024; // 10 MiB

/// Default fixed-block size for guest heaps (4 KiB pages).
pub const DEFAULT_BLOCK_SIZE: usize = 4096;

/// Initialize the sandbox environment (host simulation by default).
///
/// With `--features wamr`, also exercises the feature-gated init hook which
/// currently falls back to simulation unless native WAMR is linked.
pub fn init_sandbox() {
    #[cfg(feature = "wamr")]
    {
        let status = wamr::wasm_runtime_full_init(wamr::WamrInitConfig::default());
        let _ = status;
    }
    let _ = MEM_LIMIT;
    let _ = DEFAULT_BLOCK_SIZE;
}

/// Legacy / compatibility instance wrapping the fixed-block pool + invoke path.
///
/// Prefer [`Job`] for new code. This type preserves the Phase-1 API used by
/// transport stubs while routing allocations through the capped pool.
pub struct WamrInstance {
    job: Job,
    /// DOC 48: Linear guest heap view — length equals currently used pool bytes
    /// (block-aligned). Retained for callers that inspect `memory` directly.
    pub memory: Vec<u8>,
    pub allocated_bytes: usize,
    pub module_bytes: Vec<u8>,
}

impl WamrInstance {
    pub fn new(module_bytes: Vec<u8>) -> Self {
        let bytes = if module_bytes.is_empty() {
            // Keep legacy constructor infallible: empty modules become a minimal marker.
            b"\0asm".to_vec()
        } else {
            module_bytes
        };
        let job = Job::load(bytes.clone()).expect("default MEM_LIMIT pool must construct");
        WamrInstance {
            job,
            memory: Vec::new(),
            allocated_bytes: 0,
            module_bytes: bytes,
        }
    }

    /// Build an instance only after the VDF / Job DID admit gate succeeds.
    pub fn admit_new(
        receipt: &VdfReceipt,
        verifier: &impl VdfVerifier,
        module_bytes: Vec<u8>,
    ) -> Result<Self, JobError> {
        let bytes = if module_bytes.is_empty() {
            return Err(JobError::InvalidModule);
        } else {
            module_bytes
        };
        let job = Job::admit_and_load(receipt, verifier, bytes.clone())?;
        Ok(WamrInstance {
            job,
            memory: Vec::new(),
            allocated_bytes: 0,
            module_bytes: bytes,
        })
    }

    /// Deterministic allocator enforcing the 10MB limit via the fixed-block pool.
    /// DOC 49/50: returns a deterministic `Err` so all network nodes agree on OOM.
    pub fn allocate(&mut self, size: usize) -> Result<usize, &'static str> {
        match self.job.allocate(size) {
            Ok(ptr) => {
                self.allocated_bytes = self.job.used_memory();
                self.memory.resize(self.allocated_bytes, 0);
                Ok(ptr)
            }
            Err(e) => Err(job_error_static(&e)),
        }
    }

    /// Simulates the FFI boundary: invoke exported guest functions by name.
    /// DOC 52: Host maintains authority over guest execution.
    pub fn invoke_wasm_function(
        &mut self,
        func_name: &str,
        args: &[u8],
    ) -> Result<Vec<u8>, &'static str> {
        self.job
            .invoke(func_name, args)
            .map_err(|e| job_error_static(&e))
    }

    /// Invoke only when this instance was admitted (Job DID bound).
    pub fn invoke_admitted(
        &mut self,
        func_name: &str,
        args: &[u8],
    ) -> Result<Vec<u8>, &'static str> {
        self.job
            .invoke_admitted(func_name, args)
            .map_err(|e| job_error_static(&e))
    }

    pub fn is_admitted(&self) -> bool {
        self.job.is_admitted()
    }

    pub fn job_did(&self) -> Option<JobDid> {
        self.job.job_did()
    }
}

fn job_error_static(e: &JobError) -> &'static str {
    match e {
        JobError::OutOfMemory => "Allocation failed: 10MB memory limit exceeded.",
        JobError::ExportNotFound => "FFI Error: Exported function not found in WASM module.",
        JobError::InvalidModule => "Host Error: Invalid or empty WASM module bytes.",
        JobError::AuxStackOverflow => "Host Error: Bounded aux stack overflow.",
        JobError::NotAdmitted => "Admit denied: job has no verified Job DID.",
        JobError::Admit(AdmitError::InvalidVdf) => {
            "Admit denied: VDF receipt verification failed."
        }
        JobError::Admit(AdmitError::DidMismatch) => {
            "Admit denied: Job DID does not match VDF receipt."
        }
        JobError::Admit(AdmitError::InsufficientSteps) => {
            "Admit denied: VDF steps below minimum delay."
        }
        JobError::Admit(AdmitError::Rejected(_)) => "Admit denied: VDF receipt rejected.",
        JobError::Runtime(_) => "Host Error: runtime fault.",
    }
}

/// Placeholder retained for mesh-transport stubs.
pub fn load_wasm() {}

/// Placeholder retained for mesh-transport stubs.
pub fn execute_wasm() {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quant::{dequantize_symmetric, quantize_tensor, INT8_ABS_MAX};

    #[test]
    fn mem_limit_is_10mb() {
        assert_eq!(MEM_LIMIT, 10 * 1024 * 1024);
    }

    #[test]
    fn successful_allocate_under_limit() {
        let mut pool = FixedBlockPool::with_limit(1024, 16 * 1024).unwrap();
        let h = pool.allocate(4000).unwrap();
        assert!(h.byte_len(1024) >= 4000);
        assert!(pool.used_bytes() <= pool.capacity_bytes());
        assert!(pool.used_bytes() < MEM_LIMIT);
    }

    #[test]
    fn oom_at_limit() {
        // Tiny arena: two 64-byte blocks max.
        let mut pool = FixedBlockPool::with_limit(64, 128).unwrap();
        assert!(pool.allocate(64).is_ok());
        assert!(pool.allocate(64).is_ok());
        let err = pool.allocate(1).unwrap_err();
        assert_eq!(err, PoolError::OutOfMemory);
        assert_eq!(err.as_str(), "Allocation failed: 10MB memory limit exceeded.");
    }

    #[test]
    fn pool_rejects_over_global_mem_limit() {
        assert_eq!(
            FixedBlockPool::with_limit(4096, MEM_LIMIT + 1).unwrap_err(),
            PoolError::LimitExceedsGlobal
        );
    }

    #[test]
    fn job_oom_at_limit_via_allocate() {
        let mut job = Job::load_with_config(b"\0asm".to_vec(), 64, 128).unwrap();
        assert!(job.allocate(64).is_ok());
        assert!(job.allocate(64).is_ok());
        let err = job.allocate(1).unwrap_err();
        assert_eq!(err, JobError::OutOfMemory);
        assert_eq!(
            err.as_str(),
            "Allocation failed: 10MB memory limit exceeded."
        );
    }

    #[test]
    fn job_allocate_success_under_limit() {
        let mut job = Job::load(b"\0asm-mossy").unwrap();
        let ptr = job.allocate(8192).unwrap();
        // First free block starts at offset 0 with empty pool.
        assert_eq!(ptr, 0);
        assert!(job.used_memory() >= 8192);
        assert!(job.used_memory() <= MEM_LIMIT);
    }

    #[test]
    fn invoke_unknown_export_fails() {
        let mut job = Job::load(b"\0asm").unwrap();
        let err = job.invoke("not_a_real_export", &[]).unwrap_err();
        assert_eq!(err, JobError::ExportNotFound);
        assert_eq!(
            err.as_str(),
            "FFI Error: Exported function not found in WASM module."
        );
    }

    #[test]
    fn invoke_known_export_succeeds() {
        let mut job = Job::load(b"\0asm").unwrap();
        let out = job.invoke("get_best_move", &[]).unwrap();
        assert_eq!(out, vec![0xE2, 0xE4]);
        let eval = job.invoke("evaluate_move", &[1, 2, 3]).unwrap();
        assert_eq!(eval, vec![0x01]);
    }

    #[test]
    fn int8_quant_roundtrip_bounds() {
        let data: Vec<f32> = (-50..=50).map(|i| i as f32 * 0.07).collect();
        let (q, params) = quantize_tensor(&data).unwrap();
        assert!(q
            .iter()
            .all(|&v| (-INT8_ABS_MAX..=INT8_ABS_MAX).contains(&v)));
        let recon = dequantize_symmetric(&q, &params);
        let bound = max_roundtrip_error(&params) + 1e-5;
        for (orig, got) in data.iter().zip(recon.iter()) {
            assert!(
                (orig - got).abs() <= bound,
                "value {orig} recon {got} err {} > bound {bound}",
                (orig - got).abs()
            );
        }
        // Scale itself must be positive and finite.
        assert!(params.scale.is_finite() && params.scale > 0.0);
    }

    #[test]
    fn wamr_instance_allocate_and_oom() {
        let mut inst = WamrInstance::new(b"\0asm".to_vec());
        let p = inst.allocate(4096).unwrap();
        assert_eq!(p, 0);
        assert!(inst.allocated_bytes >= 4096);

        // Exhaust by asking for more than remaining capacity.
        let remaining_plus = MEM_LIMIT;
        let err = inst.allocate(remaining_plus).unwrap_err();
        assert_eq!(err, "Allocation failed: 10MB memory limit exceeded.");
    }

    #[test]
    fn wamr_instance_unknown_export() {
        let mut inst = WamrInstance::new(b"\0asm".to_vec());
        let err = inst.invoke_wasm_function("nope", &[]).unwrap_err();
        assert_eq!(err, "FFI Error: Exported function not found in WASM module.");
    }

    #[test]
    fn wamr_instance_admit_gate() {
        let stub = DomainSeparatedHashVdfStub::new(8);
        let receipt = stub.issue(99, 16, 7, b"wamr-admit");
        let mut inst = WamrInstance::admit_new(&receipt, &stub, b"\0asm".to_vec()).unwrap();
        assert!(inst.is_admitted());
        assert_eq!(inst.job_did(), Some(receipt.job_did));
        let out = inst.invoke_admitted("get_best_move", &[]).unwrap();
        assert_eq!(out, vec![0xE2, 0xE4]);

        // Unadmitted legacy instance cannot use invoke_admitted.
        let mut legacy = WamrInstance::new(b"\0asm".to_vec());
        assert!(!legacy.is_admitted());
        let err = legacy.invoke_admitted("get_best_move", &[]).unwrap_err();
        assert_eq!(err, "Admit denied: job has no verified Job DID.");
    }

    #[test]
    fn custom_export_marker_is_discovered() {
        let mut module = b"\0asm\nMOSSYMESH_EXPORTS:custom_fn,another\n".to_vec();
        module.extend_from_slice(&[0, 1, 2]);
        let job = Job::load(module).unwrap();
        assert!(job.exports().iter().any(|e| e == "custom_fn"));
        assert!(job.exports().iter().any(|e| e == "another"));
    }

    #[test]
    fn empty_module_rejected_by_job() {
        let err = Job::load(Vec::<u8>::new()).unwrap_err();
        assert_eq!(err, JobError::InvalidModule);
    }

    #[test]
    fn init_sandbox_smoke() {
        init_sandbox();
    }
}
