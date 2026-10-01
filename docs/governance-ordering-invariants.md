# Governance ordering invariants

The `governance` crate was removed from the workspace per issue #166 (PR #240).
If it is ever reintroduced, the web-of-trust onboarding and slashing paths must
preserve the two ordering invariants below. Both were discovered and fixed in
PR #239 (issues #164 and #165) against `governance/src/wot.rs`; the code fixes
were dropped when the crate was removed, so this document is the record of the
required behavior.

## 1. Validate-before-blacklist in `mark_malicious_and_slash` (was #164)

The voucher edge MUST be validated BEFORE the node is added to the malicious
blacklist. A failed call must leave no side effects.

Correct order in `mark_malicious_and_slash(invitee)`:

1. Return `WotError::UnknownInvitee` if the node is neither onboarded nor the
   invitee of any voucher edge.
2. Look up the voucher edge for `invitee`; return `WotError::EdgeNotFound`
   with the blacklist untouched if it does not exist.
3. Return `WotError::AlreadySlashed` if the edge is already slashed, again
   with the blacklist untouched.
4. Only then insert `invitee` into `malicious`, mark the edge slashed, and
   call `staking.slash`.

The old code inserted into `malicious` first and validated the edge second, so
a genesis node (onboarded, but with no voucher edge) failed with `EdgeNotFound`
yet stayed permanently marked malicious: it could never be slashed, and it
could never onboard again.

Regression check: onboard a genesis node with no voucher edge, call
`mark_malicious_and_slash(root)`; expect `Err(EdgeNotFound)` AND
`is_malicious(root) == false`.

## 2. Consume-nonce-after-lock in `onboard` (was #165)

Freshness MUST be checked before the staking lock, but the consent nonce MUST
be consumed only after the lock succeeds. A failed onboard must not burn the
nonce.

Correct order in `onboard(consent)`:

1. Return `WotError::MaliciousInvitee` if the voucher is blacklisted.
2. Check `consumed_consents.contains(consent.nonce)`; return
   `WotError::ConsentReplayed` if already used.
3. Return `WotError::AlreadyOnboarded` if the invitee is already onboarded.
4. Call `staking.lock(voucher, power_units)`; propagate its error.
5. Only on success: `consumed_consents.insert(consent.nonce)`, then create the
   voucher edge.

The old code inserted the nonce into `consumed_consents` before the lock, so a
failed onboard (already-onboarded retry, staking error) burned the nonce: the
legitimate retry then failed with `ConsentReplayed` and the voucher had to
re-issue consent with a fresh nonce.

Regression checks:

- Onboard bob, then retry onboard for the already-onboarded bob with a FRESH
  nonce: expect `Err(AlreadyOnboarded)`, not `ConsentReplayed`, and the fresh
  nonce must remain usable for a different invitee (carol).
- Onboard dave with `power_units = 0` so `staking.lock` fails with
  `StakingError::ZeroPower`: the nonce must survive. Calling onboard with the
  same consent again must still yield the staking error, not `ConsentReplayed`.
