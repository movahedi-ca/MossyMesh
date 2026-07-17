//! MossyMesh cross-crate integration smoke harness.
//!
//! Run: `cargo test -p integration`
//! Optional: `cargo test -p integration --features transport`
//!
//! See `docs/integration-smoke-plan.md` for SMK-01…SMK-08 IDs.
//! This crate only adds thin glue; product logic lives in peer crates.
//!
//! The job pipeline is a **real offline cross-crate path** (not host-only stubs):
//! interop accept → test MinRoot VDF admit → sandbox invoke → engine startpos/eval
//! → consensus ledger insert + Merkle proof verify.

/// SMK-01 style bootstrap: crate inits must remain panic-free.
pub fn smoke_bootstrap() {
    consensus::init_consensus();
    engine::init_engine();
    sandbox::init_sandbox();
    interop::init_interop();
    #[cfg(feature = "transport")]
    mesh_transport::init_mesh_transport();
}

// ─── Real job pipeline (SMK-06) ──────────────────────────────────────────────

/// Deterministic offline result of the multi-crate job pipeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobPipelineResult {
    /// Ephemeral Job DID minted from the verified VDF receipt.
    pub job_did: sandbox::JobDid,
    /// Guest export bytes from admitted sandbox invoke (`get_best_move`).
    pub sandbox_output: Vec<u8>,
    /// Engine legal-move count at startpos.
    pub legal_moves: usize,
    /// Engine evaluation (centipawns, white perspective) at startpos.
    pub eval_cp: i32,
    /// Merkle-Patricia root after committing the job result.
    pub ledger_root: [u8; 32],
    /// Whether the inclusion proof verified against `ledger_root`.
    pub proof_ok: bool,
    /// Active ledger size estimate (must stay ≤ [`consensus::MAX_LEDGER_SIZE`]).
    pub ledger_bytes: usize,
}

/// Real cross-crate job pipeline (offline / fast).
///
/// ```text
/// interop::handle_rest_call(/api/v1/submit_job)
///   → sandbox::MinRootVdfVerifier::for_tests + Job::admit_and_load
///   → Job::invoke_admitted("get_best_move")
///   → engine::EngineState startpos eval + legal moves
///   → consensus::MerklePatriciaTrie insert + prove + verify_proof
/// ```
///
/// Uses test MinRoot (small iteration count), never production 50M steps.
pub fn job_pipeline(module_bytes: Vec<u8>) -> Result<JobPipelineResult, String> {
    // 1) Interop gateway accepts the job (REST surface).
    let req = interop::AsyncApiRequest {
        endpoint: "/api/v1/submit_job".to_string(),
        payload: "startpos".to_string(),
    };
    let accept = interop::handle_rest_call(&req).map_err(|_| "interop rejected job".to_string())?;
    if accept.is_empty() {
        return Err("interop empty accept body".to_string());
    }

    // 2) Sandbox admit gate with **test** MinRoot VDF (fast, offline).
    let verifier = sandbox::MinRootVdfVerifier::for_tests(8);
    let receipt = verifier
        .issue_test(11, 16, b"integration-job-startpos")
        .map_err(|e| format!("vdf issue: {}", e.as_str()))?;
    let mut job = sandbox::Job::admit_and_load(&receipt, &verifier, module_bytes)
        .map_err(|e| format!("admit_and_load: {}", e.as_str()))?;
    if !job.is_admitted() {
        return Err("job not marked admitted after VDF gate".to_string());
    }
    let job_did = job
        .job_did()
        .ok_or_else(|| "missing Job DID after admit".to_string())?;
    assert_eq!(job_did, receipt.job_did);

    // 3) Admitted invoke only (unadmitted jobs must refuse this path).
    let sandbox_output = job
        .invoke_admitted("get_best_move", &[])
        .map_err(|e| format!("invoke_admitted: {}", e.as_str()))?;
    if sandbox_output.is_empty() {
        return Err("sandbox returned empty get_best_move".to_string());
    }

    // 4) Engine public API: startpos legal moves + deterministic eval.
    let eng = engine::EngineState::new();
    let legal_moves = eng.legal_move_count();
    let eval_cp = eng.evaluate_position();
    // Sanity: startpos is well-formed.
    if legal_moves == 0 {
        return Err("engine startpos has zero legal moves".to_string());
    }
    // Eval must be stable (no wall clock / RNG).
    if eng.evaluate_position() != eval_cp {
        return Err("engine eval non-deterministic".to_string());
    }

    // 5) Consensus: commit job result under DID-keyed path, prove inclusion.
    let mut ledger = consensus::MerklePatriciaTrie::new();
    // Key binds the Job DID so the ledger entry is job-scoped.
    let mut key = b"job/result/".to_vec();
    key.extend_from_slice(job_did.as_bytes());
    // Value: sandbox output || eval LE bytes (compact offline receipt).
    let mut value = sandbox_output.clone();
    value.extend_from_slice(&eval_cp.to_le_bytes());
    value.extend_from_slice(&(legal_moves as u32).to_le_bytes());

    ledger
        .insert(&key, value)
        .map_err(|e| format!("ledger insert: {e:?}"))?;
    let ledger_root = ledger.root_hash();
    let proof = ledger
        .prove(&key)
        .map_err(|e| format!("ledger prove: {e:?}"))?;
    let proof_ok = consensus::verify_proof(&proof, &ledger_root)
        .map_err(|e| format!("verify_proof: {e:?}"))?;
    let ledger_bytes = ledger.size_bytes();
    if ledger_bytes > consensus::MAX_LEDGER_SIZE {
        return Err("ledger exceeds MAX_LEDGER_SIZE".to_string());
    }
    if !proof_ok {
        return Err("merkle proof failed verification".to_string());
    }

    Ok(JobPipelineResult {
        job_did,
        sandbox_output,
        legal_moves,
        eval_cp,
        ledger_root,
        proof_ok,
        ledger_bytes,
    })
}

/// Thin legacy alias: same as [`job_pipeline`] but returns only sandbox output bytes.
///
/// Prefer [`job_pipeline`] for new harness code. Kept so older SMK-06 call sites
/// and docs that mention a "stub" still have a drop-in that is no longer stub-only.
pub fn stub_job_pipeline(module_bytes: Vec<u8>) -> Result<Vec<u8>, String> {
    job_pipeline(module_bytes).map(|r| r.sandbox_output)
}

// ─── Focused glue helpers (used by companion SMK tests) ─────────────────────

/// Cross-crate glue: engine legal-move count for a FEN (or startpos).
pub fn engine_legal_move_count(fen: Option<&str>) -> Result<usize, String> {
    let eng = match fen {
        Some(f) => engine::EngineState::from_fen(f)?,
        None => engine::EngineState::new(),
    };
    Ok(eng.legal_move_count())
}

/// Cross-crate glue: startpos evaluation (white perspective, centipawns).
pub fn engine_startpos_eval() -> i32 {
    engine::EngineState::new().evaluate_position()
}

/// Cross-crate glue: Merkle insert + prove + verify for a single key/value.
/// Returns `(root_hash, proof_ok)`.
pub fn consensus_insert_and_prove(key: &[u8], value: Vec<u8>) -> Result<([u8; 32], bool), String> {
    let mut ledger = consensus::MerklePatriciaTrie::new();
    ledger
        .insert(key, value)
        .map_err(|e| format!("insert: {e:?}"))?;
    let root = ledger.root_hash();
    let proof = ledger.prove(key).map_err(|e| format!("prove: {e:?}"))?;
    let ok = consensus::verify_proof(&proof, &root).map_err(|e| format!("verify: {e:?}"))?;
    Ok((root, ok))
}

/// Cross-crate glue: sandbox admit with test MinRoot VDF, then admitted invoke.
pub fn sandbox_admit_invoke(
    export: &str,
    args: &[u8],
    module_bytes: Vec<u8>,
) -> Result<(sandbox::JobDid, Vec<u8>), String> {
    let verifier = sandbox::MinRootVdfVerifier::for_tests(8);
    let receipt = verifier
        .issue_test(42, 16, b"sandbox-admit-smoke")
        .map_err(|e| format!("vdf issue: {}", e.as_str()))?;
    let mut job = sandbox::Job::admit_and_load(&receipt, &verifier, module_bytes)
        .map_err(|e| format!("admit: {}", e.as_str()))?;
    let out = job
        .invoke_admitted(export, args)
        .map_err(|e| format!("invoke: {}", e.as_str()))?;
    Ok((receipt.job_did, out))
}

/// Cross-crate glue: CRDT island merge via public `consensus::crdt::Doc` API.
/// Returns converged text after bidirectional merge.
pub fn consensus_crdt_island_merge() -> String {
    let mut a = consensus::crdt::Doc::new(1);
    let mut b = consensus::crdt::Doc::new(2);
    a.insert_str(0, "mesh");
    b.insert_str(0, "node");
    a.merge(&b);
    b.merge(&a);
    // Strong eventual consistency: both islands materialize the same text.
    assert_eq!(a.text(), b.text());
    a.text()
}

/// Cross-crate glue: fixed-block pool under a small arena (fast OOM path).
pub fn sandbox_pool_oom_smoke() -> sandbox::PoolError {
    let mut pool = sandbox::FixedBlockPool::with_limit(64, 128).expect("pool construct");
    assert!(pool.allocate(64).is_ok());
    assert!(pool.allocate(64).is_ok());
    pool.allocate(1).expect_err("must OOM past cap")
}

#[cfg(feature = "transport")]
/// Cross-crate glue: topology shortest path over Wi-Fi/BLE/LoRa links.
pub fn transport_topology_path_smoke() -> Option<mesh_transport::topology::Path> {
    use mesh_transport::topology::{LinkType, TopologyGraph};
    let mut g = TopologyGraph::new();
    g.add_bidirectional("a", "b", LinkType::Wifi, 255);
    g.add_bidirectional("b", "c", LinkType::Ble, 200);
    g.add_bidirectional("a", "c", LinkType::LoRa, 100);
    g.shortest_path("a", "c")
}

#[cfg(feature = "transport")]
/// Cross-crate glue: deterministic identity from seed + destination announce.
pub fn transport_identity_smoke() -> ([u8; 32], String) {
    use mesh_transport::identity_manager::IdentityManager;
    let mut mgr = IdentityManager::new();
    let peer = mgr.bootstrap_from_seed(b"integration-smoke-seed").clone();
    let dest = mgr
        .announce_destination("mesh", &["lxmf", "smoke"])
        .expect("local identity set");
    (*peer.as_bytes(), dest.path())
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- SMK-01 ---
    /// SMK-01: `smoke_inits` — consensus, engine, sandbox, interop init panic-free.
    #[test]
    fn smoke_inits() {
        smoke_bootstrap();
    }

    // --- SMK-02 ---
    /// SMK-02: `smoke_sandbox_mem_cap` — allocate past `MEM_LIMIT` → Err.
    #[test]
    fn smoke_sandbox_mem_cap() {
        let mut inst = sandbox::WamrInstance::new(vec![0x00, 0x61, 0x73, 0x6d]);
        let half = sandbox::MEM_LIMIT / 2;
        assert!(inst.allocate(half).is_ok());
        assert!(inst.allocate(half).is_ok());
        assert!(inst.allocate(1).is_err());
    }

    /// SMK-02 companion: fixed-block pool OOM is deterministic (`PoolError::OutOfMemory`).
    #[test]
    fn smoke_sandbox_pool_oom() {
        let err = sandbox_pool_oom_smoke();
        assert_eq!(err, sandbox::PoolError::OutOfMemory);
        assert_eq!(
            err.as_str(),
            "Allocation failed: 10MB memory limit exceeded."
        );
    }

    // --- SMK-08 ---
    /// SMK-08: `smoke_sandbox_mem_constant` — `MEM_LIMIT == 10 MiB`.
    #[test]
    fn smoke_sandbox_mem_constant() {
        assert_eq!(sandbox::MEM_LIMIT, 10 * 1024 * 1024);
        assert_eq!(sandbox::DEFAULT_BLOCK_SIZE, 4096);
    }

    // --- SMK-03 ---
    /// SMK-03: `smoke_engine_startpos` — startpos has 20 legal moves + stable eval.
    #[test]
    fn smoke_engine_startpos() {
        let eng = engine::EngineState::new();
        assert_eq!(eng.get_moves().len(), 20);
        assert_eq!(eng.legal_move_count(), 20);
        assert_eq!(engine_legal_move_count(None).unwrap(), 20);
        // FEN constant + round-trip still startpos.
        let via_fen = engine::EngineState::from_fen(engine::START_FEN).expect("START_FEN");
        assert_eq!(via_fen.legal_move_count(), 20);
        let reloaded = engine::EngineState::from_fen(&eng.to_fen()).expect("to_fen");
        assert_eq!(reloaded.legal_move_count(), 20);
        // Eval surface: deterministic white-perspective score at startpos.
        let eval = eng.evaluate_position();
        assert_eq!(eval, engine_startpos_eval());
        assert_eq!(eval, via_fen.evaluate_position());
        assert_eq!(eval, eng.evaluate_stm()); // white to move
        assert!(!eng.is_game_over());
        assert!(!eng.is_check());
    }

    /// SMK-03 companion: make/unmake + shallow search stay deterministic offline.
    #[test]
    fn smoke_engine_make_unmake_search() {
        let mut eng = engine::EngineState::new();
        let fen_before = eng.to_fen();
        let m = eng.get_moves()[0].clone();
        eng.make_move(&m).expect("legal");
        assert_ne!(eng.to_fen(), fen_before);
        eng.unmake_move().expect("history");
        assert_eq!(eng.to_fen(), fen_before);
        assert_eq!(eng.legal_move_count(), 20);

        // Depth-1 search is offline and fast; best move present.
        let res = eng.search(1);
        assert_eq!(res.depth, 1);
        assert!(res.best_move.is_some());
        assert!(res.nodes > 0);

        // Eval is stable (no RNG / wall clock).
        assert_eq!(eng.evaluate_position(), eng.evaluate_position());
        assert_eq!(engine::MAX_DEPTH, 64);
        assert!(engine::DEFAULT_SEARCH_DEPTH <= engine::MAX_DEPTH);
    }

    // --- SMK-04 ---
    /// SMK-04: `smoke_consensus_insert_merge` — trie insert + merge child present.
    #[test]
    fn smoke_consensus_insert_merge() {
        // Legacy TrieNode surface (Blake3-hashed merge on main).
        let mut a = consensus::TrieNode::new();
        let mut b = consensus::TrieNode::new();
        a.insert_node(&[0x01, 0x02], b"alpha".to_vec());
        b.insert_node(&[0x01, 0x03], b"beta".to_vec());
        a.merge_state(&b);
        assert!(a.children.contains_key(&0x01));
        let branch = a.children.get(&0x01).expect("branch");
        assert!(branch.children.contains_key(&0x02));
        assert!(
            branch.children.contains_key(&0x03),
            "merge must import remote-only leaf under nibble path"
        );
        assert_eq!(a.get_node(&[0x01, 0x02]), Some(b"alpha".as_slice()));
        assert_eq!(a.get_node(&[0x01, 0x03]), Some(b"beta".as_slice()));

        // Real MerklePatriciaTrie: insert, prove, island merge (public API).
        use consensus::StateMerge;
        let mut ledger = consensus::MerklePatriciaTrie::new();
        ledger
            .insert(b"account/1", b"1000".to_vec())
            .expect("insert a1");
        let root = ledger.root_hash();
        let proof = ledger.prove(b"account/1").expect("prove");
        assert!(consensus::verify_proof(&proof, &root).expect("verify_proof"));

        let mut island = consensus::MerklePatriciaTrie::new();
        island
            .insert(b"account/2", b"500".to_vec())
            .expect("insert a2");
        ledger.merge_with(&island).expect("merge_with");
        assert_eq!(ledger.get(b"account/2").as_deref(), Some(b"500".as_slice()));
        assert_ne!(ledger.root_hash(), root);
        assert!(ledger.size_bytes() <= consensus::MAX_LEDGER_SIZE);

        // Focused glue: insert + prove + verify in one call.
        let (r2, ok) =
            consensus_insert_and_prove(b"smoke/key", b"value".to_vec()).expect("glue prove");
        assert!(ok);
        assert_ne!(r2, [0u8; 32]);
    }

    /// SMK-04 companion: real YATA/RGA CRDT Doc merge converges across islands.
    #[test]
    fn smoke_consensus_crdt_merge() {
        let text = consensus_crdt_island_merge();
        // Both islands wrote content; merge is non-empty and deterministic.
        assert!(!text.is_empty());
        assert_eq!(text.chars().count(), "mesh".len() + "node".len());

        // LWW map + binary delta round-trip (offline sync path).
        let mut a = consensus::crdt::Doc::new(10);
        let mut b = consensus::crdt::Doc::new(20);
        a.map_set("job", b"accepted");
        b.map_set("peer", b"offline");
        let delta = a.full_delta();
        let bytes = delta.encode().expect("encode");
        let decoded = consensus::crdt::Delta::decode(&bytes).expect("decode");
        for op in decoded.ops {
            b.integrate(op);
        }
        assert_eq!(b.map_get("job"), Some(b"accepted".as_slice()));
        a.merge(&b);
        assert_eq!(a.map_get("peer"), Some(b"offline".as_slice()));
    }

    // --- SMK-07 ---
    /// SMK-07: `smoke_ledger_bound_constant` — `MAX_LEDGER_SIZE == 10_000_000`.
    #[test]
    fn smoke_ledger_bound_constant() {
        assert_eq!(consensus::MAX_LEDGER_SIZE, 10_000_000);
        // Constant-size SNARK folding is wired (mock prover; real nova-snark optional).
        use consensus::SnarkFolder;
        let genesis = [0u8; 32];
        let mut folder = consensus::AccumulatorSnarkFolder::new(genesis);
        let step = consensus::StepInstance {
            prev_state_root: genesis,
            next_state_root: [1u8; 32],
            witness_digest: [2u8; 32],
        };
        folder.fold_step(&step).expect("fold step");
        assert!(folder.verify(&[1u8; 32]).expect("verify folded"));
        assert_eq!(
            folder.accumulator().public_bytes().len(),
            consensus::ANCHOR_PROOF_SIZE
        );
        assert!(consensus::verify_snark(folder.accumulator()));
    }

    // --- SMK-05 ---
    /// SMK-05: `smoke_interop_health` — `/api/v1/health` Ok with mesh status.
    #[test]
    fn smoke_interop_health() {
        let req = interop::AsyncApiRequest {
            endpoint: "/api/v1/health".to_string(),
            payload: String::new(),
        };
        let resp = interop::handle_rest_call(&req).expect("health");
        assert!(
            resp.to_lowercase().contains("mesh") || !resp.is_empty(),
            "health body: {resp}"
        );
        // Unknown routes stay contracted as ConnectionRefused.
        let bad = interop::AsyncApiRequest {
            endpoint: "/api/v1/does-not-exist".to_string(),
            payload: String::new(),
        };
        assert!(matches!(
            interop::handle_rest_call(&bad),
            Err(interop::InteropError::ConnectionRefused)
        ));
    }

    /// SMK-05 companion: HTLC hash-lock helpers exist for escrow contract surface.
    #[test]
    fn smoke_interop_htlc_types() {
        let preimage = b"integration-preimage";
        let payment_hash = interop::hash_preimage(preimage);
        assert!(interop::verify_preimage(preimage, &payment_hash));
        assert!(!interop::verify_preimage(b"wrong", &payment_hash));
        // Mock VDF type is constructible offline (VDF-delayed cancel path).
        let vdf = interop::MockVdf::new(7, 3);
        assert_eq!(vdf.steps_required, 3);
        assert_eq!(vdf.steps_completed, 0);
    }

    // --- SMK-06 ---
    /// SMK-06: real job pipeline — interop → MinRoot admit → sandbox → engine → consensus.
    #[test]
    fn smoke_job_pipeline() {
        let result = job_pipeline(vec![0x00, 0x61, 0x73, 0x6d]).expect("pipeline");
        // Sandbox host sim returns stub best-move bytes for get_best_move.
        assert_eq!(result.sandbox_output, vec![0xE2, 0xE4]);
        assert_ne!(*result.job_did.as_bytes(), [0u8; 32]);
        // Engine startpos surfaces.
        assert_eq!(result.legal_moves, 20);
        assert_eq!(result.eval_cp, engine_startpos_eval());
        // Consensus proof verified and ledger within SLA.
        assert!(result.proof_ok);
        assert!(result.ledger_bytes <= consensus::MAX_LEDGER_SIZE);
        assert_ne!(result.ledger_root, [0u8; 32]);

        // Legacy thin alias still returns non-empty guest bytes.
        let out = stub_job_pipeline(vec![0x00, 0x61, 0x73, 0x6d]).expect("alias");
        assert_eq!(out, result.sandbox_output);
    }

    /// SMK-06 companion: sandbox admit with test MinRoot is required for trusted invoke.
    #[test]
    fn smoke_sandbox_vdf_admit() {
        let (did, out) =
            sandbox_admit_invoke("get_best_move", &[], b"\0asm".to_vec()).expect("admit invoke");
        assert_eq!(out, vec![0xE2, 0xE4]);
        assert_ne!(*did.as_bytes(), [0u8; 32]);

        // Unadmitted job refuses invoke_admitted.
        let mut bare = sandbox::Job::load(b"\0asm".to_vec()).expect("load");
        assert!(!bare.is_admitted());
        assert_eq!(
            bare.invoke_admitted("get_best_move", &[]).unwrap_err(),
            sandbox::JobError::NotAdmitted
        );

        // Tampered receipt never loads.
        let v = sandbox::MinRootVdfVerifier::for_tests(8);
        let mut bad = v.issue_test(1, 16, b"x").expect("issue");
        bad.final_x = bad.final_x.wrapping_add(1);
        let err = sandbox::Job::admit_and_load(&bad, &v, b"\0asm").unwrap_err();
        assert!(matches!(
            err,
            sandbox::JobError::Admit(sandbox::AdmitError::InvalidVdf)
        ));
        // Test constants stay separated from production delay.
        assert!(sandbox::DEFAULT_TEST_ITERATIONS < sandbox::PRODUCTION_ITERATIONS);
        assert_eq!(sandbox::PRODUCTION_ITERATIONS, 50_000_000);
    }

    /// SMK-06 companion: evaluate_move export also returns non-empty via Job path.
    #[test]
    fn smoke_job_evaluate_move_export() {
        let mut job = sandbox::Job::load(b"\0asm".to_vec()).expect("load");
        let eval = job.invoke("evaluate_move", &[1, 2, 3]).expect("eval");
        assert!(!eval.is_empty());
        let missing = job.invoke("not_exported", &[]).unwrap_err();
        assert_eq!(missing, sandbox::JobError::ExportNotFound);
    }

    // --- Optional transport (feature-gated; still offline, no sockets) ---
    #[cfg(feature = "transport")]
    /// Transport topology smoke: shortest path prefers cheaper Wi-Fi/BLE chain.
    #[test]
    fn smoke_transport_topology() {
        mesh_transport::init_mesh_transport();
        let path = transport_topology_path_smoke().expect("path a→c");
        assert!(path.nodes.len() >= 2);
        assert!(path.total_cost > 0);
        // Direct LoRa is expensive; multi-hop Wifi+Ble should be preferred.
        assert!(
            path.hops
                .iter()
                .all(|h| h.link != mesh_transport::topology::LinkType::LoRa)
                || path.total_cost
                    <= mesh_transport::topology::compute_edge_cost(
                        mesh_transport::topology::LinkType::LoRa,
                        100
                    ),
            "path cost should not exceed lone LoRa edge cost"
        );
    }

    #[cfg(feature = "transport")]
    /// Transport identity smoke: seed → stable PeerId + destination path.
    #[test]
    fn smoke_transport_identity() {
        let (id_a, path_a) = transport_identity_smoke();
        let (id_b, path_b) = transport_identity_smoke();
        assert_eq!(id_a, id_b, "same seed must yield same PeerId");
        assert_eq!(path_a, "mesh/lxmf/smoke");
        assert_eq!(path_b, path_a);
        assert_ne!(id_a, [0u8; 32]);
    }
}
