//! Governance Module for MossyMesh
//!
//! Implements liquid DAO governance without a permanent central authority:
//! - Web of Trust (WoT) onboarding with voucher slash on malicious invitees
//! - Quadratic staking collateral locks
//! - 3-of-5 admin multi-sig whose authority weight decays to zero over 90 days
//! - ZK-blinded commit–reveal voting for verified edge nodes
//!
//! DOC: Network starts with admin multi-sig; control transitions to blind edge
//! voters as admin weight reaches zero after the decay window.

pub mod multisig;
pub mod staking;
pub mod voting;
pub mod wot;

pub use multisig::{AdminMultisig, MultisigError, MultisigProposal, Signature};
pub use staking::{CollateralLock, QuadraticStaking, StakingError};
pub use voting::{BallotChoice, BlindedBallot, VotingError, ZkBlindedVoting};
pub use wot::{NodeId, VoucherEdge, WotError, WotGraph};

/// Days until admin multi-sig authority is fully extinguished.
pub const ADMIN_DECAY_DAYS: u64 = 90;

/// Threshold of admin signatures required while multi-sig retains authority.
pub const MULTISIG_THRESHOLD: usize = 3;

/// Number of admin keys in the genesis multi-sig set.
pub const MULTISIG_SIGNERS: usize = 5;

/// Initialize the governance subsystem (logging / discovery hook).
pub fn init_governance() {
    println!(
        "Governance: WoT onboarding, quadratic staking, {}-of-{} multi-sig ({}-day decay), ZK-blinded voting.",
        MULTISIG_THRESHOLD, MULTISIG_SIGNERS, ADMIN_DECAY_DAYS
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constants_match_spec() {
        assert_eq!(ADMIN_DECAY_DAYS, 90);
        assert_eq!(MULTISIG_THRESHOLD, 3);
        assert_eq!(MULTISIG_SIGNERS, 5);
    }
}
