//! TWAMM (Time-Weighted Average Market Maker) orchestration.
//!
//! Streams large orders over time while enforcing a strict **2% max-spread cap**
//! against the reference mid price when bridging local mesh liquidity to a global AMM.

use serde::{Deserialize, Serialize};

/// Maximum allowed absolute spread versus reference mid price, in basis points.
/// 200 bps = 2.00%.
pub const MAX_SPREAD_BPS: u32 = 200;

/// One basis-point unit as a fraction of price (1/10_000).
const BPS_DENOM: u64 = 10_000;

/// Buy or sell side of a TWAMM stream order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OrderSide {
    Buy,
    Sell,
}

/// A TWAMM virtual order that is executed as a continuous stream of mini-fills.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TwammOrder {
    /// Stable order identifier (mesh-local or gateway-assigned).
    pub id: String,
    pub side: OrderSide,
    /// Total notional still remaining to stream (asset units, fixed-point scale 1e6).
    pub remaining_in: u64,
    /// Number of equal stream slices still outstanding.
    pub slices_remaining: u32,
    /// Reference mid price used for spread checks (quote per base, scale 1e6).
    pub reference_price: u64,
}

/// Result of a single stream slice fill (or a rejected slice).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamFill {
    pub order_id: String,
    pub amount_in: u64,
    pub amount_out: u64,
    pub execution_price: u64,
    pub spread_bps: u32,
}

/// Errors produced by the TWAMM engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TwammError {
    /// Spread between execution and reference mid exceeds the 2% cap.
    SpreadExceeded { spread_bps: u32, max_bps: u32 },
    /// Order has no remaining size or slices.
    OrderExhausted,
    /// Reference or execution price is zero / invalid.
    InvalidPrice,
    /// Order id not found in the engine book.
    OrderNotFound,
    /// Zero-sized stream request.
    ZeroAmount,
}

impl std::fmt::Display for TwammError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TwammError::SpreadExceeded { spread_bps, max_bps } => {
                write!(
                    f,
                    "spread {spread_bps} bps exceeds max cap of {max_bps} bps"
                )
            }
            TwammError::OrderExhausted => write!(f, "order exhausted"),
            TwammError::InvalidPrice => write!(f, "invalid price"),
            TwammError::OrderNotFound => write!(f, "order not found"),
            TwammError::ZeroAmount => write!(f, "zero amount"),
        }
    }
}

impl std::error::Error for TwammError {}

/// Compute absolute spread in basis points between `execution_price` and `reference_price`.
///
/// `spread_bps = |exec - ref| * 10_000 / ref`
pub fn spread_bps(execution_price: u64, reference_price: u64) -> Result<u32, TwammError> {
    if reference_price == 0 || execution_price == 0 {
        return Err(TwammError::InvalidPrice);
    }
    let diff = if execution_price >= reference_price {
        execution_price - reference_price
    } else {
        reference_price - execution_price
    };
    let bps = diff
        .saturating_mul(BPS_DENOM)
        .checked_div(reference_price)
        .ok_or(TwammError::InvalidPrice)?;
    Ok(bps.min(u32::MAX as u64) as u32)
}

/// Returns `Ok(spread_bps)` if within the 2% cap, otherwise `Err(SpreadExceeded)`.
pub fn enforce_max_spread(execution_price: u64, reference_price: u64) -> Result<u32, TwammError> {
    let bps = spread_bps(execution_price, reference_price)?;
    if bps > MAX_SPREAD_BPS {
        return Err(TwammError::SpreadExceeded {
            spread_bps: bps,
            max_bps: MAX_SPREAD_BPS,
        });
    }
    Ok(bps)
}

/// Apply a buy or sell against a quoted execution price and return `(amount_out, execution_price)`.
fn quote_fill(side: OrderSide, amount_in: u64, execution_price: u64) -> Result<(u64, u64), TwammError> {
    if amount_in == 0 {
        return Err(TwammError::ZeroAmount);
    }
    if execution_price == 0 {
        return Err(TwammError::InvalidPrice);
    }
    // Prices are scale 1e6; amount_out = amount_in * price / 1e6 for buys (quote→base inverted for sell).
    let scale: u128 = 1_000_000;
    let amount_out = match side {
        OrderSide::Buy => {
            // Spend `amount_in` quote to receive base at execution_price (quote per base).
            ((amount_in as u128) * scale) / (execution_price as u128)
        }
        OrderSide::Sell => {
            // Sell `amount_in` base to receive quote at execution_price.
            ((amount_in as u128) * (execution_price as u128)) / scale
        }
    } as u64;
    Ok((amount_out, execution_price))
}

/// In-memory TWAMM book that streams orders slice-by-slice under the spread cap.
#[derive(Debug, Default)]
pub struct TwammEngine {
    orders: Vec<TwammOrder>,
    next_id: u64,
}

impl TwammEngine {
    pub fn new() -> Self {
        Self {
            orders: Vec::new(),
            next_id: 1,
        }
    }

    /// Submit a new TWAMM order split into `slices` equal stream parts.
    pub fn submit_order(
        &mut self,
        side: OrderSide,
        amount_in: u64,
        slices: u32,
        reference_price: u64,
    ) -> Result<String, TwammError> {
        if amount_in == 0 {
            return Err(TwammError::ZeroAmount);
        }
        if reference_price == 0 {
            return Err(TwammError::InvalidPrice);
        }
        let slices = slices.max(1);
        let id = format!("twamm-{}", self.next_id);
        self.next_id += 1;
        self.orders.push(TwammOrder {
            id: id.clone(),
            side,
            remaining_in: amount_in,
            slices_remaining: slices,
            reference_price,
        });
        Ok(id)
    }

    /// Stream one slice of `order_id` at the given execution price.
    /// Enforces the global 2% max-spread cap before filling.
    pub fn stream_slice(
        &mut self,
        order_id: &str,
        execution_price: u64,
    ) -> Result<StreamFill, TwammError> {
        let idx = self
            .orders
            .iter()
            .position(|o| o.id == order_id)
            .ok_or(TwammError::OrderNotFound)?;

        if self.orders[idx].slices_remaining == 0 || self.orders[idx].remaining_in == 0 {
            return Err(TwammError::OrderExhausted);
        }

        let bps = enforce_max_spread(execution_price, self.orders[idx].reference_price)?;

        let slices = self.orders[idx].slices_remaining as u64;
        let amount_in = if slices == 1 {
            self.orders[idx].remaining_in
        } else {
            self.orders[idx].remaining_in / slices
        };
        if amount_in == 0 {
            return Err(TwammError::ZeroAmount);
        }

        let side = self.orders[idx].side;
        let (amount_out, exec) = quote_fill(side, amount_in, execution_price)?;

        self.orders[idx].remaining_in = self.orders[idx].remaining_in.saturating_sub(amount_in);
        self.orders[idx].slices_remaining -= 1;

        Ok(StreamFill {
            order_id: order_id.to_string(),
            amount_in,
            amount_out,
            execution_price: exec,
            spread_bps: bps,
        })
    }

    /// Peek at an order by id.
    pub fn get_order(&self, order_id: &str) -> Option<&TwammOrder> {
        self.orders.iter().find(|o| o.id == order_id)
    }

    /// Number of open orders with remaining size.
    pub fn open_order_count(&self) -> usize {
        self.orders
            .iter()
            .filter(|o| o.remaining_in > 0 && o.slices_remaining > 0)
            .count()
    }

    /// JSON summary of open book state (for REST).
    pub fn status_json(&self) -> String {
        let open: Vec<&TwammOrder> = self
            .orders
            .iter()
            .filter(|o| o.remaining_in > 0 && o.slices_remaining > 0)
            .collect();
        format!(
            "{{\"max_spread_bps\":{},\"open_orders\":{},\"orders\":{}}}",
            MAX_SPREAD_BPS,
            open.len(),
            serde_json_orders(&open)
        )
    }
}

fn serde_json_orders(orders: &[&TwammOrder]) -> String {
    // Lightweight manual JSON to avoid requiring serde_json as a hard runtime dep path.
    let parts: Vec<String> = orders
        .iter()
        .map(|o| {
            let side = match o.side {
                OrderSide::Buy => "buy",
                OrderSide::Sell => "sell",
            };
            format!(
                "{{\"id\":\"{}\",\"side\":\"{}\",\"remaining_in\":{},\"slices_remaining\":{},\"reference_price\":{}}}",
                o.id, side, o.remaining_in, o.slices_remaining, o.reference_price
            )
        })
        .collect();
    format!("[{}]", parts.join(","))
}

/// Parse a minimal JSON-ish payload for REST submit:
/// `side=buy|sell,amount=<u64>,slices=<u32>,ref_price=<u64>[,exec_price=<u64>]`
/// or key=value pairs separated by commas / ampersands.
pub fn parse_order_payload(payload: &str) -> Option<(OrderSide, u64, u32, u64, Option<u64>)> {
    let mut side = None;
    let mut amount = None;
    let mut slices = 1u32;
    let mut ref_price = None;
    let mut exec_price = None;

    for part in payload.split(|c| c == ',' || c == '&' || c == ';') {
        let part = part.trim().trim_matches(|c| c == '{' || c == '}' || c == '"');
        if part.is_empty() {
            continue;
        }
        let mut kv = part.splitn(2, |c| c == '=' || c == ':');
        let key = kv.next()?.trim().trim_matches('"').to_ascii_lowercase();
        let val = kv.next()?.trim().trim_matches('"');
        match key.as_str() {
            "side" => {
                side = match val.to_ascii_lowercase().as_str() {
                    "buy" => Some(OrderSide::Buy),
                    "sell" => Some(OrderSide::Sell),
                    _ => None,
                };
            }
            "amount" | "amount_in" => amount = val.parse().ok(),
            "slices" => slices = val.parse().unwrap_or(1),
            "ref_price" | "reference_price" => ref_price = val.parse().ok(),
            "exec_price" | "execution_price" => exec_price = val.parse().ok(),
            _ => {}
        }
    }

    Some((side?, amount?, slices.max(1), ref_price?, exec_price))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spread_exactly_two_percent_is_allowed() {
        // ref = 1_000_000, exec = 1_020_000 → 2.00% = 200 bps
        let bps = enforce_max_spread(1_020_000, 1_000_000).expect("2% should pass");
        assert_eq!(bps, MAX_SPREAD_BPS);
    }

    #[test]
    fn spread_just_over_two_percent_is_rejected() {
        // 201 bps
        let err = enforce_max_spread(1_020_100, 1_000_000).unwrap_err();
        match err {
            TwammError::SpreadExceeded { spread_bps, max_bps } => {
                assert!(spread_bps > MAX_SPREAD_BPS);
                assert_eq!(max_bps, MAX_SPREAD_BPS);
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn spread_below_cap_passes() {
        // 1% = 100 bps
        let bps = enforce_max_spread(990_000, 1_000_000).unwrap();
        assert_eq!(bps, 100);
    }

    #[test]
    fn zero_reference_price_is_invalid() {
        assert_eq!(
            enforce_max_spread(1_000_000, 0),
            Err(TwammError::InvalidPrice)
        );
    }

    #[test]
    fn stream_slice_respects_spread_cap() {
        let mut eng = TwammEngine::new();
        let id = eng
            .submit_order(OrderSide::Sell, 1_000_000, 4, 1_000_000)
            .unwrap();

        // Within cap: 1.5%
        let fill = eng.stream_slice(&id, 1_015_000).unwrap();
        assert_eq!(fill.spread_bps, 150);
        assert_eq!(fill.amount_in, 250_000);

        // Over cap: 3%
        let err = eng.stream_slice(&id, 1_030_000).unwrap_err();
        assert!(matches!(err, TwammError::SpreadExceeded { .. }));

        // Order remaining unchanged after rejection
        let order = eng.get_order(&id).unwrap();
        assert_eq!(order.remaining_in, 750_000);
        assert_eq!(order.slices_remaining, 3);
    }

    #[test]
    fn stream_rejects_when_exhausted() {
        let mut eng = TwammEngine::new();
        let id = eng
            .submit_order(OrderSide::Buy, 100, 1, 1_000_000)
            .unwrap();
        eng.stream_slice(&id, 1_000_000).unwrap();
        assert_eq!(eng.stream_slice(&id, 1_000_000), Err(TwammError::OrderExhausted));
    }

    #[test]
    fn max_spread_constant_is_two_percent() {
        assert_eq!(MAX_SPREAD_BPS, 200);
    }
}
