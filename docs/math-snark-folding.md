# Constant-Size SNARK Folding — Mathematical Notes

**Owner:** Math / agent math-snark (extends `agent/07-state-compression`)  
**Code:** [`consensus/src/snark.rs`](../consensus/src/snark.rs), [`consensus/src/folding.rs`](../consensus/src/folding.rs)  
**SLAs:** edge active ledger ≤ 10 MB; radio anchor ≈ 200-byte constant proof; MicroSpartan-style verifier ≈ 10k gates.

This document formalizes the *intended* Nova-style IVC invariants that the consensus crate encodes, maps them to types, and states honestly what the **mock** prover guarantees versus what a real `nova-snark` (Pallas/Vesta) integration must supply.

---

## 1. Definitions

### 1.1 State roots

Let \(\mathcal{R} = \{0,1\}^{256}\) be the set of Merkle / trie roots (32-byte digests).

A ledger transition is a pair \((r_{\mathrm{prev}}, r_{\mathrm{next}}) \in \mathcal{R} \times \mathcal{R}\) together with a witness digest \(w \in \{0,1\}^{256}\) (e.g. WASM execution trace hash).

### 1.2 Step instance

A **step instance** is the public claim of one incremental transition:

\[
S = (r_{\mathrm{prev}}, r_{\mathrm{next}}, w) \in \mathcal{R} \times \mathcal{R} \times \{0,1\}^{256}.
\]

**Code:** `StepInstance { prev_state_root, next_state_root, witness_digest }`.

**Step digest** (domain-separated):

\[
\delta(S) := H_{\mathrm{step}}\big(\texttt{mossymesh/step/v1}\ \|\ r_{\mathrm{prev}}\ \|\ r_{\mathrm{next}}\ \|\ w\big)
\]

where \(H_{\mathrm{step}}\) is length-prefixed SHA-256 over concatenated parts (see `hash_parts`).

### 1.3 Accumulator (folded proof)

An **accumulator** (public IVC state after \(n\) folds) is:

\[
A_n = \big(\pi_{\mathrm{pub}},\ n,\ r_{\mathrm{claim}}\big)
\]

where:

| Symbol | Meaning | Code field |
| --- | --- | --- |
| \(\pi_{\mathrm{pub}}\) | Fixed-length public representation | `SnarkProof.public` |
| \(n \in \mathbb{N}_0\) | Number of steps folded | `SnarkProof.fold_count` |
| \(r_{\mathrm{claim}} \in \mathcal{R}\) | Claimed final state root | `SnarkProof.claimed_state_root` |

**Genesis (identity) accumulator** at root \(r_0\):

\[
A_0 = \mathrm{Genesis}(r_0),\quad n=0,\quad r_{\mathrm{claim}}=r_0.
\]

### 1.4 Fold operator

The fold operator extends an accumulator by one step:

\[
\mathrm{Fold}(A_n, S) \mapsto A_{n+1}
\]

subject to the **linking predicate** (defined below). Intuitively:

\[
A_{n+1}\ \text{claims}\ r_{\mathrm{claim}}' = r_{\mathrm{next}}\ \text{and}\ n' = n+1
\]

while compressing prior history into a constant-size public blob.

### 1.5 Public input for verification

\[
\mathrm{PI} = (r_{\mathrm{genesis}}, r_{\mathrm{final}}, n_{\min})
\]

**Code:** `PublicInput { genesis_state_root, final_state_root, min_fold_count }`.

### 1.6 Domain separation tags

Mock commitments use distinct ASCII domain separators so transcripts do not collide across roles (and so a future real Nova path can choose different tags without silently reusing mock hashes):

| Domain constant | Bytes | Role |
| --- | --- | --- |
| `DOMAIN_PROOF` | `mossymesh/snark/v1` | Genesis / single-step commitments |
| `DOMAIN_STEP` | `mossymesh/step/v1` | Step instance digests |
| `DOMAIN_FOLD` | `mossymesh/fold/v1` | Fold commitment chain |
| (aux) | `agg`, `pad`, `pad/next`, `preprocess`, `circuit` | Aggregation, padding, preprocessing |

**Invariant (domain hygiene):** no mock commitment for fold uses the step-only domain alone; fold commits under `DOMAIN_FOLD` and aggregates step digests under `agg`. A production Nova integration MUST use a separate domain / transcript (e.g. Poseidon over the cycle of curves) and MUST NOT accept mock-domain proofs as sound.

---

## 2. Public layout (constant size)

### 2.1 Anchor size

\[
|\pi_{\mathrm{pub}}| = \mathtt{ANCHOR\_PROOF\_SIZE} = 200\ \text{bytes}.
\]

This is the HF/Ham radio anchoring budget cited in the project README.

### 2.2 Byte layout

```
offset 0  .. 31 : commitment     (32)
offset 32 .. 63 : step_digest    (32)   // leaf step dig or aggregated chain dig
offset 64 .. 71 : fold_count     (u64 LE)
offset 72       : flags          (u8)   // 0 = genesis/leaf encode; 1 = folded
offset 73 ..199 : deterministic padding derived from (commitment, step_digest, fold_count)
```

Padding is not free-form attacker-controlled zeros: it is expanded from

\[
\mathrm{pad}_0 = H(\texttt{pad}\ \|\ C\ \|\ \delta\ \|\ n),\quad
\mathrm{pad}_{k+1} = H(\texttt{pad/next}\ \|\ \mathrm{pad}_k).
\]

---

## 3. Invariants as lemmas

Below, “holds in code” means the mock implementation enforces the check structurally. “Holds for real Nova” is the cryptographic claim required of a production prover.

### Lemma 1 (Constant public size)

**Statement.** For every well-formed accumulator \(A_n\) produced by `genesis`, `from_step`, or `fold_proofs`,

\[
\big|A_n.\pi_{\mathrm{pub}}\big| = 200.
\]

**Proof (structural).** \(\pi_{\mathrm{pub}}\) has Rust type `[u8; ANCHOR_PROOF_SIZE]`. Encoding functions write into a fixed array of length 200; no field grows with \(n\). \(\square\)

**Tests:** `fold_preserves_constant_public_size`, `size_invariant_holds_after_n_folds`, `verification_payload_stays_sub_megabyte_after_many_folds`.

**SLA link:** P3-DoD-1 (sub-megabyte constant/bounded edge proof); radio 200-byte anchor.

---

### Lemma 2 (Fold-count monotonicity and embedding)

**Statement.** If \(A' = \mathrm{Fold}(A, S)\) succeeds, then

\[
A'.n = A.n + 1
\]

(with checked arithmetic; overflow rejects), and the LE `u64` at bytes `[64..72)` of \(\pi_{\mathrm{pub}}\) equals \(A'.n\).

**Proof (code).** `new_fold = old.fold_count.checked_add(1)`; embedded via `encode_folded_public`. Well-formedness requires `parse_public().fold_count == fold_count`. \(\square\)

---

### Lemma 3 (State-root linking / continuity)

**Statement.** \(\mathrm{Fold}(A, S)\) succeeds only if

\[
A.r_{\mathrm{claim}} = S.r_{\mathrm{prev}}.
\]

Then \(A'.r_{\mathrm{claim}} = S.r_{\mathrm{next}}\).

**Proof (code).** Explicit equality check in `fold_proofs`; assignment of `claimed_state_root` to `next_state_root`. \(\square\)

**Corollary (chain continuity).** For a sequence \(S_1,\ldots,S_k\) folded from genesis \(r_0\), success implies

\[
S_1.r_{\mathrm{prev}} = r_0,\quad
S_{i+1}.r_{\mathrm{prev}} = S_i.r_{\mathrm{next}}\ \ (1 \le i < k),\quad
A_k.r_{\mathrm{claim}} = S_k.r_{\mathrm{next}}.
\]

**Tests:** `fold_rejects_unlinked_step`, `fold_rejects_broken_mid_chain_linkage`, `verify_folded_proof_accepts_valid_chain`.

---

### Lemma 4 (Verification public-input consistency)

**Statement.** `verify_folded_proof(A, PI)` accepts only if:

1. \(A\) is well-formed (`is_well_formed`);
2. \(|A.\pi_{\mathrm{pub}}| = 200\);
3. \(A.n \ge \mathrm{PI}.n_{\min}\);
4. \(A.r_{\mathrm{claim}} = \mathrm{PI}.r_{\mathrm{final}}\);
5. verification payload length \(< 1\,\mathrm{MiB}\);
6. if \(A.n = 0\), then \(A.r_{\mathrm{claim}} = \mathrm{PI}.r_{\mathrm{genesis}}\).

**Note.** The mock does **not** re-derive the commitment hash chain against a secret witness. Acceptance is structural + public-input matching, not knowledge soundness.

**Tests:** `verify_rejects_wrong_final_root`, `verify_rejects_under_min_fold_count`, `verify_rejects_tampered_public_bytes`, genesis mismatch cases.

---

### Lemma 5 (Determinism of the mock fold)

**Statement.** For fixed \(A\) and \(S\), \(\mathrm{Fold}(A, S)\) is a pure function of the byte inputs: same inputs ⇒ identical `SnarkProof` (including padding).

**Proof.** SHA-256 is deterministic; all domains and encodings are fixed. \(\square\)

**SLA link:** SLA-DET / cross-device determinism — all honest nodes compute the same mock anchor bytes for the same ledger history.

**Tests:** `fold_is_deterministic`, `multi_step_fold_is_deterministic`.

---

### Lemma 6 (MicroSpartan gate-budget bound — declared)

**Statement.** Preprocessing artifacts declare

\[
\mathrm{gate\_count} = \mathtt{MICROSPARTAN\_GATE\_COUNT} = 10\,000,
\]

with metadata length exactly \(\mathtt{MICROSPARTAN\_PREPROCESS\_META\_BYTES} = 512\), and total preprocessing payload \(\ll 1\,\mathrm{MiB}\).

**Interpretation.** This is a **budget constant** and interface contract for a future constant-sized recursive verifier circuit (MicroSpartan / Spartan-style), not a measured gate count of a compiled R1CS in this crate. `verify_preprocessing` rejects any other gate count so edge nodes refuse “growing circuit” metadata.

**Tests:** `microsparatan_gate_budget_is_10k`, `microsparatan_preprocess_is_constant_and_bounded`.

---

### Lemma 7 (Sub-megabyte verification payload)

**Statement.** For any well-formed proof and public input constructed by the API,

\[
|\pi_{\mathrm{pub}}| + |r_{\mathrm{genesis}}| + |r_{\mathrm{final}}| + 8 + 8 + 32 < 1\,048\,576.
\]

In practice this is \(200 + 32 + 32 + 8 + 8 + 32 = 312\) bytes — independent of fold depth.

**SLA link:** edge verification must not grow with ledger history length.

---

## 4. Mock fold algebra (exact hash equations)

Let \(H\) denote `hash_parts` (length-prefixed SHA-256).

**Genesis commitment:**

\[
C_0 = H(\texttt{mossymesh/snark/v1}\ \|\ r_0\ \|\ 0),\quad
\delta_0 = H(\texttt{mossymesh/step/v1}\ \|\ r_0),\quad
\mathrm{flags}=0.
\]

**Single-step leaf** (`from_step`):

\[
C_1 = H(\texttt{mossymesh/snark/v1}\ \|\ r_{\mathrm{prev}}\ \|\ r_{\mathrm{next}}\ \|\ w\ \|\ 1),\quad
\delta_1 = \delta(S),\quad n=1,\ \mathrm{flags}=0.
\]

**Fold step** (`fold_proofs`), given old public parse \((C, \delta_{\mathrm{agg}}, n, \cdot)\):

\[
\begin{aligned}
n' &= n + 1,\\
\delta' &= \delta(S),\\
C' &= H(\texttt{mossymesh/fold/v1}\ \|\ C\ \|\ \delta'\ \|\ r_{\mathrm{next}}\ \|\ n'),\\
\delta_{\mathrm{agg}}' &= H(\texttt{agg}\ \|\ \delta_{\mathrm{agg}}\ \|\ \delta'\ \|\ n'),\\
\mathrm{flags}' &= 1.
\end{aligned}
\]

These equations make the mock **transcript-chaining** explicit. They are **not** a knowledge-sound folding scheme.

---

## 5. Collision, malleability, and honesty limits (mock prover)

The mock is intentional scaffolding for size / linking / determinism SLAs. It is **not** a SNARK.

| Property | Real Nova / Spartan | Mock (`sha2` commitments) |
| --- | --- | --- |
| Knowledge soundness | Witness-hiding + argument of knowledge for R1CS | **None** — anyone can recompute \(H(\ldots)\) |
| Zero-knowledge | Approx. (with suitable setup / Fiat–Shamir) | **None** — digests are public hashes of public data |
| Non-malleability of proofs | Relies on random oracle / group hardness | **Weak** — flip bytes ⇒ usually `is_well_formed` fails if fold_count desynced; commitment bits alone may still pass structural checks |
| Collision resistance of digests | SHA-256 assumed CR for binding mock transcripts | Same assumption for *binding* only, not for *soundness* |
| Forgeability | Computationally hard under curve assumptions | **Trivial** for an adversary who can write `SnarkProof` fields |

**Concrete malleability notes:**

1. **Structural verify gap:** `verify_folded_proof` does not recompute \(C'\) from an authoritative step list; a forged well-formed public blob with matching `claimed_state_root` / `fold_count` can pass mock verify.
2. **`fold_snarks` witness hole:** reconstructing `StepInstance` from a step proof uses the *parsed step digest as `witness_digest`*, which is not the original witness — API compatibility only; callers must use `fold_proofs` with explicit steps.
3. **Domain separation** prevents accidental mix-ups between step digests and fold commitments *within* the mock, but does not create hardness.
4. **Padding derivation** stops “zero-tail” free edits from looking unique without changing pad inputs; an attacker who controls commitment still controls padding.

**Production requirement:** replace mock fold/verify with nova-snark IVC (or equivalent) before treating anchors as fraud proofs or slashable evidence.

---

## 6. Mapping to code types

| Math object | Rust type / symbol |
| --- | --- |
| \(r \in \mathcal{R}\) | `[u8; 32]` state root |
| \(S\) | `StepInstance` |
| \(\delta(S)\) | `StepInstance::digest` |
| \(A_n\) | `SnarkProof` |
| \(\pi_{\mathrm{pub}}\) | `SnarkProof.public` / `public_bytes()` |
| \(\mathrm{PI}\) | `PublicInput` |
| \(\mathrm{Fold}\) | `fold_proofs` |
| \(\mathrm{Fold}^*\) sequence | `fold_sequence` |
| \(\mathrm{Verify}\) | `verify_folded_proof` |
| Gate budget artifact | `MicroSpartanPreprocessing` |
| Anchor size | `ANCHOR_PROOF_SIZE = 200` |
| Payload cap | `MAX_VERIFICATION_PAYLOAD_BYTES = 1_048_576` |
| Gate budget | `MICROSPARTAN_GATE_COUNT = 10_000` |

**Cross-crate contract** (`docs/interface-contracts.md`): `CommitReceipt.proof` prefers constant-size SNARK bytes — this module’s `public_bytes()` is the intended payload.

---

## 7. What real `nova-snark` replaces vs what the mock keeps

### 7.1 Replace (cryptographic core)

| Mock today | Real Nova path |
| --- | --- |
| SHA-256 `hash_parts` commitments | Folded R1CS instance + relaxed R1CS accumulator over Pallas/Vesta (cycle of curves) |
| `verify_folded_proof` structural checks | Recursive verifier: check NIFS / folding correct + primary SNARK (e.g. Compressed SNARK / Spartan) |
| `MicroSpartanPreprocessing` metadata stub | Actual circuit synthesis of “verify one step + verify prior accumulator”, gate count measured ≤ budget |
| `flags` / padding layout as sole wire format | May keep 200-byte **compressed** public anchor (hash of full proof or fixed serialization of primary/secondary outputs) for radio; full proof may live on hub SSD |

### 7.2 Keep (interface invariants)

These must remain true after swapping the prover:

1. **Constant-size public anchor** = 200 bytes for HF/Ham (`ANCHOR_PROOF_SIZE`).
2. **Linking:** fold only if claimed root matches step `prev`.
3. **Fold count** non-decreasing by exactly 1 per accepted step (or equivalent IVC step counter).
4. **Determinism:** same public inputs + circuit version ⇒ same verification outcome on all devices.
5. **Sub-megabyte** verification path on edge (full proof may be larger on hubs; edge checks compressed form + public IO).
6. **Domain / version tags** so mock proofs never verify under the real verifier (and vice versa).

### 7.3 Suggested migration domain

When enabling real Nova, introduce a distinct domain, e.g. `mossymesh/nova/v1`, and reject `mossymesh/fold/v1` mock transcripts in production builds (feature flag `real-nova`).

---

## 8. Informal soundness goal (production)

For production IVC we want (sketch):

> If `verify_folded_proof(A, PI)` accepts under the real verifier, then except with negligible probability there exist witnesses \(w_1,\ldots,w_n\) and roots \(r_0,\ldots,r_n\) with \(r_0 = \mathrm{PI}.r_{\mathrm{genesis}}\), \(r_n = \mathrm{PI}.r_{\mathrm{final}}\), \(n \ge n_{\min}\), such that each step \((r_{i-1}, r_i, w_i)\) is a valid ledger transition relation \(R\).

The mock only approximates the *shape* of this statement (size, counters, root fields), not the existence of witnesses for \(R\).

---

## 9. Change-control checklist (crypto)

Before merging a prover swap to `main`:

- [ ] Prove (doc + test) that public anchor remains 200 bytes after \(N \ge 256\) folds.
- [ ] Prove linking rejects broken `prev` continuity.
- [ ] Document gate count of the recursive circuit ≤ 10k **or** revise README/SLA with Architect sign-off.
- [ ] Show edge verification payload / RAM stays within SLA-RAM (≤ 10 MB active ledger, sub-MB proof path).
- [ ] Ensure mock and real domains are non-interchangeable.
- [ ] Explicit security review of malleability for slash / radio-anchor threat model.

---

## 10. References (project-local)

- Root `README.md` — 200-byte anchor; ~10k MicroSpartan gates; nova-snark Pallas/Vesta.
- `docs/sla-and-dod.md` — Phase 3 DoD (constant/bounded proof; ≤ 10 MB ledger).
- `docs/interface-contracts.md` — `CommitReceipt.proof`.
- Commit baseline: `aa0c13b` (`feat(consensus): constant-size SNARK folding for ledger compression`).
