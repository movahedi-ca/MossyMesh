//! Quadratic staking collateral locks for MossyMesh governance.
//!
//! Staking `p` units of power locks `p²` collateral tokens. This makes
//! concentrated influence (and voucher underwriting) progressively expensive.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::wot::NodeId;

/// Quadratic cost: collateral required to lock `power_units` of influence.
#[inline]
pub fn quadratic_cost(power_units: u64) -> u128 {
    let p = power_units as u128;
    p.saturating_mul(p)
}

/// Integer square-root of collateral → recovered power units (floor).
#[inline]
pub fn power_from_collateral(collateral: u128) -> u64 {
    // isqrt for u128
    if collateral == 0 {
        return 0;
    }
    let mut x = collateral;
    let mut y = (x + 1) / 2;
    while y < x {
        x = y;
        y = (x + collateral / x) / 2;
    }
    x as u64
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StakingError {
    ZeroPower,
    InsufficientLock,
    NothingToSlash,
}

/// A single collateral lock entry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CollateralLock {
    pub staker: NodeId,
    pub power_units: u64,
    pub collateral: u128,
}

/// Ledger of quadratic collateral locks and slash accounting.
#[derive(Clone, Debug, Default)]
pub struct QuadraticStaking {
    /// Aggregate locked collateral per node.
    locked: HashMap<NodeId, u128>,
    /// Aggregate power units currently locked per node.
    power: HashMap<NodeId, u64>,
    /// Lifetime slashed collateral per node.
    slashed: HashMap<NodeId, u128>,
}

impl QuadraticStaking {
    pub fn new() -> Self {
        Self::default()
    }

    /// Lock quadratic collateral for `power_units`. Returns the lock receipt.
    pub fn lock(&mut self, staker: NodeId, power_units: u64) -> Result<CollateralLock, StakingError> {
        if power_units == 0 {
            return Err(StakingError::ZeroPower);
        }
        let collateral = quadratic_cost(power_units);
        *self.locked.entry(staker).or_insert(0) += collateral;
        *self.power.entry(staker).or_insert(0) += power_units;
        Ok(CollateralLock {
            staker,
            power_units,
            collateral,
        })
    }

    /// Slash `power_units` worth of lock from `staker` (removes p² collateral).
    pub fn slash(&mut self, staker: &NodeId, power_units: u64) -> Result<u128, StakingError> {
        if power_units == 0 {
            return Err(StakingError::ZeroPower);
        }
        let cost = quadratic_cost(power_units);
        let locked = self.locked.get_mut(staker).ok_or(StakingError::NothingToSlash)?;
        if *locked < cost {
            return Err(StakingError::InsufficientLock);
        }
        *locked -= cost;
        if *locked == 0 {
            self.locked.remove(staker);
        }

        if let Some(p) = self.power.get_mut(staker) {
            *p = p.saturating_sub(power_units);
            if *p == 0 {
                self.power.remove(staker);
            }
        }

        *self.slashed.entry(*staker).or_insert(0) += cost;
        Ok(cost)
    }

    /// Release (unlock) without slashing — e.g. invitee graduated cleanly.
    pub fn unlock(&mut self, staker: &NodeId, power_units: u64) -> Result<u128, StakingError> {
        if power_units == 0 {
            return Err(StakingError::ZeroPower);
        }
        let cost = quadratic_cost(power_units);
        let locked = self.locked.get_mut(staker).ok_or(StakingError::NothingToSlash)?;
        if *locked < cost {
            return Err(StakingError::InsufficientLock);
        }
        *locked -= cost;
        if *locked == 0 {
            self.locked.remove(staker);
        }
        if let Some(p) = self.power.get_mut(staker) {
            *p = p.saturating_sub(power_units);
            if *p == 0 {
                self.power.remove(staker);
            }
        }
        Ok(cost)
    }

    pub fn locked_collateral(&self, staker: &NodeId) -> u128 {
        self.locked.get(staker).copied().unwrap_or(0)
    }

    pub fn locked_power(&self, staker: &NodeId) -> u64 {
        self.power.get(staker).copied().unwrap_or(0)
    }

    pub fn slashed_total(&self, staker: &NodeId) -> u128 {
        self.slashed.get(staker).copied().unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quadratic_cost_table() {
        assert_eq!(quadratic_cost(0), 0);
        assert_eq!(quadratic_cost(1), 1);
        assert_eq!(quadratic_cost(2), 4);
        assert_eq!(quadratic_cost(10), 100);
        assert_eq!(quadratic_cost(100), 10_000);
    }

    #[test]
    fn power_from_collateral_is_isqrt() {
        assert_eq!(power_from_collateral(0), 0);
        assert_eq!(power_from_collateral(1), 1);
        assert_eq!(power_from_collateral(4), 2);
        assert_eq!(power_from_collateral(99), 9);
        assert_eq!(power_from_collateral(100), 10);
    }

    #[test]
    fn lock_and_slash_accounting() {
        let mut s = QuadraticStaking::new();
        let n = NodeId::from_label("staker");
        s.lock(n, 5).unwrap();
        assert_eq!(s.locked_collateral(&n), 25);
        assert_eq!(s.locked_power(&n), 5);

        let slashed = s.slash(&n, 3).unwrap();
        assert_eq!(slashed, 9);
        assert_eq!(s.locked_collateral(&n), 16); // 25 - 9
        assert_eq!(s.locked_power(&n), 2);
        assert_eq!(s.slashed_total(&n), 9);
    }

    #[test]
    fn slash_more_than_locked_fails() {
        let mut s = QuadraticStaking::new();
        let n = NodeId::from_label("staker");
        s.lock(n, 2).unwrap();
        assert_eq!(s.slash(&n, 3), Err(StakingError::InsufficientLock));
    }
}
