# MinRoot-style VDF — parameter theory & security claims

**Owner module:** `mesh-transport/src/vdf_sybil.rs`  
**Consumers:** sandbox admit gate (`sandbox/src/admit.rs`), job DIDs, HTLC VDF-delayed cancel (mock in interop).  
**Agent:** 08 Cryptography (`AGENTS.md`).

This note freezes the mathematical surface used for Sybil-costed Ephemeral Job DIDs. It distinguishes **proven algebraic facts**, **heuristic sequentiality**, and **engineering calibration**.

---

## 1. Map definition

MossyMesh evaluates the iterated map

```text
x₀ = start_x
x_{i+1} = (x_i + i)^d  (mod p)    for i = 1 … T
```

where the exponent is the modular inverse of five in the multiplicative group order:

```text
d ≡ 5⁻¹  (mod p − 1)
```

so that, on `𝔽_p^*`, exponentiation by `d` is the unique group automorphism corresponding to taking fifth roots when that map is bijective.

### Classical closed form

When **`p ≡ 3 (mod 5)`**, `(2p − 1)` is divisible by 5 and

```text
d = (2p − 1) / 5
```

satisfies `5d ≡ 1 (mod p − 1)` because `2p − 1 = 2(p − 1) + 1`.

### Validity condition on `p`

| Residue of `p` mod 5 | `gcd(5, p−1)` | Fifth-root map | Code behavior |
| ---: | ---: | --- | --- |
| `0` | — | `p` not prime (except 5) | Reject if no inverse / `p < 5` |
| `1` | `5` | **Not** a unique automorphism | **`None` / verify fails** |
| `2` | `1` | Inverse exists; no classical closed form | Use `d = 5⁻¹ mod (p−1)` |
| `3` | `1` | Classical `d = (2p−1)/5` | Preferred |
| `4` | `1` | Inverse exists; no classical closed form | Use `d = 5⁻¹ mod (p−1)` |

**Hard rule:** `p ≥ 5` and **`p ≢ 1 (mod 5)`**. Prefer **prime** `p` and **`p ≡ 3 (mod 5)`**.

Implementation APIs:

- `fifth_root_exponent_for(p) -> Option<u64>`
- `VdfParams::is_valid()`
- `try_evaluate_vdf` / `verify_vdf_proof` reject invalid moduli

---

## 2. Sequentiality claim vs actual map

### What the map actually is

- Each step is one modular exponentiation `(x + i)^d mod p`.
- Step `i+1` consumes the full output of step `i` ⇒ the **depth-`T` chain is sequential**.
- Verification in this crate **re-executes all `T` steps** and compares `final_x`. There is **no** Wesolowski/Pietrzak short proof and **no** asymptotically faster verify.

### What is *not* claimed

| Claim sometimes seen in marketing | Status here |
| --- | --- |
| “Memory-hard MinRoot” | **False for this map.** Classic MinRoot is arithmetic-sequential, not memory-hard (contrast Argon2 / Scrypt). |
| “GPU/ASIC useless” | **Overstated.** ASICs can still speed modular mul; they cannot *parallelize a single chain’s depth*. Many cores can mint many independent DIDs. |
| “VDF with fast verify” | **Not this module.** Cost(verify) ≈ Cost(eval) = `Θ(T · log d)` modular multiplies. |
| “Proven time-lock under standard assumptions” | **Heuristic / literature-dependent.** Sequentiality of iterated roots is a *conjecture* in the MinRoot line of work, not a reduction we prove in-tree. |

### Honest sequentiality statement

> Under the heuristic that iterated fifth-root (equivalently exponent-`d`) maps in prime fields have no depth-compressing shortcut better than sequential evaluation, an honest prover spends ~`T` sequential modular exponentiations; a verifier who re-runs the chain spends the same order of work.

For mesh Sybil resistance we only need **asymmetric cost per identity/job**, not a consensus-grade VDF with SNARK-speed verify.

---

## 3. Ephemeral Job DID

```text
JobDID = SHA-256( VDF_output_bytes || job_meta )
```

- `VDF_output_bytes` = `final_x` as **8-byte big-endian** (`u64::to_be_bytes`).
- `job_meta` is application-defined opaque bytes (job payload commitment, submitter, etc.).
- Same `(final_x, job_meta)` ⇒ same DID (deterministic, stable).
- Sandbox admit (`DomainSeparatedHashVdfStub`) is a **test stub** with a *different* domain-separated mint; production admit must call transport MinRoot verify + this DID formula (or a pre-verified receipt).

---

## 4. Parameter selection checklist (~10 min delay)

Use this checklist before freezing consensus / mainnet parameters.

1. **Field / modulus**
   - [ ] `p` prime (or cryptographically agreed prime field).
   - [ ] `p ≢ 1 (mod 5)`; prefer `p ≡ 3 (mod 5)`.
   - [ ] Classical check: `(2p − 1) % 5 == 0` and `5 · d % (p − 1) == 1`.
   - [ ] Bit length: u64 stand-in is fine for PoC; production may move to ≥255-bit fields when big-int MinRoot lands.
2. **Exponent**
   - [ ] `d = fifth_root_exponent_for(p)` is `Some`.
   - [ ] Prefer classical `d = (2p − 1) / 5`.
3. **Iteration count `T`**
   - [ ] Benchmark **single-core** `evaluate_vdf` on the **slowest supported class** (Pi Zero 2 W class).
   - [ ] Target wall-clock **≈ 600 s ± tolerance (e.g. 8–12 min) for honest nodes.
   - [ ] Record: CPU model, clock, rustc version, `T`, measured seconds, iterations/sec.
   - [ ] Set consensus `T` from the slow tier so laptops cannot mint DIDs in seconds relative to phones/Pis unless that asymmetry is explicitly accepted.
4. **Verify policy**
   - [ ] Reject `p ≡ 1 (mod 5)`, `p < 5`, iteration under-claims, wrong `final_x`.
   - [ ] Bind DID: `SHA-256(be_bytes(final_x) || job_meta)`.
5. **Operational**
   - [ ] Publish `(modulus_id → (p, T, d))` so all islands agree.
   - [ ] Do not treat sandbox hash stub as Sybil-hard.

### Rough cost model (u64 modular exp)

Per step: `Θ(log₂ d)` modular multiplies. For production `p ≈ 10⁹`, `d ≈ 4·10⁸`, `log₂ d ≈ 29`.

```text
work ≈ T · 30  modular multiplies of 64-bit limbs
```

`T = 50_000_000` ⇒ ~`1.5 · 10⁹` multiplies. On a few million mul/s class device that is order **minutes**; **always re-benchmark** rather than trusting this back-of-envelope.

---

## 5. Recommended parameters

### Test / CI

| Parameter | Value | Notes |
| --- | ---: | --- |
| `modulus` | `103` (`DEFAULT_TEST_MODULUS`) | Prime, `103 ≡ 3 (mod 5)`, `d = 41` |
| `iterations` | `8`–`64` (typical `16`) | Fast unit tests |
| Constructor | `VdfParams::for_tests(T)` | |

### Production (starting point — must re-benchmark)

| Parameter | Value | Notes |
| --- | ---: | --- |
| `modulus` | `1_000_000_033` (`PRODUCTION_MODULUS`) | Prime, `≡ 3 (mod 5)`, `d = 400_000_013` |
| `iterations` | `50_000_000` (`PRODUCTION_ITERATIONS`) | Target ≈10 min on slow edge after calibration |
| Constructor | `VdfParams::production()` | |

**Prior fix note:** production modulus was chosen / corrected to satisfy `p ≡ 3 (mod 5)` so classical `d = (2p − 1)/5` is integral and `5d ≡ 1 (mod p−1)`. Do not replace with a round decimal that is `≡ 1 (mod 5)`.

### Future (not yet in this module)

| Parameter | Direction |
| --- | --- |
| Field | Pallas/Vesta-scale prime, big-int `mod_exp` |
| Proof | Optional Wesolowski short proof for fast foreign verify |
| Delay | Keep ~10 min Job DID burn; HTLC cancel may use shorter `T` |

---

## 6. Security claims: proven vs heuristic

| Statement | Status |
| --- | --- |
| If `p ≡ 3 (mod 5)`, then `d = (2p−1)/5` is an integer and `5d ≡ 1 (mod p−1)` | **Proven** (elementary number theory) |
| If `p ≢ 1 (mod 5)` and `p ≥ 5`, `5` is invertible mod `p−1` | **Proven** |
| Evaluating the chain is deterministic; verify detects wrong `final_x` / wrong `T` (when re-executing) | **Proven** relative to implementation |
| `JobDID` is collision-resistant if SHA-256 is | **Standard hash assumption** |
| No parallel algorithm evaluates one chain substantially faster than sequential depth `T` | **Heuristic** (MinRoot / iterated algebraic map sequentiality) |
| 50M iterations ≈ 10 minutes on fleet hardware | **Engineering measurement**, not a theorem |
| Prevents all Sybil / spam | **False** — only raises marginal cost per DID; wealthy attackers buy CPU-time |

### Threat model (brief)

- **In scope:** Cheap mass minting of Job DIDs on one machine via parallelizing a *single* proof’s steps; silent acceptance of `p ≡ 1 (mod 5)` “proofs”; DID substitution for different `job_meta`.
- **Out of scope for this module alone:** Adaptive adversaries with huge sequential ASIC farms; network-layer spam; sandbox bypass without admit gate; short-proof forgeability (N/A — no short proof yet).

---

## 7. Sybil cost model (brief)

Let `C_seq` be the wall-clock cost of one honest evaluation at parameters `(p, T)` on the attacker’s best **single sequential pipeline**.

| Quantity | Expression | Comment |
| --- | --- | --- |
| Cost per Job DID | `≈ C_seq` | Plus negligible SHA-256 |
| Cost for `N` independent DIDs | `≈ N · C_seq / P` | `P` = number of parallel sequential pipelines (cores/ASICs) |
| Parallelism *within* one proof | `≈ 1` (depth) | Goal of sequential map |
| Mesh effect | Jobs require verified burn | Raises cost of fake worker flood / identity spam |

**Design intent:** ~10 minutes of single-core work per ephemeral job identity so that spinning thousands of fake workers is expensive in real time and energy, without requiring a trusted central rate limiter.

**Limitation:** An attacker with `P` cores still mints at rate `P / C_seq` DIDs per unit time. Pair with WoT staking, VRF assignment, honeypots, and escrow slashing (`AGENTS.md` security / governance agents) — VDF is a **rate/cost brake**, not a sole root of trust.

---

## 8. Implementation invariants (tests)

Unit tests in `vdf_sybil.rs` enforce:

1. Invalid modulus `p ≡ 1 (mod 5)` → exponent `None`, evaluate `None`, verify `false`.
2. Wrong `final_x` → verify fails.
3. Wrong / mismatched steps → verify fails.
4. Prove/verify determinism for fixed `(input, params)`.
5. DID stability: fixed `(output, meta)` ⇒ fixed 32-byte digest; big-endian encoding.
6. Production surface: `PRODUCTION_MODULUS ≡ 3 (mod 5)` and classical `d`.

---

## 9. References (external literature)

- MinRoot VDF design discussions (iterated roots in prime fields; sequentiality heuristics).
- Boneh–Bünz–Fisch et al., verifiable delay functions (general VDF definitions; Wesolowski / Pietrzak proofs for *fast verify* — **not** implemented here).
- Project docs: `README.md` (Phase 2 VDF Job DID), `docs/interface-contracts.md` (`VdfProof`, admit gate), `docs/sla-and-dod.md` (P2-DoD VDF).
