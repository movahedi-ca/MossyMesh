//! OpenAPI gateway activated when the mesh regains upstream internet.
//!
//! Bridges local offline credit balances into a mock global AMM pool once the
//! `internet_reconnected` flag flips true. While offline, the gateway stays dormant.

use std::collections::HashMap;

use crate::twamm::{OrderSide, TwammEngine, TwammError, MAX_SPREAD_BPS};

/// Mock representation of a global AMM pool (e.g. Uniswap-style constant product).
#[derive(Debug, Clone)]
pub struct GlobalAmmMock {
    /// Base-asset reserve (mesh credits / governance token units).
    pub reserve_base: u64,
    /// Quote-asset reserve (stable units, scale 1e6).
    pub reserve_quote: u64,
}

impl GlobalAmmMock {
    pub fn new(reserve_base: u64, reserve_quote: u64) -> Self {
        Self {
            reserve_base,
            reserve_quote,
        }
    }

    /// Spot mid price: quote per base, scale 1e6.
    pub fn mid_price(&self) -> u64 {
        if self.reserve_base == 0 {
            return 0;
        }
        ((self.reserve_quote as u128) * 1_000_000 / (self.reserve_base as u128)) as u64
    }

    /// Apply a simple constant-product swap, returning amount out.
    pub fn swap(&mut self, side: OrderSide, amount_in: u64) -> Result<u64, GatewayError> {
        if amount_in == 0 {
            return Err(GatewayError::ZeroAmount);
        }
        if self.reserve_base == 0 || self.reserve_quote == 0 {
            return Err(GatewayError::EmptyPool);
        }
        match side {
            OrderSide::Buy => {
                // Spend quote, receive base: dy = y * dx / (x + dx)
                let dx = amount_in;
                let dy = ((self.reserve_base as u128) * (dx as u128))
                    / ((self.reserve_quote as u128) + (dx as u128));
                let dy = dy as u64;
                self.reserve_quote = self.reserve_quote.saturating_add(dx);
                self.reserve_base = self.reserve_base.saturating_sub(dy);
                Ok(dy)
            }
            OrderSide::Sell => {
                // Spend base, receive quote
                let dx = amount_in;
                let dy = ((self.reserve_quote as u128) * (dx as u128))
                    / ((self.reserve_base as u128) + (dx as u128));
                let dy = dy as u64;
                self.reserve_base = self.reserve_base.saturating_add(dx);
                self.reserve_quote = self.reserve_quote.saturating_sub(dy);
                Ok(dy)
            }
        }
    }
}

/// Errors from the OpenAPI gateway bridge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GatewayError {
    /// Internet has not reconnected; gateway is dormant.
    GatewayDormant,
    /// Account has insufficient local credit balance.
    InsufficientBalance,
    /// Unknown local account.
    UnknownAccount,
    ZeroAmount,
    EmptyPool,
    Twamm(TwammError),
}

impl std::fmt::Display for GatewayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GatewayError::GatewayDormant => {
                write!(f, "openapi gateway dormant (internet not reconnected)")
            }
            GatewayError::InsufficientBalance => write!(f, "insufficient local credit balance"),
            GatewayError::UnknownAccount => write!(f, "unknown account"),
            GatewayError::ZeroAmount => write!(f, "zero amount"),
            GatewayError::EmptyPool => write!(f, "empty global AMM pool"),
            GatewayError::Twamm(e) => write!(f, "twamm: {e}"),
        }
    }
}

impl std::error::Error for GatewayError {}

impl From<TwammError> for GatewayError {
    fn from(value: TwammError) -> Self {
        GatewayError::Twamm(value)
    }
}

/// OpenAPI gateway: dormant until internet reconnects, then bridges local credits → global AMM.
#[derive(Debug)]
pub struct OpenApiGateway {
    /// When true, REST OpenAPI surface is live and bridging is allowed.
    pub internet_reconnected: bool,
    /// Local mesh credit balances keyed by node / account id.
    pub local_credits: HashMap<String, u64>,
    /// Mock global AMM pool reached only after reconnect.
    pub global_amm: GlobalAmmMock,
    /// TWAMM engine used to stream large bridges under the 2% spread cap.
    pub twamm: TwammEngine,
    /// Last bridge summary for status endpoints.
    pub last_bridge_note: Option<String>,
}

impl Default for OpenApiGateway {
    fn default() -> Self {
        Self::new()
    }
}

impl OpenApiGateway {
    pub fn new() -> Self {
        Self {
            internet_reconnected: false,
            local_credits: HashMap::new(),
            global_amm: GlobalAmmMock::new(1_000_000, 1_000_000_000_000), // mid ≈ 1.0e6 scale
            twamm: TwammEngine::new(),
            last_bridge_note: None,
        }
    }

    /// Flip the internet-reconnect flag and activate the gateway.
    pub fn on_internet_reconnect(&mut self) {
        self.internet_reconnected = true;
        self.last_bridge_note = Some("openapi gateway activated on internet reconnect".into());
    }

    /// Explicitly mark the island offline again (gateway goes dormant).
    pub fn on_internet_disconnect(&mut self) {
        self.internet_reconnected = false;
        self.last_bridge_note = Some("openapi gateway dormant (offline)".into());
    }

    /// Whether the OpenAPI surface should accept external traffic.
    pub fn is_active(&self) -> bool {
        self.internet_reconnected
    }

    pub fn set_local_credit(&mut self, account: impl Into<String>, balance: u64) {
        self.local_credits.insert(account.into(), balance);
    }

    pub fn local_balance(&self, account: &str) -> u64 {
        self.local_credits.get(account).copied().unwrap_or(0)
    }

    /// Bridge `amount` of local credits from `account` into the global AMM via TWAMM,
    /// enforcing the 2% max-spread cap against the live global mid.
    ///
    /// Only available when `internet_reconnected` is true.
    pub fn bridge_local_to_global(
        &mut self,
        account: &str,
        amount: u64,
        slices: u32,
    ) -> Result<BridgeReceipt, GatewayError> {
        if !self.internet_reconnected {
            return Err(GatewayError::GatewayDormant);
        }
        if amount == 0 {
            return Err(GatewayError::ZeroAmount);
        }
        let bal = self
            .local_credits
            .get(account)
            .copied()
            .ok_or(GatewayError::UnknownAccount)?;
        if bal < amount {
            return Err(GatewayError::InsufficientBalance);
        }

        let reference_price = self.global_amm.mid_price();
        if reference_price == 0 {
            return Err(GatewayError::EmptyPool);
        }

        let order_id = self
            .twamm
            .submit_order(OrderSide::Sell, amount, slices.max(1), reference_price)?;

        let mut filled_in = 0u64;
        let mut filled_out = 0u64;
        let mut max_spread_seen = 0u32;
        let slices = slices.max(1);

        for _ in 0..slices {
            // Re-quote mid each slice; reject if it drifted beyond 2%.
            let exec = self.global_amm.mid_price();
            match self.twamm.stream_slice(&order_id, exec) {
                Ok(fill) => {
                    // Actually move inventory in the mock AMM.
                    let out = self.global_amm.swap(OrderSide::Sell, fill.amount_in)?;
                    filled_in = filled_in.saturating_add(fill.amount_in);
                    filled_out = filled_out.saturating_add(out);
                    max_spread_seen = max_spread_seen.max(fill.spread_bps);
                }
                Err(TwammError::SpreadExceeded { spread_bps, max_bps }) => {
                    // Refund unfilled remainder back to local balance accounting below.
                    let _ = (spread_bps, max_bps);
                    break;
                }
                Err(e) => return Err(GatewayError::Twamm(e)),
            }
        }

        // Debit only what was actually filled.
        if filled_in > 0 {
            if let Some(b) = self.local_credits.get_mut(account) {
                *b = b.saturating_sub(filled_in);
            }
        }

        let receipt = BridgeReceipt {
            account: account.to_string(),
            order_id,
            amount_bridged: filled_in,
            quote_received: filled_out,
            max_spread_bps_seen: max_spread_seen,
            max_spread_cap_bps: MAX_SPREAD_BPS,
            gateway_active: true,
        };
        self.last_bridge_note = Some(format!(
            "bridged {} credits for {} (cap {} bps)",
            filled_in, account, MAX_SPREAD_BPS
        ));
        Ok(receipt)
    }

    /// Human / REST status blob.
    pub fn status_json(&self) -> String {
        let note = self
            .last_bridge_note
            .as_deref()
            .unwrap_or("idle");
        format!(
            "{{\"active\":{},\"internet_reconnected\":{},\"max_spread_bps\":{},\"global_mid\":{},\"accounts\":{},\"note\":\"{}\"}}",
            self.is_active(),
            self.internet_reconnected,
            MAX_SPREAD_BPS,
            self.global_amm.mid_price(),
            self.local_credits.len(),
            note.replace('"', "'")
        )
    }
}

/// Receipt returned after a successful (or partial) local→global bridge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeReceipt {
    pub account: String,
    pub order_id: String,
    pub amount_bridged: u64,
    pub quote_received: u64,
    pub max_spread_bps_seen: u32,
    pub max_spread_cap_bps: u32,
    pub gateway_active: bool,
}

impl BridgeReceipt {
    pub fn to_json(&self) -> String {
        format!(
            "{{\"account\":\"{}\",\"order_id\":\"{}\",\"amount_bridged\":{},\"quote_received\":{},\"max_spread_bps_seen\":{},\"max_spread_cap_bps\":{},\"gateway_active\":{}}}",
            self.account,
            self.order_id,
            self.amount_bridged,
            self.quote_received,
            self.max_spread_bps_seen,
            self.max_spread_cap_bps,
            self.gateway_active
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gateway_dormant_until_reconnect() {
        let mut gw = OpenApiGateway::new();
        gw.set_local_credit("node-a", 10_000);
        assert!(!gw.is_active());
        let err = gw.bridge_local_to_global("node-a", 100, 1).unwrap_err();
        assert_eq!(err, GatewayError::GatewayDormant);

        gw.on_internet_reconnect();
        assert!(gw.is_active());
        let receipt = gw.bridge_local_to_global("node-a", 100, 1).unwrap();
        assert_eq!(receipt.amount_bridged, 100);
        assert!(receipt.max_spread_bps_seen <= MAX_SPREAD_BPS);
        assert_eq!(gw.local_balance("node-a"), 9_900);
    }

    #[test]
    fn disconnect_makes_gateway_dormant_again() {
        let mut gw = OpenApiGateway::new();
        gw.on_internet_reconnect();
        assert!(gw.is_active());
        gw.on_internet_disconnect();
        assert!(!gw.is_active());
    }
}
