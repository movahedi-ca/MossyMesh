//! Escrowed compute credits backed by HTLCs with VDF-delayed cancellation.
//!
//! Parties hold free balances; locking funds into an [`Htlc`] moves them into
//! escrow until claim (receiver), timeout refund (sender), or VDF cancel (sender).

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::htlc::{Htlc, HtlcError, HtlcParams, HtlcState};

/// Errors from credit ledger / escrow operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CreditError {
    /// Account does not exist in the ledger.
    UnknownAccount(String),
    /// Free balance is insufficient to fund escrow.
    InsufficientBalance { available: u64, required: u64 },
    /// Escrow id is already registered.
    DuplicateEscrow([u8; 32]),
    /// No escrow found for the given id.
    UnknownEscrow([u8; 32]),
    /// Wrapped HTLC state-machine error.
    Htlc(HtlcError),
}

impl std::fmt::Display for CreditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CreditError::UnknownAccount(a) => write!(f, "unknown account: {a}"),
            CreditError::InsufficientBalance {
                available,
                required,
            } => write!(f, "insufficient balance: have {available}, need {required}"),
            CreditError::DuplicateEscrow(id) => {
                write!(f, "duplicate escrow id: {}", hex::encode(id))
            }
            CreditError::UnknownEscrow(id) => {
                write!(f, "unknown escrow id: {}", hex::encode(id))
            }
            CreditError::Htlc(e) => write!(f, "htlc error: {e}"),
        }
    }
}

impl std::error::Error for CreditError {}

impl From<HtlcError> for CreditError {
    fn from(value: HtlcError) -> Self {
        CreditError::Htlc(value)
    }
}

/// Minimal hex encode for error display without pulling a hex crate dependency.
mod hex {
    pub fn encode(bytes: &[u8; 32]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }
}

/// Per-account free compute credit balance.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    pub free: u64,
}

/// Ledger of free balances plus open / settled HTLC escrows.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CreditLedger {
    accounts: HashMap<String, Account>,
    escrows: HashMap<[u8; 32], Htlc>,
}

impl CreditLedger {
    pub fn new() -> Self {
        Self::default()
    }

    /// Credit free balance (e.g. job payout or genesis mint). Creates account if needed.
    pub fn mint(&mut self, account: impl Into<String>, amount: u64) {
        let acct = self.accounts.entry(account.into()).or_default();
        acct.free = acct.free.saturating_add(amount);
    }

    /// Free (non-escrowed) balance for an account, or 0 if unknown.
    pub fn free_balance(&self, account: &str) -> u64 {
        self.accounts.get(account).map(|a| a.free).unwrap_or(0)
    }

    /// Open HTLC escrows (still in `Funded` state).
    pub fn open_escrows(&self) -> impl Iterator<Item = &Htlc> {
        self.escrows.values().filter(|h| h.is_open())
    }

    /// Lookup an escrow by id.
    pub fn escrow(&self, id: &[u8; 32]) -> Option<&Htlc> {
        self.escrows.get(id)
    }

    /// Lock `params.amount` from the sender into a new HTLC escrow.
    pub fn open_escrow(&mut self, params: HtlcParams) -> Result<[u8; 32], CreditError> {
        if self.escrows.contains_key(&params.id) {
            return Err(CreditError::DuplicateEscrow(params.id));
        }
        let sender = params.sender.clone();
        let amount = params.amount;
        let account = self
            .accounts
            .get_mut(&sender)
            .ok_or_else(|| CreditError::UnknownAccount(sender.clone()))?;
        if account.free < amount {
            return Err(CreditError::InsufficientBalance {
                available: account.free,
                required: amount,
            });
        }

        let htlc = Htlc::fund(params)?;
        account.free -= amount;
        let id = htlc.id;
        self.escrows.insert(id, htlc);
        Ok(id)
    }

    /// Receiver claims escrow with the SHA-256 preimage; credits move to receiver free balance.
    pub fn claim_escrow(&mut self, id: &[u8; 32], preimage: &[u8]) -> Result<u64, CreditError> {
        let htlc = self
            .escrows
            .get_mut(id)
            .ok_or(CreditError::UnknownEscrow(*id))?;
        htlc.claim(preimage)?;
        let amount = htlc.amount;
        let receiver = htlc.receiver.clone();
        self.credit_free(&receiver, amount);
        Ok(amount)
    }

    /// Sender refunds after timeout; credits return to sender free balance.
    pub fn refund_escrow(
        &mut self,
        id: &[u8; 32],
        current_height: u64,
    ) -> Result<u64, CreditError> {
        let htlc = self
            .escrows
            .get_mut(id)
            .ok_or(CreditError::UnknownEscrow(*id))?;
        htlc.refund(current_height)?;
        let amount = htlc.amount;
        let sender = htlc.sender.clone();
        self.credit_free(&sender, amount);
        Ok(amount)
    }

    /// Advance the mock VDF attached to an open escrow.
    pub fn advance_vdf(&mut self, id: &[u8; 32], steps: u64) -> Result<(), CreditError> {
        let htlc = self
            .escrows
            .get_mut(id)
            .ok_or(CreditError::UnknownEscrow(*id))?;
        htlc.advance_vdf(steps)?;
        Ok(())
    }

    /// VDF-delayed cancel: after sequential delay, credits return to sender.
    pub fn vdf_cancel_escrow(&mut self, id: &[u8; 32]) -> Result<u64, CreditError> {
        let htlc = self
            .escrows
            .get_mut(id)
            .ok_or(CreditError::UnknownEscrow(*id))?;
        htlc.vdf_cancel()?;
        let amount = htlc.amount;
        let sender = htlc.sender.clone();
        self.credit_free(&sender, amount);
        Ok(amount)
    }

    fn credit_free(&mut self, account: &str, amount: u64) {
        let acct = self.accounts.entry(account.to_string()).or_default();
        acct.free = acct.free.saturating_add(amount);
    }

    /// Snapshot of escrow state for an id.
    pub fn escrow_state(&self, id: &[u8; 32]) -> Option<HtlcState> {
        self.escrows.get(id).map(|h| h.state)
    }

    /// Sum of free balances across all accounts.
    pub fn total_free(&self) -> u64 {
        self.accounts.values().map(|a| a.free).sum()
    }

    /// Sum of amounts still locked in `Funded` escrows.
    pub fn total_escrowed(&self) -> u64 {
        self.escrows
            .values()
            .filter(|h| h.is_open())
            .map(|h| h.amount)
            .sum()
    }

    /// Conservation quantity: free + open-escrowed (docs/math-htlc-twamm.md H6).
    pub fn conserved_supply(&self) -> u64 {
        self.total_free().saturating_add(self.total_escrowed())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::htlc::hash_preimage;

    fn escrow_params(
        id: u8,
        sender: &str,
        receiver: &str,
        amount: u64,
        preimage: &[u8],
    ) -> HtlcParams {
        let mut eid = [0u8; 32];
        eid[0] = id;
        HtlcParams {
            id: eid,
            sender: sender.into(),
            receiver: receiver.into(),
            amount,
            payment_hash: hash_preimage(preimage),
            timeout_height: 100,
            funded_height: 0,
            vdf_steps: 4,
            vdf_seed: Some(7),
        }
    }

    #[test]
    fn escrow_claim_moves_credits_to_receiver() {
        let mut ledger = CreditLedger::new();
        ledger.mint("alice", 5_000);
        ledger.mint("bob", 0);

        let preimage = b"job-result-proof";
        let params = escrow_params(1, "alice", "bob", 1_200, preimage);
        let id = ledger.open_escrow(params).unwrap();

        assert_eq!(ledger.free_balance("alice"), 3_800);
        assert_eq!(ledger.free_balance("bob"), 0);
        assert_eq!(ledger.escrow_state(&id), Some(HtlcState::Funded));

        let paid = ledger.claim_escrow(&id, preimage).unwrap();
        assert_eq!(paid, 1_200);
        assert_eq!(ledger.free_balance("alice"), 3_800);
        assert_eq!(ledger.free_balance("bob"), 1_200);
        assert_eq!(ledger.escrow_state(&id), Some(HtlcState::Claimed));
    }

    #[test]
    fn timeout_refund_returns_credits_to_sender() {
        let mut ledger = CreditLedger::new();
        ledger.mint("alice", 2_000);
        let preimage = b"secret";
        let id = ledger
            .open_escrow(escrow_params(2, "alice", "bob", 500, preimage))
            .unwrap();

        assert!(matches!(
            ledger.refund_escrow(&id, 99),
            Err(CreditError::Htlc(HtlcError::TimeoutNotReached))
        ));

        let refunded = ledger.refund_escrow(&id, 100).unwrap();
        assert_eq!(refunded, 500);
        assert_eq!(ledger.free_balance("alice"), 2_000);
        assert_eq!(ledger.escrow_state(&id), Some(HtlcState::Refunded));
    }

    #[test]
    fn invalid_preimage_leaves_escrow_funded() {
        let mut ledger = CreditLedger::new();
        ledger.mint("alice", 1_000);
        let preimage = b"correct";
        let id = ledger
            .open_escrow(escrow_params(3, "alice", "bob", 100, preimage))
            .unwrap();

        assert!(matches!(
            ledger.claim_escrow(&id, b"wrong"),
            Err(CreditError::Htlc(HtlcError::InvalidPreimage))
        ));
        assert_eq!(ledger.free_balance("alice"), 900);
        assert_eq!(ledger.free_balance("bob"), 0);
        assert_eq!(ledger.escrow_state(&id), Some(HtlcState::Funded));
    }

    #[test]
    fn vdf_cancel_returns_credits_after_delay() {
        let mut ledger = CreditLedger::new();
        ledger.mint("alice", 800);
        let preimage = b"vdf-secret";
        let id = ledger
            .open_escrow(escrow_params(4, "alice", "bob", 300, preimage))
            .unwrap();

        assert!(matches!(
            ledger.vdf_cancel_escrow(&id),
            Err(CreditError::Htlc(HtlcError::VdfNotComplete))
        ));

        ledger.advance_vdf(&id, 4).unwrap();
        let amount = ledger.vdf_cancel_escrow(&id).unwrap();
        assert_eq!(amount, 300);
        assert_eq!(ledger.free_balance("alice"), 800);
        assert_eq!(ledger.escrow_state(&id), Some(HtlcState::VdfCancelled));
    }

    #[test]
    fn insufficient_balance_rejects_open() {
        let mut ledger = CreditLedger::new();
        ledger.mint("alice", 50);
        let err = ledger
            .open_escrow(escrow_params(5, "alice", "bob", 100, b"x"))
            .unwrap_err();
        assert_eq!(
            err,
            CreditError::InsufficientBalance {
                available: 50,
                required: 100
            }
        );
        assert_eq!(ledger.free_balance("alice"), 50);
    }

    /// Ledger-level: second claim does not double-pay the receiver.
    #[test]
    fn no_double_claim_on_escrow() {
        let mut ledger = CreditLedger::new();
        ledger.mint("alice", 1_000);
        ledger.mint("bob", 0);
        let preimage = b"unique-job-proof";
        let id = ledger
            .open_escrow(escrow_params(6, "alice", "bob", 400, preimage))
            .unwrap();

        assert_eq!(ledger.claim_escrow(&id, preimage).unwrap(), 400);
        assert_eq!(ledger.free_balance("bob"), 400);

        assert!(matches!(
            ledger.claim_escrow(&id, preimage),
            Err(CreditError::Htlc(HtlcError::AlreadySettled))
        ));
        // Receiver balance unchanged after rejected second claim.
        assert_eq!(ledger.free_balance("bob"), 400);
        assert_eq!(ledger.escrow_state(&id), Some(HtlcState::Claimed));
    }

    /// VDF cancel before delay is rejected; after delay returns funds once.
    #[test]
    fn vdf_cancel_before_delay_rejected() {
        let mut ledger = CreditLedger::new();
        ledger.mint("alice", 500);
        let id = ledger
            .open_escrow(escrow_params(7, "alice", "bob", 200, b"d"))
            .unwrap();

        assert!(matches!(
            ledger.vdf_cancel_escrow(&id),
            Err(CreditError::Htlc(HtlcError::VdfNotComplete))
        ));
        assert_eq!(ledger.free_balance("alice"), 300);

        ledger.advance_vdf(&id, 4).unwrap();
        assert_eq!(ledger.vdf_cancel_escrow(&id).unwrap(), 200);
        assert_eq!(ledger.free_balance("alice"), 500);

        // No double cancel payout.
        assert!(matches!(
            ledger.vdf_cancel_escrow(&id),
            Err(CreditError::Htlc(HtlcError::AlreadySettled))
        ));
        assert_eq!(ledger.free_balance("alice"), 500);
    }

    // --- Formal invariants (docs/math-htlc-twamm.md §1.5) ---

    /// H6: free + open escrow is conserved across open/claim (no mint).
    #[test]
    fn invariant_h6_supply_conserved_on_claim() {
        let mut ledger = CreditLedger::new();
        ledger.mint("alice", 10_000);
        ledger.mint("bob", 100);
        let supply0 = ledger.conserved_supply();
        assert_eq!(supply0, 10_100);

        let preimage = b"conserve-claim";
        let id = ledger
            .open_escrow(escrow_params(10, "alice", "bob", 2_500, preimage))
            .unwrap();
        assert_eq!(ledger.conserved_supply(), supply0);
        assert_eq!(ledger.total_escrowed(), 2_500);
        assert_eq!(ledger.free_balance("alice"), 7_500);

        ledger.claim_escrow(&id, preimage).unwrap();
        assert_eq!(ledger.total_escrowed(), 0);
        assert_eq!(ledger.conserved_supply(), supply0);
        assert_eq!(ledger.free_balance("bob"), 2_600);
    }

    /// H6: no double-spend — second claim does not re-credit receiver.
    #[test]
    fn invariant_h6_no_double_spend_after_claim() {
        let mut ledger = CreditLedger::new();
        ledger.mint("alice", 1_000);
        let preimage = b"once-only";
        let id = ledger
            .open_escrow(escrow_params(11, "alice", "bob", 400, preimage))
            .unwrap();
        ledger.claim_escrow(&id, preimage).unwrap();
        assert_eq!(ledger.free_balance("bob"), 400);

        let err = ledger.claim_escrow(&id, preimage).unwrap_err();
        assert!(matches!(
            err,
            CreditError::Htlc(HtlcError::AlreadySettled)
        ));
        assert_eq!(ledger.free_balance("bob"), 400);
        assert_eq!(ledger.conserved_supply(), 1_000);
    }

    /// H6: refund and VDF cancel also conserve; dual settlement blocked.
    #[test]
    fn invariant_h6_refund_and_vdf_cancel_conserve() {
        let mut ledger = CreditLedger::new();
        ledger.mint("alice", 5_000);
        let supply0 = ledger.conserved_supply();

        let id_r = ledger
            .open_escrow(escrow_params(12, "alice", "bob", 700, b"refund-path"))
            .unwrap();
        ledger.refund_escrow(&id_r, 100).unwrap();
        assert_eq!(ledger.conserved_supply(), supply0);
        assert_eq!(
            ledger.refund_escrow(&id_r, 100).unwrap_err(),
            CreditError::Htlc(HtlcError::AlreadySettled)
        );

        let id_v = ledger
            .open_escrow(escrow_params(13, "alice", "bob", 300, b"vdf-path"))
            .unwrap();
        ledger.advance_vdf(&id_v, 4).unwrap();
        ledger.vdf_cancel_escrow(&id_v).unwrap();
        assert_eq!(ledger.conserved_supply(), supply0);
        assert_eq!(
            ledger.claim_escrow(&id_v, b"vdf-path").unwrap_err(),
            CreditError::Htlc(HtlcError::AlreadySettled)
        );
        assert_eq!(ledger.free_balance("alice"), 5_000);
    }

    /// H7: duplicate escrow id rejected; balance unchanged.
    #[test]
    fn invariant_h7_unique_escrow_id() {
        let mut ledger = CreditLedger::new();
        ledger.mint("alice", 1_000);
        let p = escrow_params(14, "alice", "bob", 100, b"dup");
        ledger.open_escrow(p.clone()).unwrap();
        let err = ledger.open_escrow(p).unwrap_err();
        assert!(matches!(err, CreditError::DuplicateEscrow(_)));
        assert_eq!(ledger.free_balance("alice"), 900);
        assert_eq!(ledger.total_escrowed(), 100);
    }
}
