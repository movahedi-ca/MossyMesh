# Wesolowski VDF — parameter theory & security claims

**Owner module:** `mesh-transport/src/vdf_sybil.rs`
**Consumers:** sandbox admit gate (`sandbox/src/admit.rs`, duplicated arithmetic, no circular dep), job DIDs, HTLC VDF-delayed cancel (mock in interop).
**Agent:** 08 Cryptography (`AGENTS.md`).

This note freezes the mathematical surface used for Sybil-costed Ephemeral Job DIDs. It distinguishes **proven algebraic facts**, **heuristic sequentiality**, and **engineering calibration**.

**History:** this module previously used a MinRoot-style iterated map whose verification re-ran all `T` steps. That made rejecting a bogus proof as expensive as evaluating it, a CPU denial-of-service amplifier (issue #199): an attacker could mint random outputs for free and force every verifier to burn the full delay on each one. The Wesolowski construction below has **fast verification** (milliseconds, independent of `T`), which closes that amplifier.

---

## 1. Construction definition

Work in `(Z/NZ)*` where `N` is the RSA-2048 challenge modulus (see section 2). Public inputs: 64-bit seed `input`, delay `t = params.iterations`.

```text
x  = hash_to_group(input, t)      # SHA-256 counter mode -> [2, N-1]
y  = x^(2^t) mod N                # t sequential squarings (evaluation)
l  = H_prime(x, y, t)             # Fiat-Shamir 128-bit prime (challenge)
pi = x^floor(2^t / l) mod N       # one extra sequential pass (proof)
```

Verification checks, with `r = 2^t mod l`:

```text
pi^l * x^r = y   (mod N)
```

`hash_to_group`: 8x SHA-256(`domain || 0x01 || input_be8 || t_be8 || counter_be4`) concatenated to 2048 bits, reduced into `[2, N-1]`. Deterministic, binds the group element to `t`.

`H_prime`: SHA-256(`domain || 0x02 || x_be256 || y_be256 || t_be8 || counter_be4`); take the first 16 bytes with bits 127 and 0 set, increment the counter until the candidate passes trial division (primes < 1000) and 12 Miller-Rabin rounds with fixed prime bases. Because `l` is hashed from the *claimed* `y`, the prover cannot grind a favorable challenge.

Domain: `b"mossymesh.vdf.wesolowski.v1"` for all hashes in both crates.

### Why verification is fast

Verification recomputes `x` (8 hashes) and `l` (one prime search, ~milliseconds), computes `r = 2^t mod l` by fast modular exponentiation (`O(log t)` small-integer ops), then checks `pi^l * x^r = y (mod N)` with two modular exponentiations at ~128-bit exponents. **There is no loop over `t` anywhere in the verify path.** Measured cost is milliseconds, independent of the claimed iteration count. Rejection of garbage therefore costs a fixed small amount (see section 7).

### Proof computation (binary long division in the exponent)

`pi = x^floor(2^t / l)` is computed in a second sequential pass maintaining the invariant `pi_i = x^floor(2^i / l)`, `r_i = 2^i mod l`: each step squares `pi` (and `r`), and multiplies `pi` by `x` when the doubled remainder overflows `l`. Total honest prover work is on the order of 2.5 sequential 2048-bit modular multiplications per step.

---

## 2. Group of unknown order: which N, and the trust assumption

`N` is the **RSA-2048 challenge integer**: the 617-digit semiprime published by RSA Laboratories for the RSA Factoring Challenge (2001, US$200,000 prize; retired in 2007 with this number still unfactored, and still unfactored today). Registered as `MODULUS_ID_RSA2048_CHALLENGE = 1` (mirrored in sandbox as `MODULUS_ID_WESOLOWSKI_RSA2048`; a cross-crate test pins the ids equal).

Why this `N`:

- **Nothing up my sleeve.** RSA Laboratories generated the challenge numbers on an offline computer and destroyed the hard drive afterwards, so no party is known to hold the factors. No setup ceremony, no generated parameters, no "generate and forget the factors" honesty to take on faith.
- **Security level.** A 2048-bit RSA modulus carries roughly 112 bits of factoring security (NIST SP 800-57). The Fiat-Shamir challenge is 128 bits, so the construction targets the lower of the two: **about 112-bit security**. Appropriate for a delay gate, which protects rate rather than long-term confidentiality.

**Trust assumption, stated plainly:** factoring this `N` stays infeasible. Anyone who learns the factors reduces the exponent `2^t` modulo `phi(N)` and evaluates the VDF in `O(log t)` time, minting identity proofs without the delay and collapsing the Sybil gate. If this `N` is ever factored, or 128-bit factoring security is required, rotate to a fresh 3072-bit RSA modulus from a trusted multi-party ceremony under a new `MODULUS_ID_*` value; the id registry exists so rotation does not change the proof format.

The constant is pinned by a decimal-string checksum test (all 617 digits, cross-checked against Wikipedia "RSA numbers" and the MTC3/RSA Inc challenge PDF) against transcription corruption.

---

## 3. Ephemeral Job DID

```text
JobDID = SHA-256( VDF_output_bytes || job_meta )
```

- `VDF_output_bytes` = `y` as **256-byte big-endian** (`VdfProof.output`).
- `job_meta` is application-defined opaque bytes (job payload commitment, submitter, etc.).
- Same `(output, meta)` => same DID (deterministic, stable).
- The sandbox admit gate duplicates the Wesolowski arithmetic (it cannot depend on mesh-transport); the duplication is pinned by a cross-crate consistency test in `mesh-transport` (transport-issued proofs admit through the sandbox verifier, sandbox-issued receipts verify in transport). The sandbox also ships a domain-separated hash PoW **stub** (`DomainSeparatedHashVdfStub`) for admit-path tests that must not burn sequential work; it is not Sybil-hard.

---

## 4. Parameter selection checklist (~10 min delay)

Use this checklist before freezing consensus / mainnet parameters.

1. **Group**
   - [ ] Modulus is the RSA-2048 challenge integer (checksum test green).
   - [ ] `modulus_id` registry agrees across `mesh-transport` and `sandbox` (cross-crate test green).
2. **Iteration count `t`**
   - [ ] Benchmark **single-core** `evaluate_vdf` on the **slowest supported class** (Pi Zero 2 W class).
   - [ ] Target wall-clock **~= 600 s +/- tolerance (e.g. 8-12 min)** for honest nodes.
   - [ ] Record: CPU model, clock, rustc version, `t`, measured seconds, iterations/sec.
   - [ ] Set consensus `t` from the slow tier so laptops cannot mint DIDs in seconds relative to phones/Pis unless that asymmetry is explicitly accepted.
3. **Verify policy**
   - [ ] Reject unregistered `modulus_id`, zero iterations, iteration under-claims (`claimed_iterations != params.iterations`), non-canonical encodings (wrong length, zero, `>= N`), and equation failures.
   - [ ] Reject garbage in a fixed small cost: the timing tests (`garbage_proof_rejected_in_fraction_of_eval_time`, `verify_is_a_small_fraction_of_evaluate`) must stay green.
   - [ ] Bind DID: `SHA-256(y_be256 || job_meta)`.
4. **Operational**
   - [ ] Publish `(modulus_id -> (N, t))` so all islands agree.
   - [ ] Do not treat the sandbox hash stub as Sybil-hard.

### Rough cost model (2048-bit modular multiplies)

Per step: ~1 squaring (evaluation) or ~1.5 mults (proof pass). `t = 50_000_000` => ~`1.25 x 10^8` sequential 2048-bit multiplies. Measured single-squaring cost is ~9us on reference x86-64 (release, portable `num-bigint` backend), putting the full proof in the tens-of-minutes range there; edge silicon is slower. **Always re-benchmark** rather than trusting this back-of-envelope.

---

## 5. Recommended parameters

### Test / CI

| Parameter | Value | Notes |
| --- | --- | --- |
| `modulus_id` | `1` (`MODULUS_ID_RSA2048_CHALLENGE`) | RSA-2048 challenge group |
| `iterations` | `8`-`64` (typical `16`) | Fast unit tests |
| Constructor | `VdfParams::for_tests(t)` | Clamped to `1..=10_000` |

### Production (starting point -- must re-benchmark)

| Parameter | Value | Notes |
| --- | --- | --- |
| `modulus_id` | `1` | RSA-2048 challenge group |
| `iterations` | `50_000_000` (`PRODUCTION_ITERATIONS`) | Target ~10 min on slow edge after calibration |
| Constructor | `VdfParams::production()` | |

`PRODUCTION_ITERATIONS` must never appear on a test path (tests use `VdfParams::for_tests` / `DEFAULT_TEST_ITERATIONS`); the clamp is enforced by construction.

---

## 6. Security claims: proven vs heuristic

| Statement | Status |
| --- | --- |
| The verify equation holds for honestly generated `(x, y, pi)` | **Proven** (algebra; also pinned by honest-proof tests) |
| Verification cost is independent of `t` | **Proven** relative to implementation (no loop over `t`; pinned by timing tests) |
| Garbage proofs are rejected in milliseconds regardless of claimed `t` | **Proven** relative to implementation (pinned by timing tests) |
| `JobDID` is collision-resistant if SHA-256 is | **Standard hash assumption** |
| The Fiat-Shamir challenge `l` is a 128-bit prime bound to `(x, y, t)` | **Proven** relative to implementation (deterministic prime search; pinned by tests) |
| No parallel algorithm evaluates one chain substantially faster than sequential depth `t` | **Heuristic** (repeated-squaring sequentiality; the standard VDF assumption) |
| Passing verification without the sequential work implies an `l`-th root computation | **Reduces to the adaptive root assumption** (Wesolowski 2019, Section 3) |
| Factoring the RSA-2048 challenge modulus is infeasible | **Assumption** (112-bit security per NIST; nothing-up-my-sleeve generation) |
| 50M iterations ~= 10 minutes on fleet hardware | **Engineering measurement**, not a theorem |
| Prevents all Sybil / spam | **False** -- only raises marginal cost per DID; wealthy attackers buy CPU-time |

### Threat model (brief)

- **In scope:** Cheap mass minting of Job DIDs via parallelizing a *single* proof's steps; CPU-DoS by forcing verifiers to burn full delays on bogus proofs (**closed by fast verification**, issue #199); iteration under-claims; modulus substitution (unregistered group ids rejected); DID substitution for different `job_meta`; transcription corruption of the modulus constant (checksum test).
- **Out of scope for this module alone:** Factorization of `N` (collapses the gate; see rotation plan in section 2); adaptive adversaries with huge sequential ASIC farms; network-layer spam; sandbox bypass without the admit gate.

---

## 7. Sybil cost model (brief)

Let `C_seq` be the wall-clock cost of one honest evaluation at parameters `(N, t)` on the attacker's best **single sequential pipeline**.

| Quantity | Expression | Comment |
| --- | --- | --- |
| Cost per Job DID | `~= C_seq` | Plus negligible proving overhead (~1.5x) and SHA-256 |
| Cost for `M` independent DIDs | `~= M . C_seq / P` | `P` = number of parallel sequential pipelines (cores/ASICs) |
| Parallelism *within* one proof | `~= 1` (depth) | Goal of the sequential construction |
| Cost to force one garbage verification | `~= milliseconds` | **Independent of claimed `t`** (issue #199 fix) |
| Mesh effect | Jobs require verified burn | Raises cost of fake worker flood / identity spam |

**Design intent:** ~10 minutes of single-core work per ephemeral job identity so that spinning thousands of fake workers is expensive in real time and energy, without requiring a trusted central rate limiter. Verification asymmetry (minutes to mint, milliseconds to check) means verifiers cannot be CPU-exhausted by bogus receipts.

**Limitation:** An attacker with `P` cores still mints at rate `P / C_seq` DIDs per unit time. Pair with WoT staking, VRF assignment, honeypots, and escrow slashing (`AGENTS.md` security / governance agents) -- VDF is a **rate/cost brake**, not a sole root of trust.

---

## 8. Implementation invariants (tests)

Unit tests in `vdf_sybil.rs` and `sandbox/src/admit.rs` enforce:

1. Honest proofs verify; evaluation is deterministic for fixed `(input, params)`.
2. `hash_to_group` is deterministic, in `[2, N-1]`, and binds both seed and `t`.
3. Fiat-Shamir challenge is a deterministic 128-bit prime bound to `(x, y, t)`.
4. Tampered output / mutated proof bytes / malformed encodings (wrong length, zero, `>= N`) -> reject with `OutputMismatch` / `InvalidVdf`.
5. Iteration under-claim (`claimed_iterations != params.iterations`) -> reject.
6. Zero iterations -> reject. Unknown `modulus_id` -> reject.
7. **Timing (issue #199):** garbage rejection and honest verification each take a small fraction of evaluation time at `t = 8_000` / `t = 10_000`, so verification can never regress to re-running the sequential steps.
8. Production modulus is the RSA-2048 challenge integer (617-digit checksum).
9. DID stability: fixed `(output, meta)` => fixed 32-byte digest; different meta or output => different DID.
10. Cross-crate: transport proofs admit through the sandbox verifier and vice versa; group ids agree.
11. Test/production parameter separation: `for_tests` clamps to `MAX_TEST_ITERATIONS`; production surface constants asserted.

---

## 9. References (external literature)

- Benjamin Wesolowski, "Efficient Verifiable Delay Functions", EUROCRYPT 2019 (construction, adaptive root assumption).
- Boneh-Bunz-Fisch et al., "Verifiable Delay Functions" (VDF definitions; Pietrzak alternative).
- RSA Laboratories, RSA Factoring Challenge (challenge number generation; offline computer, destroyed media).
- NIST SP 800-57 Part 1 (112-bit security for 2048-bit factoring).
- Project docs: `README.md` (Phase 2 VDF Job DID), `docs/interface-contracts.md` (`VdfProof`, admit gate), `docs/sla-and-dod.md` (P2-DoD VDF).
