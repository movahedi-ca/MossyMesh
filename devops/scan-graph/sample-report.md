# Sample slop report

> Sample generated 2026-10-03 from the real mossymesh repo (rev 86fd0252171be6ea809f62665288e15653ed402b). It shows the report format; nightly runs produce fresh reports under devops/scan-graph/output/.

---

# Slop report

Generated 2026-10-03 21:29 UTC from repo rev `86fd0252171be6ea809f62665288e15653ed402b`.

2157 entities, 6215 edges indexed. 133 findings across 5 detectors.

## Dead / unreferenced functions (88)

| Finding | Location | Confidence | Note |
| --- | --- | --- | --- |
| Unreferenced function `fmt` | ai/src/compute.rs:43 | medium | No incoming call edges. Dynamic dispatch, trait impls, and macro-generated callers can hide real callers, so confirm manually. |
| Unreferenced function `name` | ai/src/compute.rs:86 | medium | No incoming call edges. Dynamic dispatch, trait impls, and macro-generated callers can hide real callers, so confirm manually. |
| Unreferenced function `name` | ai/src/compute.rs:130 | medium | No incoming call edges. Dynamic dispatch, trait impls, and macro-generated callers can hide real callers, so confirm manually. |
| Unreferenced function `name` | ai/src/compute.rs:295 | medium | No incoming call edges. Dynamic dispatch, trait impls, and macro-generated callers can hide real callers, so confirm manually. |
| Unreferenced function `fmt` | ai/src/paged_attention.rs:71 | medium | No incoming call edges. Dynamic dispatch, trait impls, and macro-generated callers can hide real callers, so confirm manually. |
| Unreferenced function `fmt` | ai/src/sitf.rs:96 | medium | No incoming call edges. Dynamic dispatch, trait impls, and macro-generated callers can hide real callers, so confirm manually. |
| Unreferenced function `initialTheme` | captive-portal/src/App.tsx:21 | medium | No incoming call edges. Dynamic dispatch, trait impls, and macro-generated callers can hide real callers, so confirm manually. |
| Unreferenced function `App` | captive-portal/src/App.tsx:31 | medium | No incoming call edges. Dynamic dispatch, trait impls, and macro-generated callers can hide real callers, so confirm manually. |
| Unreferenced function `apply` | consensus/fuzz/fuzz_targets/crdt_merge.rs:26 | medium | No incoming call edges. Dynamic dispatch, trait impls, and macro-generated callers can hide real callers, so confirm manually. |
| Unreferenced function `fmt` | consensus/src/crdt.rs:112 | medium | No incoming call edges. Dynamic dispatch, trait impls, and macro-generated callers can hide real callers, so confirm manually. |
| Unreferenced function `fmt` | consensus/src/erasure.rs:28 | medium | No incoming call edges. Dynamic dispatch, trait impls, and macro-generated callers can hide real callers, so confirm manually. |
| Unreferenced function `fmt` | consensus/src/error.rs:25 | medium | No incoming call edges. Dynamic dispatch, trait impls, and macro-generated callers can hide real callers, so confirm manually. |
| Unreferenced function `accumulator` | consensus/src/lib.rs:84 | medium | No incoming call edges. Dynamic dispatch, trait impls, and macro-generated callers can hide real callers, so confirm manually. |
| Unreferenced function `accumulator` | consensus/src/lib.rs:158 | medium | No incoming call edges. Dynamic dispatch, trait impls, and macro-generated callers can hide real callers, so confirm manually. |
| Unreferenced function `accumulator` | consensus/src/lib.rs:198 | medium | No incoming call edges. Dynamic dispatch, trait impls, and macro-generated callers can hide real callers, so confirm manually. |
| Unreferenced function `backend_name` | consensus/src/verifier.rs:25 | medium | No incoming call edges. Dynamic dispatch, trait impls, and macro-generated callers can hide real callers, so confirm manually. |
| Unreferenced function `backend_name` | consensus/src/verifier.rs:44 | medium | No incoming call edges. Dynamic dispatch, trait impls, and macro-generated callers can hide real callers, so confirm manually. |
| Unreferenced function `backend_name` | consensus/src/verifier.rs:92 | medium | No incoming call edges. Dynamic dispatch, trait impls, and macro-generated callers can hide real callers, so confirm manually. |
| Unreferenced function `paths` | engine/src/tablebase.rs:38 | medium | No incoming call edges. Dynamic dispatch, trait impls, and macro-generated callers can hide real callers, so confirm manually. |
| Unreferenced function `paths` | engine/src/tablebase.rs:68 | medium | No incoming call edges. Dynamic dispatch, trait impls, and macro-generated callers can hide real callers, so confirm manually. |
| Unreferenced function `fmt` | engine/src/tablebase.rs:136 | medium | No incoming call edges. Dynamic dispatch, trait impls, and macro-generated callers can hide real callers, so confirm manually. |
| Unreferenced function `paths` | engine/src/tablebase.rs:215 | medium | No incoming call edges. Dynamic dispatch, trait impls, and macro-generated callers can hide real callers, so confirm manually. |
| Unreferenced function `PortalBody` | frontend/src/App.tsx:21 | medium | No incoming call edges. Dynamic dispatch, trait impls, and macro-generated callers can hide real callers, so confirm manually. |
| Unreferenced function `App` | frontend/src/App.tsx:52 | medium | No incoming call edges. Dynamic dispatch, trait impls, and macro-generated callers can hide real callers, so confirm manually. |
| Unreferenced function `squareName` | frontend/src/components/Chessboard.tsx:15 | medium | No incoming call edges. Dynamic dispatch, trait impls, and macro-generated callers can hide real callers, so confirm manually. |
| Unreferenced function `describeStatus` | frontend/src/components/Chessboard.tsx:19 | medium | No incoming call edges. Dynamic dispatch, trait impls, and macro-generated callers can hide real callers, so confirm manually. |
| Unreferenced function `getDerivedStateFromError` | frontend/src/components/ErrorBoundary.tsx:8 | medium | No incoming call edges. Dynamic dispatch, trait impls, and macro-generated callers can hide real callers, so confirm manually. |
| Unreferenced function `componentDidCatch` | frontend/src/components/ErrorBoundary.tsx:9 | medium | No incoming call edges. Dynamic dispatch, trait impls, and macro-generated callers can hide real callers, so confirm manually. |
| Unreferenced function `render` | frontend/src/components/ErrorBoundary.tsx:12 | medium | No incoming call edges. Dynamic dispatch, trait impls, and macro-generated callers can hide real callers, so confirm manually. |
| Unreferenced function `labelFor` | frontend/src/components/NetworkStatus.tsx:4 | medium | No incoming call edges. Dynamic dispatch, trait impls, and macro-generated callers can hide real callers, so confirm manually. |
| Unreferenced function `health_doc` | interop/src/api_docs.rs:78 | medium | No incoming call edges. Dynamic dispatch, trait impls, and macro-generated callers can hide real callers, so confirm manually. |
| Unreferenced function `submit_job_doc` | interop/src/api_docs.rs:92 | medium | No incoming call edges. Dynamic dispatch, trait impls, and macro-generated callers can hide real callers, so confirm manually. |
| Unreferenced function `liquidity_doc` | interop/src/api_docs.rs:107 | medium | No incoming call edges. Dynamic dispatch, trait impls, and macro-generated callers can hide real callers, so confirm manually. |
| Unreferenced function `fmt` | interop/src/credits.rs:31 | medium | No incoming call edges. Dynamic dispatch, trait impls, and macro-generated callers can hide real callers, so confirm manually. |
| Unreferenced function `fmt` | interop/src/htlc.rs:51 | medium | No incoming call edges. Dynamic dispatch, trait impls, and macro-generated callers can hide real callers, so confirm manually. |
| Unreferenced function `health_handler` | interop/src/lib.rs:188 | medium | No incoming call edges. Dynamic dispatch, trait impls, and macro-generated callers can hide real callers, so confirm manually. |
| Unreferenced function `payload_digest` | interop/src/lib.rs:248 | medium | No incoming call edges. Dynamic dispatch, trait impls, and macro-generated callers can hide real callers, so confirm manually. |
| Unreferenced function `submit_job_handler` | interop/src/lib.rs:256 | medium | No incoming call edges. Dynamic dispatch, trait impls, and macro-generated callers can hide real callers, so confirm manually. |
| Unreferenced function `drop` | interop/src/lib.rs:663 | medium | No incoming call edges. Dynamic dispatch, trait impls, and macro-generated callers can hide real callers, so confirm manually. |
| Unreferenced function `fmt` | interop/src/liquidity.rs:96 | medium | No incoming call edges. Dynamic dispatch, trait impls, and macro-generated callers can hide real callers, so confirm manually. |
| Unreferenced function `fmt` | interop/src/openapi_gateway.rs:83 | medium | No incoming call edges. Dynamic dispatch, trait impls, and macro-generated callers can hide real callers, so confirm manually. |
| Unreferenced function `fmt` | interop/src/twamm.rs:66 | medium | No incoming call edges. Dynamic dispatch, trait impls, and macro-generated callers can hide real callers, so confirm manually. |
| Unreferenced function `serde_json_orders` | interop/src/twamm.rs:348 | medium | No incoming call edges. Dynamic dispatch, trait impls, and macro-generated callers can hide real callers, so confirm manually. |
| Unreferenced function `fmt` | mesh-transport/src/ble_auth.rs:203 | medium | No incoming call edges. Dynamic dispatch, trait impls, and macro-generated callers can hide real callers, so confirm manually. |
| Unreferenced function `fmt` | mesh-transport/src/ble_hal.rs:128 | medium | No incoming call edges. Dynamic dispatch, trait impls, and macro-generated callers can hide real callers, so confirm manually. |
| Unreferenced function `fmt` | mesh-transport/src/ble_mesh.rs:297 | medium | No incoming call edges. Dynamic dispatch, trait impls, and macro-generated callers can hide real callers, so confirm manually. |
| Unreferenced function `rejects_unprovisioned_secret` | mesh-transport/src/ble_peripheral/tests.rs:280 | medium | No incoming call edges. Dynamic dispatch, trait impls, and macro-generated callers can hide real callers, so confirm manually. |
| Unreferenced function `tick_starts_advertising_with_mesh_prefix_and_service_uuid` | mesh-transport/src/ble_peripheral/tests.rs:293 | medium | No incoming call edges. Dynamic dispatch, trait impls, and macro-generated callers can hide real callers, so confirm manually. |
| Unreferenced function `advertise_failure_stays_idle_and_retries` | mesh-transport/src/ble_peripheral/tests.rs:308 | medium | No incoming call edges. Dynamic dispatch, trait impls, and macro-generated callers can hide real callers, so confirm manually. |
| Unreferenced function `connect_sends_hello_with_fresh_challenge` | mesh-transport/src/ble_peripheral/tests.rs:319 | medium | No incoming call edges. Dynamic dispatch, trait impls, and macro-generated callers can hide real callers, so confirm manually. |
| Unreferenced function `full_handshake_reaches_active_and_registers_neighbor` | mesh-transport/src/ble_peripheral/tests.rs:341 | medium | No incoming call edges. Dynamic dispatch, trait impls, and macro-generated callers can hide real callers, so confirm manually. |
| Unreferenced function `wrong_secret_gets_reject_and_backoff` | mesh-transport/src/ble_peripheral/tests.rs:361 | medium | No incoming call edges. Dynamic dispatch, trait impls, and macro-generated callers can hide real callers, so confirm manually. |
| Unreferenced function `tampered_proof_rejected` | mesh-transport/src/ble_peripheral/tests.rs:395 | medium | No incoming call edges. Dynamic dispatch, trait impls, and macro-generated callers can hide real callers, so confirm manually. |
| Unreferenced function `proof_before_auth_init_is_protocol_violation` | mesh-transport/src/ble_peripheral/tests.rs:425 | medium | No incoming call edges. Dynamic dispatch, trait impls, and macro-generated callers can hide real callers, so confirm manually. |
| Unreferenced function `auth_timeout_disconnects_and_backs_off` | mesh-transport/src/ble_peripheral/tests.rs:438 | medium | No incoming call edges. Dynamic dispatch, trait impls, and macro-generated callers can hide real callers, so confirm manually. |
| Unreferenced function `rng_failure_on_connect_backs_off` | mesh-transport/src/ble_peripheral/tests.rs:451 | medium | No incoming call edges. Dynamic dispatch, trait impls, and macro-generated callers can hide real callers, so confirm manually. |
| Unreferenced function `notify_failure_on_hello_drops_link` | mesh-transport/src/ble_peripheral/tests.rs:460 | medium | No incoming call edges. Dynamic dispatch, trait impls, and macro-generated callers can hide real callers, so confirm manually. |
| Unreferenced function `lsa_exchange_updates_neighbor_table` | mesh-transport/src/ble_peripheral/tests.rs:469 | medium | No incoming call edges. Dynamic dispatch, trait impls, and macro-generated callers can hide real callers, so confirm manually. |
| Unreferenced function `lsa_before_auth_is_ignored` | mesh-transport/src/ble_peripheral/tests.rs:491 | medium | No incoming call edges. Dynamic dispatch, trait impls, and macro-generated callers can hide real callers, so confirm manually. |
| Unreferenced function `malformed_frames_trigger_reject_limit_disconnect` | mesh-transport/src/ble_peripheral/tests.rs:506 | medium | No incoming call edges. Dynamic dispatch, trait impls, and macro-generated callers can hide real callers, so confirm manually. |
| Unreferenced function `interleaved_fragment_message_ids_rejected` | mesh-transport/src/ble_peripheral/tests.rs:521 | medium | No incoming call edges. Dynamic dispatch, trait impls, and macro-generated callers can hide real callers, so confirm manually. |
| Unreferenced function `write_to_non_writable_characteristic_rejected` | mesh-transport/src/ble_peripheral/tests.rs:554 | medium | No incoming call edges. Dynamic dispatch, trait impls, and macro-generated callers can hide real callers, so confirm manually. |
| Unreferenced function `second_connection_while_active_is_refused` | mesh-transport/src/ble_peripheral/tests.rs:569 | medium | No incoming call edges. Dynamic dispatch, trait impls, and macro-generated callers can hide real callers, so confirm manually. |
| Unreferenced function `bye_returns_to_advertising_without_backoff` | mesh-transport/src/ble_peripheral/tests.rs:588 | medium | No incoming call edges. Dynamic dispatch, trait impls, and macro-generated callers can hide real callers, so confirm manually. |
| Unreferenced function `peer_timeout_disconnects_and_backs_off` | mesh-transport/src/ble_peripheral/tests.rs:603 | medium | No incoming call edges. Dynamic dispatch, trait impls, and macro-generated callers can hide real callers, so confirm manually. |
| Unreferenced function `lsa_rebroadcast_cadence` | mesh-transport/src/ble_peripheral/tests.rs:617 | medium | No incoming call edges. Dynamic dispatch, trait impls, and macro-generated callers can hide real callers, so confirm manually. |
| Unreferenced function `backoff_is_exponential_and_recovers` | mesh-transport/src/ble_peripheral/tests.rs:647 | medium | No incoming call edges. Dynamic dispatch, trait impls, and macro-generated callers can hide real callers, so confirm manually. |
| Unreferenced function `successful_auth_resets_backoff` | mesh-transport/src/ble_peripheral/tests.rs:684 | medium | No incoming call edges. Dynamic dispatch, trait impls, and macro-generated callers can hide real callers, so confirm manually. |
| Unreferenced function `unknown_connection_events_ignored` | mesh-transport/src/ble_peripheral/tests.rs:717 | medium | No incoming call edges. Dynamic dispatch, trait impls, and macro-generated callers can hide real callers, so confirm manually. |
| Unreferenced function `empty_node_name_rejected` | mesh-transport/src/ble_peripheral/tests.rs:739 | medium | No incoming call edges. Dynamic dispatch, trait impls, and macro-generated callers can hide real callers, so confirm manually. |
| Unreferenced function `hal_rejects_oversized_notify` | mesh-transport/src/ble_peripheral/tests.rs:759 | medium | No incoming call edges. Dynamic dispatch, trait impls, and macro-generated callers can hide real callers, so confirm manually. |
| Unreferenced function `fmt` | mesh-transport/src/identity_manager.rs:53 | medium | No incoming call edges. Dynamic dispatch, trait impls, and macro-generated callers can hide real callers, so confirm manually. |
| Unreferenced function `fmt` | mesh-transport/src/identity_manager.rs:87 | medium | No incoming call edges. Dynamic dispatch, trait impls, and macro-generated callers can hide real callers, so confirm manually. |
| Unreferenced function `fmt` | mesh-transport/src/identity_manager.rs:152 | medium | No incoming call edges. Dynamic dispatch, trait impls, and macro-generated callers can hide real callers, so confirm manually. |
| Unreferenced function `hex_prefix` | mesh-transport/src/identity_manager.rs:381 | medium | No incoming call edges. Dynamic dispatch, trait impls, and macro-generated callers can hide real callers, so confirm manually. |
| Unreferenced function `hex_short` | mesh-transport/src/network.rs:335 | medium | No incoming call edges. Dynamic dispatch, trait impls, and macro-generated callers can hide real callers, so confirm manually. |
| Unreferenced function `fmt` | mesh-transport/src/vdf_sybil.rs:217 | medium | No incoming call edges. Dynamic dispatch, trait impls, and macro-generated callers can hide real callers, so confirm manually. |
| Unreferenced function `fmt` | sandbox/src/admit.rs:180 | medium | No incoming call edges. Dynamic dispatch, trait impls, and macro-generated callers can hide real callers, so confirm manually. |
| Unreferenced function `min_steps` | sandbox/src/admit.rs:311 | medium | No incoming call edges. Dynamic dispatch, trait impls, and macro-generated callers can hide real callers, so confirm manually. |
| Unreferenced function `required_modulus_id` | sandbox/src/admit.rs:313 | medium | No incoming call edges. Dynamic dispatch, trait impls, and macro-generated callers can hide real callers, so confirm manually. |
| Unreferenced function `min_steps` | sandbox/src/admit.rs:386 | medium | No incoming call edges. Dynamic dispatch, trait impls, and macro-generated callers can hide real callers, so confirm manually. |
| Unreferenced function `required_modulus_id` | sandbox/src/admit.rs:390 | medium | No incoming call edges. Dynamic dispatch, trait impls, and macro-generated callers can hide real callers, so confirm manually. |
| Unreferenced function `min_steps` | sandbox/src/admit.rs:525 | medium | No incoming call edges. Dynamic dispatch, trait impls, and macro-generated callers can hide real callers, so confirm manually. |
| Unreferenced function `required_modulus_id` | sandbox/src/admit.rs:529 | medium | No incoming call edges. Dynamic dispatch, trait impls, and macro-generated callers can hide real callers, so confirm manually. |
| Unreferenced function `fmt` | sandbox/src/host.rs:57 | medium | No incoming call edges. Dynamic dispatch, trait impls, and macro-generated callers can hide real callers, so confirm manually. |
| Unreferenced function `fmt` | sandbox/src/job.rs:47 | medium | No incoming call edges. Dynamic dispatch, trait impls, and macro-generated callers can hide real callers, so confirm manually. |
| Unreferenced function `fmt` | sandbox/src/pool.rs:42 | medium | No incoming call edges. Dynamic dispatch, trait impls, and macro-generated callers can hide real callers, so confirm manually. |
| Unreferenced function `fmt` | sandbox/src/quant.rs:65 | medium | No incoming call edges. Dynamic dispatch, trait impls, and macro-generated callers can hide real callers, so confirm manually. |

## Single-call wrapper functions (22)

| Finding | Location | Confidence | Note |
| --- | --- | --- | --- |
| `map_op_time` only forwards to `new` | consensus/src/crdt.rs:495 | medium-high | Body holds a single call expression and nothing else. Check whether the indirection still earns its keep. |
| `default` only forwards to `new` | consensus/src/lib.rs:105 | medium-high | Body holds a single call expression and nothing else. Check whether the indirection still earns its keep. |
| `default` only forwards to `genesis` | consensus/src/lib.rs:172 | medium-high | Body holds a single call expression and nothing else. Check whether the indirection still earns its keep. |
| `default` only forwards to `new` | consensus/src/trie.rs:190 | medium-high | Body holds a single call expression and nothing else. Check whether the indirection still earns its keep. |
| `default` only forwards to `new` | consensus/src/trie.rs:857 | medium-high | Body holds a single call expression and nothing else. Check whether the indirection still earns its keep. |
| `fold` only forwards to `fold_proofs` | consensus/src/verifier.rs:48 | medium-high | Body holds a single call expression and nothing else. Check whether the indirection still earns its keep. |
| `verify` only forwards to `verify_folded_proof` | consensus/src/verifier.rs:56 | medium-high | Body holds a single call expression and nothing else. Check whether the indirection still earns its keep. |
| `default` only forwards to `new` | engine/src/lib.rs:86 | medium-high | Body holds a single call expression and nothing else. Check whether the indirection still earns its keep. |
| `componentDidCatch` only forwards to `error` | frontend/src/components/ErrorBoundary.tsx:9 | medium-high | Body holds a single call expression and nothing else. Check whether the indirection still earns its keep. |
| `new` only forwards to `into` | interop/src/lib.rs:641 | medium-high | Body holds a single call expression and nothing else. Check whether the indirection still earns its keep. |
| `drop` only forwards to `close` | interop/src/lib.rs:663 | medium-high | Body holds a single call expression and nothing else. Check whether the indirection still earns its keep. |
| `default` only forwards to `new` | interop/src/openapi_gateway.rs:121 | medium-high | Body holds a single call expression and nothing else. Check whether the indirection still earns its keep. |
| `poll_event` only forwards to `pop_front` | mesh-transport/src/ble_peripheral/tests.rs:79 | medium-high | Body holds a single call expression and nothing else. Check whether the indirection still earns its keep. |
| `fmt` only forwards to `write_str` | mesh-transport/src/identity_manager.rs:53 | medium-high | Body holds a single call expression and nothing else. Check whether the indirection still earns its keep. |
| `default` only forwards to `new` | mesh-transport/src/kademlia_routing.rs:258 | medium-high | Body holds a single call expression and nothing else. Check whether the indirection still earns its keep. |
| `default` only forwards to `new` | mesh-transport/src/kademlia_routing.rs:289 | medium-high | Body holds a single call expression and nothing else. Check whether the indirection still earns its keep. |
| `default` only forwards to `new` | mesh-transport/src/lora_mac.rs:278 | medium-high | Body holds a single call expression and nothing else. Check whether the indirection still earns its keep. |
| `default` only forwards to `new` | mesh-transport/src/packet_translator.rs:397 | medium-high | Body holds a single call expression and nothing else. Check whether the indirection still earns its keep. |
| `default` only forwards to `new` | mesh-transport/src/simulation.rs:269 | medium-high | Body holds a single call expression and nothing else. Check whether the indirection still earns its keep. |
| `default` only forwards to `for_tests` | mesh-transport/src/vdf_sybil.rs:268 | medium-high | Body holds a single call expression and nothing else. Check whether the indirection still earns its keep. |
| `default` only forwards to `production` | sandbox/src/admit.rs:435 | medium-high | Body holds a single call expression and nothing else. Check whether the indirection still earns its keep. |
| `default` only forwards to `new` | sandbox/src/admit.rs:519 | medium-high | Body holds a single call expression and nothing else. Check whether the indirection still earns its keep. |

## No-op deprecations (0)

_No findings._

## Oversized test files (23)

| Finding | Location | Confidence | Note |
| --- | --- | --- | --- |
| Test file has 547 lines | ai/src/compute.rs | high | Plain metric: over the 500-line budget. Consider splitting. |
| Test file has 692 lines | ai/src/paged_attention.rs | high | Plain metric: over the 500-line budget. Consider splitting. |
| Test file has 680 lines | consensus/src/crdt.rs | high | Plain metric: over the 500-line budget. Consider splitting. |
| Test file has 1247 lines | consensus/src/trie.rs | high | Plain metric: over the 500-line budget. Consider splitting. |
| Test file has 692 lines | governance/src/multisig.rs | high | Plain metric: over the 500-line budget. Consider splitting. |
| Test file has 806 lines | governance/src/voting.rs | high | Plain metric: over the 500-line budget. Consider splitting. |
| Test file has 660 lines | integration/src/lib.rs | high | Plain metric: over the 500-line budget. Consider splitting. |
| Test file has 604 lines | interop/src/htlc.rs | high | Plain metric: over the 500-line budget. Consider splitting. |
| Test file has 1146 lines | interop/src/lib.rs | high | Plain metric: over the 500-line budget. Consider splitting. |
| Test file has 510 lines | interop/src/liquidity.rs | high | Plain metric: over the 500-line budget. Consider splitting. |
| Test file has 682 lines | interop/src/twamm.rs | high | Plain metric: over the 500-line budget. Consider splitting. |
| Test file has 937 lines | mesh-transport/src/ble_mesh.rs | high | Plain metric: over the 500-line budget. Consider splitting. |
| Test file has 767 lines | mesh-transport/src/ble_peripheral/tests.rs | high | Plain metric: over the 500-line budget. Consider splitting. |
| Test file has 577 lines | mesh-transport/src/identity_manager.rs | high | Plain metric: over the 500-line budget. Consider splitting. |
| Test file has 724 lines | mesh-transport/src/kademlia_routing.rs | high | Plain metric: over the 500-line budget. Consider splitting. |
| Test file has 519 lines | mesh-transport/src/lora_mac.rs | high | Plain metric: over the 500-line budget. Consider splitting. |
| Test file has 510 lines | mesh-transport/src/network.rs | high | Plain metric: over the 500-line budget. Consider splitting. |
| Test file has 599 lines | mesh-transport/src/packet_translator.rs | high | Plain metric: over the 500-line budget. Consider splitting. |
| Test file has 518 lines | mesh-transport/src/simulation.rs | high | Plain metric: over the 500-line budget. Consider splitting. |
| Test file has 517 lines | mesh-transport/src/stun_hole_punch.rs | high | Plain metric: over the 500-line budget. Consider splitting. |
| Test file has 909 lines | mesh-transport/src/vdf_sybil.rs | high | Plain metric: over the 500-line budget. Consider splitting. |
| Test file has 607 lines | mesh-transport/tests/ble_peripheral_integration.rs | high | Plain metric: over the 500-line budget. Consider splitting. |
| Test file has 815 lines | sandbox/src/admit.rs | high | Plain metric: over the 500-line budget. Consider splitting. |

## Possibly unused dependencies (0)

_No findings._

## How to read this

Confidence labels say how much to trust each finding before acting:

- **high**: directly measured facts (a metric or an explicit attribute). Safe to act on.
- **medium-high**: strong structural signal with a small chance of a false positive.
- **medium**: worth a look, but callers can hide behind dynamic dispatch, trait impls,
  macros, or FFI boundaries. Confirm before deleting.
- **low**: leads, not verdicts. The unused-dependency check is a plain text/edge
  heuristic; build scripts and re-exports routinely defeat it.

This report is a triage list. Every finding below medium-high confidence should get
a human glance before any code is touched.
