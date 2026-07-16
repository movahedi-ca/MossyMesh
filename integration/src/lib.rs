//! MossyMesh cross-crate integration smoke harness.
//!
//! Run: `cargo test -p integration`
//! Optional: `cargo test -p integration --features transport`
//!
//! See `docs/integration-smoke-plan.md`.

/// SMK-01 style bootstrap: crate inits must remain panic-free.
pub fn smoke_bootstrap() {
    consensus::init_consensus();
    engine::init_engine();
    sandbox::init_sandbox();
    interop::init_interop();
    #[cfg(feature = "transport")]
    mesh_transport::init_mesh_transport();
}

/// Logical job pipeline used by SMK-06 (stub-level).
pub fn stub_job_pipeline(module_bytes: Vec<u8>) -> Result<Vec<u8>, String> {
    let req = interop::AsyncApiRequest {
        endpoint: "/api/v1/submit_job".to_string(),
        payload: "startpos".to_string(),
    };
    interop::handle_rest_call(&req).map_err(|_| "interop rejected job".to_string())?;

    let instance = sandbox::WamrInstance::new(module_bytes);
    instance
        .invoke_wasm_function("get_best_move", &[])
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smoke_inits() {
        smoke_bootstrap();
    }

    #[test]
    fn smoke_sandbox_mem_cap() {
        let mut inst = sandbox::WamrInstance::new(vec![0x00]);
        let half = sandbox::MEM_LIMIT / 2;
        assert!(inst.allocate(half).is_ok());
        assert!(inst.allocate(half).is_ok());
        assert!(inst.allocate(1).is_err());
    }

    #[test]
    fn smoke_sandbox_mem_constant() {
        assert_eq!(sandbox::MEM_LIMIT, 10 * 1024 * 1024);
    }

    #[test]
    fn smoke_engine_startpos() {
        let eng = engine::EngineState::new();
        assert_eq!(eng.get_moves().len(), 20);
    }

    #[test]
    fn smoke_consensus_insert_merge() {
        let mut a = consensus::TrieNode::new();
        let mut b = consensus::TrieNode::new();
        a.insert_node(&[0x01, 0x02], b"alpha".to_vec());
        b.insert_node(&[0x01, 0x03], b"beta".to_vec());
        a.merge_state(&b);
        assert!(a.children.contains_key(&0x01));
    }

    #[test]
    fn smoke_ledger_bound_constant() {
        assert_eq!(consensus::MAX_LEDGER_SIZE, 10_000_000);
    }

    #[test]
    fn smoke_interop_health() {
        let req = interop::AsyncApiRequest {
            endpoint: "/api/v1/health".to_string(),
            payload: String::new(),
        };
        let resp = interop::handle_rest_call(&req).expect("health");
        assert!(resp.to_lowercase().contains("mesh") || !resp.is_empty());
    }

    #[test]
    fn smoke_job_pipeline_stub() {
        let out = stub_job_pipeline(vec![0x00, 0x61, 0x73, 0x6d]).expect("pipeline");
        assert!(!out.is_empty());
    }
}
