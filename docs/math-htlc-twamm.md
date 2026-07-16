# Mathematical Invariants: HTLC, TWAMM, Liquidity Bridge

Formal statements of the MossyMesh interop economic/safety invariants.
Implementation lives in `interop/src/{htlc,credits,twamm,liquidity}.rs`.

Notation:

- \( \mathbb{N}_0 = \{0,1,2,\ldots\} \)
- Prices and credit amounts are non-negative integers (fixed-point scale \(10^6\) where noted).
- Basis points (bps): \( 1\,\mathrm{bp} = 10^{-4} \) of the reference price.

---

## 1. HTLC: hashlock + VDF/timelock delayed cancel

### 1.1 State space

An HTLC \(H\) has state

\[
\sigma(H) \in \{\mathsf{Funded},\, \mathsf{Claimed},\, \mathsf{Refunded},\, \mathsf{VdfCancelled}\}.
\]

Terminal (settled) states: \( \mathcal{T} = \{\mathsf{Claimed},\, \mathsf{Refunded},\, \mathsf{VdfCancelled}\} \).

### 1.2 Hashlock (claim)

Let \( h = H.\mathsf{payment\_hash} \in \{0,1\}^{256} \) and preimage \( p \in \{0,1\}^* \).

\[
\mathsf{claim}(H, p) \text{ succeeds} \iff
\sigma(H)=\mathsf{Funded}
\;\wedge\;
\mathrm{SHA256}(p) = h.
\]

On success: \( \sigma(H) \leftarrow \mathsf{Claimed} \), and \( H.\mathsf{claimed\_preimage} = p \).

**Invariant H1 (hashlock integrity).**  
No transition to \(\mathsf{Claimed}\) exists without a preimage verifying under SHA-256.

**Invariant H2 (wrong preimage is a no-op).**  
If \( \mathrm{SHA256}(p) \neq h \), then \( \sigma(H) \) is unchanged and no credits leave escrow.

### 1.3 Timeout refund

Let \( t^\star = H.\mathsf{timeout\_height} \), \( t = \) current height.

\[
\mathsf{refund}(H, t) \text{ succeeds} \iff
\sigma(H)=\mathsf{Funded}
\;\wedge\;
t \ge t^\star.
\]

On success: \( \sigma(H) \leftarrow \mathsf{Refunded} \).

**Invariant H3 (timelock).**  
\( t < t^\star \Rightarrow \mathsf{refund} \) fails with \(\mathsf{TimeoutNotReached}\).

### 1.4 VDF-delayed cancel

Let \( V \) be the attached sequential mock VDF with required steps \( s^\star \) and completed steps \( s \).

\[
V.\mathsf{is\_complete} \iff s \ge s^\star.
\]

\[
\mathsf{vdf\_cancel}(H) \text{ succeeds} \iff
\sigma(H)=\mathsf{Funded}
\;\wedge\;
V.\mathsf{is\_complete}.
\]

On success: \( \sigma(H) \leftarrow \mathsf{VdfCancelled} \).

**Invariant H4 (VDF gate).**  
\( s < s^\star \Rightarrow \mathsf{vdf\_cancel} \) fails with \(\mathsf{VdfNotComplete}\).  
Steps are sequential (\( x_{i+1} = f(x_i, i) \)); completing \( s^\star \) steps cannot be parallelized in the real MinRoot VDF (mock used only in tests).

**Invariant H5 (single settlement).**  
From \(\sigma \in \mathcal{T}\), every further \(\mathsf{claim}\), \(\mathsf{refund}\), or \(\mathsf{vdf\_cancel}\) fails with \(\mathsf{AlreadySettled}\).  
Equivalently: there is at most one successful settlement transition per HTLC.

### 1.5 Credit ledger: no double-spend of escrowed credits

Let free balances \( B(a) \in \mathbb{N}_0 \) for account \( a \), and open escrows \( E \) with amounts \( A(e) \).

**Total conserved supply** after a sequence of ops that exclude `mint`:

\[
\sum_a B(a) + \sum_{e:\,\sigma(e)=\mathsf{Funded}} A(e)
= \text{constant}.
\]

On open: \( B(\mathsf{sender}) \leftarrow B(\mathsf{sender}) - A \), escrow holds \( A \).  
On claim: escrow releases \( A \) to \( B(\mathsf{receiver}) \).  
On refund / VDF cancel: escrow releases \( A \) to \( B(\mathsf{sender}) \).

**Invariant H6 (no double-spend).**  
Each escrow amount \( A \) is released to free balance **exactly once**.  
Because of H5, a second claim/refund/cancel cannot re-credit \( A \).

**Invariant H7 (unique escrow id).**  
Opening an escrow with an existing id fails (`DuplicateEscrow`); the same credits cannot be locked twice under one id.

**Invariant H8 (funded amount).**  
\( A > 0 \) at open; zero-amount HTLCs are rejected (`InvalidAmount`).

---

## 2. TWAMM: max-spread ≤ 2% (200 bps)

### 2.1 Spread definition

Given reference mid price \( r > 0 \) and execution price \( e > 0 \) (both scale \(10^6\)):

\[
\mathrm{spread\_bps}(e, r)
\;=\;
\left\lfloor
\frac{|e - r| \cdot 10\,000}{r}
\right\rfloor
\in \mathbb{N}_0.
\]

Implementation uses integer arithmetic (`saturating_mul` then `checked_div`), so the
floor is the native integer quotient. The acceptance rule is defined on this
**integer** bps, not on a floating-point percentage.

### 2.2 Hard cap

\[
\mathrm{MAX\_SPREAD\_BPS} = 200
\quad\Leftrightarrow\quad
2.00\%.
\]

**Invariant T1 (max-spread inequality).**  
A stream slice at price \( e \) against reference \( r \) is accepted iff

\[
\mathrm{spread\_bps}(e, r) \;\le\; 200.
\]

Equivalently, rejection:

\[
\mathrm{spread\_bps}(e, r) \;>\; 200
\;\Rightarrow\;
\mathsf{SpreadExceeded}.
\]

Discrete price boundaries (example \( r = 10^6 \)):

\[
\mathrm{spread\_bps}=200 \iff 20\,000 \le |e-r| < 20\,100,
\qquad
\mathrm{spread\_bps}\ge 201 \iff |e-r| \ge 20\,100.
\]

Boundary cases:

| \( e \) relative to \( r=10^6 \) | integer bps | Decision |
| --- | --- | --- |
| \( e = r \) | 0 | accept |
| \( |e-r| = 20\,000 \) (exact 2% in reals) | 200 | accept |
| \( |e-r| = 20\,099 \) | 200 | accept (floor) |
| \( |e-r| = 20\,100 \) | 201 | **reject** |

**Invariant T2 (rejection is non-mutating).**  
If a slice is rejected for spread, order `remaining_in` and `slices_remaining` are unchanged.

**Invariant T3 (invalid prices).**  
\( r = 0 \) or \( e = 0 \) ⇒ `InvalidPrice` (spread undefined).

**Invariant T4 (order exhaustion).**  
If `slices_remaining = 0` or `remaining_in = 0`, further slices fail with `OrderExhausted`.

### 2.3 Economic reading

The 2% cap bounds adverse selection when bridging local mesh liquidity to a global AMM: no TWAMM mini-fill may clear more than 200 bps from the reference mid used at order submit.

---

## 3. Liquidity bridge: offline → online airdrop points

### 3.1 Accrual (offline only)

Constants:

\[
P_{\mathrm{epoch}} = 100,\qquad
T_{\mathrm{point}} = 1000
\quad\text{(token micro-units per point)}.
\]

For a genesis node \( n \) while the mesh is **offline** (`internet_reconnected = false`):

\[
\Delta \mathrm{points}(n, k) = k \cdot P_{\mathrm{epoch}},\qquad k \in \mathbb{N}_0.
\]

**Invariant L1 (offline-only accrual).**  
If `internet_reconnected`, then \( \Delta \mathrm{points} = 0 \).

**Invariant L2 (genesis-only retroactive points).**  
Non-genesis accounts cannot accrue (`NotGenesis`).

### 3.2 Claim (online only)

Unclaimed points:

\[
U(n) = \mathrm{points}(n) - \left\lfloor \frac{\mathrm{claimed\_tokens}(n)}{T_{\mathrm{point}}} \right\rfloor.
\]

Claim succeeds iff online and \( U(n) > 0 \):

\[
\mathrm{tokens} = U(n) \cdot T_{\mathrm{point}},\qquad
\mathrm{claimed\_tokens}(n) \mathrel{+}= \mathrm{tokens}.
\]

**Invariant L3 (claim requires reconnect).**  
Offline claim fails with `StillOffline`.

**Invariant L4 (no double claim of the same points).**  
After a full claim, \( U(n) = 0 \); a second claim fails with `NothingToClaim`.

### 3.3 Conservation notes

Let \(\mathcal{G}\) be the set of accounts.

**Per-account point conservation (L5):**

\[
\forall n:\quad
\left\lfloor \frac{\mathrm{claimed\_tokens}(n)}{T_{\mathrm{point}}} \right\rfloor + U(n)
= \mathrm{points}(n).
\]

(Exact when `claimed_tokens` is always a multiple of \( T_{\mathrm{point}} \), which the claim path enforces.)

**Network conservation (L6):**

\[
\sum_{n \in \mathcal{G}} \mathrm{points}(n)
= \mathrm{total\_points\_issued}
\quad\text{(when points only increase via `accrue_offline_epochs`)}.
\]

\[
\sum_{n \in \mathcal{G}} \mathrm{claimed\_tokens}(n)
= \mathrm{total\_tokens\_airdropped}.
\]

**Token–point link (L7):**

\[
\mathrm{total\_tokens\_airdropped}
= T_{\mathrm{point}} \cdot \sum_{n} \left\lfloor \frac{\mathrm{claimed\_tokens}(n)}{T_{\mathrm{point}}} \right\rfloor
\le T_{\mathrm{point}} \cdot \mathrm{total\_points\_issued}.
\]

Points are **not** destroyed on claim; they remain as an audit trail while `claimed_tokens` tracks converted airdrop. Unclaimed residual \( U(n) \) is what remains convertible after reconnect.

---

## 4. Test map

| Invariant | Primary tests |
| --- | --- |
| H1–H5 | `htlc::tests::*`, `htlc::tests::invariant_*` |
| H6–H8 | `credits::tests::*`, `credits::tests::invariant_*` |
| T1–T4 | `twamm::tests::*`, `twamm::tests::invariant_*` |
| L1–L7 | `liquidity::tests::*`, `liquidity::tests::invariant_*` |

---

## 5. Implementation constants (source of truth)

| Symbol | Code | Value |
| --- | --- | --- |
| Max spread | `twamm::MAX_SPREAD_BPS` | 200 |
| BPS denom | `10_000` | — |
| Points / offline epoch | `liquidity::POINTS_PER_OFFLINE_EPOCH` | 100 |
| Tokens / point | `liquidity::TOKENS_PER_POINT` | 1_000 |
| Hash | `htlc::hash_preimage` | SHA-256 |
| VDF (test) | `htlc::MockVdf` | sequential mock steps |
