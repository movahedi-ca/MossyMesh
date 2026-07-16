//! Interop Module for MossyMesh
//!
//! Phase 5: AsyncAPI / OpenAPI gateway, TWAMM orchestration (2% max-spread),
//! and retroactive AMM liquidity mining for genesis offline nodes.

pub mod liquidity;
pub mod openapi_gateway;
pub mod twamm;

use std::sync::{Mutex, OnceLock};

use liquidity::LiquidityMiner;
use openapi_gateway::OpenApiGateway;
use twamm::{parse_order_payload, OrderSide, TwammEngine, MAX_SPREAD_BPS};

/// Process-wide gateway (activated on internet reconnect).
fn gateway() -> &'static Mutex<OpenApiGateway> {
    static GW: OnceLock<Mutex<OpenApiGateway>> = OnceLock::new();
    GW.get_or_init(|| Mutex::new(OpenApiGateway::new()))
}

/// Process-wide liquidity miner for genesis offline nodes.
fn miner() -> &'static Mutex<LiquidityMiner> {
    static MINER: OnceLock<Mutex<LiquidityMiner>> = OnceLock::new();
    MINER.get_or_init(|| Mutex::new(LiquidityMiner::new()))
}

/// Standalone TWAMM book for pure order streaming (also used by the gateway).
fn twamm_book() -> &'static Mutex<TwammEngine> {
    static BOOK: OnceLock<Mutex<TwammEngine>> = OnceLock::new();
    BOOK.get_or_init(|| Mutex::new(TwammEngine::new()))
}

pub mod credits;
pub mod htlc;

pub use credits::{Account, CreditError, CreditLedger};
pub use htlc::{
    hash_preimage, verify_preimage, Htlc, HtlcError, HtlcParams, HtlcState, MockVdf,
};

pub fn init_interop() {
    println!(
        "Interop: OpenAPI gateway (dormant), TWAMM max-spread {} bps, liquidity mining ready...",
        MAX_SPREAD_BPS
    );
}

/// Signal that upstream internet has returned; activates OpenAPI bridging + airdrop claims.
pub fn signal_internet_reconnect() {
    if let Ok(mut gw) = gateway().lock() {
        gw.on_internet_reconnect();
    }
    if let Ok(mut m) = miner().lock() {
        m.on_internet_reconnect();
    }
    println!("Interop: internet reconnect — OpenAPI gateway ACTIVE");
}

/// Signal that the island is offline again.
pub fn signal_internet_disconnect() {
    if let Ok(mut gw) = gateway().lock() {
        gw.on_internet_disconnect();
    }
    if let Ok(mut m) = miner().lock() {
        m.on_internet_disconnect();
    }
    println!("Interop: internet disconnect — OpenAPI gateway DORMANT");
}

pub struct AsyncApiRequest {
    pub endpoint: String,
    pub payload: String,
}

/// Simulates routing an incoming HTTP REST request to the offline Mesh network.
///
/// Supported endpoints:
/// - `GET/POST` `/api/v1/health`
/// - `POST` `/api/v1/submit_job`
/// - `/api/v1/twamm` — TWAMM status / stream order submit
/// - `/api/v1/liquidity` — genesis mining status / accrue / claim
/// - `/api/v1/gateway` — OpenAPI gateway status / reconnect / bridge (helper)
pub fn handle_rest_call(req: &AsyncApiRequest) -> Result<String, InteropError> {
    // Normalize path: strip query string, trailing slash noise.
    let path = req.endpoint.split('?').next().unwrap_or(&req.endpoint);
    let path = path.trim_end_matches('/');
    let path = if path.is_empty() { "/" } else { path };

    match path {
        "/api/v1/health" => Ok("Mesh Island Active".to_string()),
        "/api/v1/submit_job" => {
            println!("Routing job payload [{}] into Kademlia DHT...", req.payload);
            Ok("Job Accepted".to_string())
        }
        "/api/v1/twamm" => handle_twamm(req),
        "/api/v1/liquidity" => handle_liquidity(req),
        "/api/v1/gateway" => handle_gateway(req),
        _ => Err(InteropError::ConnectionRefused),
    }
}

fn handle_twamm(req: &AsyncApiRequest) -> Result<String, InteropError> {
    let payload = req.payload.trim();
    if payload.is_empty() || payload.eq_ignore_ascii_case("status") {
        let book = twamm_book()
            .lock()
            .map_err(|_| InteropError::Timeout)?;
        return Ok(book.status_json());
    }

    // action=stream|submit + order fields
    if let Some((side, amount, slices, ref_price, exec_price)) = parse_order_payload(payload) {
        let mut book = twamm_book()
            .lock()
            .map_err(|_| InteropError::Timeout)?;
        let id = book
            .submit_order(side, amount, slices, ref_price)
            .map_err(|e| {
                println!("TWAMM submit error: {e}");
                InteropError::BadRequest
            })?;

        if let Some(exec) = exec_price {
            match book.stream_slice(&id, exec) {
                Ok(fill) => {
                    return Ok(format!(
                        "{{\"order_id\":\"{}\",\"amount_in\":{},\"amount_out\":{},\"execution_price\":{},\"spread_bps\":{},\"max_spread_bps\":{},\"side\":\"{}\"}}",
                        fill.order_id,
                        fill.amount_in,
                        fill.amount_out,
                        fill.execution_price,
                        fill.spread_bps,
                        MAX_SPREAD_BPS,
                        match side {
                            OrderSide::Buy => "buy",
                            OrderSide::Sell => "sell",
                        }
                    ));
                }
                Err(e) => {
                    println!("TWAMM stream error: {e}");
                    return Err(InteropError::SpreadCapExceeded);
                }
            }
        }

        return Ok(format!(
            "{{\"order_id\":\"{}\",\"status\":\"accepted\",\"slices\":{},\"max_spread_bps\":{}}}",
            id, slices, MAX_SPREAD_BPS
        ));
    }

    Err(InteropError::BadRequest)
}

fn handle_liquidity(req: &AsyncApiRequest) -> Result<String, InteropError> {
    let payload = req.payload.trim();
    if payload.is_empty() || payload.eq_ignore_ascii_case("status") {
        let m = miner().lock().map_err(|_| InteropError::Timeout)?;
        return Ok(m.status_json());
    }

    // Parse action + node_id + optional epochs
    let mut action = String::new();
    let mut node_id = String::new();
    let mut epochs: u64 = 1;

    for part in payload.split(|c| c == ',' || c == '&' || c == ';') {
        let part = part.trim().trim_matches(|c| c == '{' || c == '}' || c == '"');
        if part.is_empty() {
            continue;
        }
        let mut kv = part.splitn(2, |c| c == '=' || c == ':');
        let key = kv
            .next()
            .unwrap_or("")
            .trim()
            .trim_matches('"')
            .to_ascii_lowercase();
        let val = kv.next().unwrap_or("").trim().trim_matches('"');
        match key.as_str() {
            "action" => action = val.to_ascii_lowercase(),
            "node_id" | "node" | "account" => node_id = val.to_string(),
            "epochs" => epochs = val.parse().unwrap_or(1),
            _ => {}
        }
    }

    if node_id.is_empty() && action != "status" {
        // Allow bare register-style: "register:pi-zero-1"
        if let Some((a, n)) = payload.split_once(':') {
            action = a.trim().to_ascii_lowercase();
            node_id = n.trim().to_string();
        }
    }

    let mut m = miner().lock().map_err(|_| InteropError::Timeout)?;

    match action.as_str() {
        "" | "status" => Ok(m.status_json()),
        "register" | "register_genesis" => {
            if node_id.is_empty() {
                return Err(InteropError::BadRequest);
            }
            m.register_genesis(&node_id);
            m.account_json(&node_id).map_err(|_| InteropError::BadRequest)
        }
        "accrue" => {
            if node_id.is_empty() {
                return Err(InteropError::BadRequest);
            }
            m.ensure_genesis(&node_id);
            let gained = m
                .accrue_offline_epochs(&node_id, epochs)
                .map_err(|_| InteropError::BadRequest)?;
            Ok(format!(
                "{{\"node_id\":\"{}\",\"epochs\":{},\"points_gained\":{},\"total_points\":{}}}",
                node_id,
                epochs,
                gained,
                m.get(&node_id).map(|a| a.points).unwrap_or(0)
            ))
        }
        "claim" => {
            if node_id.is_empty() {
                return Err(InteropError::BadRequest);
            }
            match m.claim_airdrop(&node_id) {
                Ok(tokens) => Ok(format!(
                    "{{\"node_id\":\"{}\",\"tokens_airdropped\":{},\"status\":\"claimed\"}}",
                    node_id, tokens
                )),
                Err(liquidity::LiquidityError::StillOffline) => Err(InteropError::GatewayDormant),
                Err(_) => Err(InteropError::BadRequest),
            }
        }
        "get" => {
            if node_id.is_empty() {
                return Err(InteropError::BadRequest);
            }
            m.account_json(&node_id).map_err(|_| InteropError::BadRequest)
        }
        _ => Err(InteropError::BadRequest),
    }
}

fn handle_gateway(req: &AsyncApiRequest) -> Result<String, InteropError> {
    let payload = req.payload.trim().to_ascii_lowercase();
    if payload.is_empty() || payload == "status" {
        let gw = gateway().lock().map_err(|_| InteropError::Timeout)?;
        return Ok(gw.status_json());
    }

    if payload == "reconnect" || payload.contains("action=reconnect") {
        signal_internet_reconnect();
        let gw = gateway().lock().map_err(|_| InteropError::Timeout)?;
        return Ok(gw.status_json());
    }

    if payload == "disconnect" || payload.contains("action=disconnect") {
        signal_internet_disconnect();
        let gw = gateway().lock().map_err(|_| InteropError::Timeout)?;
        return Ok(gw.status_json());
    }

    // bridge: account=...,amount=...,slices=...
    let mut account = String::new();
    let mut amount: u64 = 0;
    let mut slices: u32 = 1;
    let mut action = String::new();

    for part in req.payload.split(|c| c == ',' || c == '&' || c == ';') {
        let part = part.trim().trim_matches(|c| c == '{' || c == '}' || c == '"');
        let mut kv = part.splitn(2, |c| c == '=' || c == ':');
        let key = kv
            .next()
            .unwrap_or("")
            .trim()
            .trim_matches('"')
            .to_ascii_lowercase();
        let val = kv.next().unwrap_or("").trim().trim_matches('"');
        match key.as_str() {
            "action" => action = val.to_ascii_lowercase(),
            "account" | "node" | "node_id" => account = val.to_string(),
            "amount" => amount = val.parse().unwrap_or(0),
            "slices" => slices = val.parse().unwrap_or(1),
            "credit" | "balance" => {
                // balance form: credit=node-a:10000
                if let Some((acc, bal)) = val.split_once(':') {
                    // handled after loop via early return below — stash in account/amount misuse avoided
                    let _ = (acc, bal);
                }
            }
            _ => {}
        }
        if key == "credit" || key == "balance" {
            if let Some((acc, bal)) = val.split_once(':') {
                let mut gw = gateway().lock().map_err(|_| InteropError::Timeout)?;
                let bal: u64 = bal.parse().unwrap_or(0);
                gw.set_local_credit(acc.trim(), bal);
                return Ok(format!(
                    "{{\"account\":\"{}\",\"local_credit\":{}}}",
                    acc.trim(),
                    bal
                ));
            }
        }
    }

    if action == "bridge" || (!account.is_empty() && amount > 0) {
        let mut gw = gateway().lock().map_err(|_| InteropError::Timeout)?;
        match gw.bridge_local_to_global(&account, amount, slices) {
            Ok(receipt) => Ok(receipt.to_json()),
            Err(openapi_gateway::GatewayError::GatewayDormant) => Err(InteropError::GatewayDormant),
            Err(openapi_gateway::GatewayError::Twamm(twamm::TwammError::SpreadExceeded { .. })) => {
                Err(InteropError::SpreadCapExceeded)
            }
            Err(e) => {
                println!("Gateway bridge error: {e}");
                Err(InteropError::BadRequest)
            }
        }
    } else {
        Err(InteropError::BadRequest)
    }
}

/// Simulates an ongoing WebSocket event loop syncing state to the external internet.
pub fn handle_websocket(mut connection_alive: bool) {
    let mut tick = 0;
    while connection_alive && tick < 3 {
        println!("WebSocket Sync Tick {}...", tick);
        tick += 1;
        // Simulate break
        if tick == 2 {
            connection_alive = false;
        }
    }
    println!("WebSocket Connection Closed.");
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InteropError {
    Timeout,
    ConnectionRefused,
    BadRequest,
    /// TWAMM rejected fill: spread above 2% cap.
    SpreadCapExceeded,
    /// OpenAPI gateway is offline / internet not reconnected.
    GatewayDormant,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_and_submit_job_unchanged() {
        let health = handle_rest_call(&AsyncApiRequest {
            endpoint: "/api/v1/health".into(),
            payload: String::new(),
        })
        .unwrap();
        assert_eq!(health, "Mesh Island Active");

        let job = handle_rest_call(&AsyncApiRequest {
            endpoint: "/api/v1/submit_job".into(),
            payload: r#"{"action":"move"}"#.into(),
        })
        .unwrap();
        assert_eq!(job, "Job Accepted");
    }

    #[test]
    fn twamm_endpoint_enforces_spread_cap() {
        // Within 2%
        let ok = handle_rest_call(&AsyncApiRequest {
            endpoint: "/api/v1/twamm".into(),
            payload: "side=sell,amount=1000000,slices=1,ref_price=1000000,exec_price=1010000"
                .into(),
        });
        assert!(ok.is_ok(), "1% spread should pass: {ok:?}");

        // Over 2%
        let err = handle_rest_call(&AsyncApiRequest {
            endpoint: "/api/v1/twamm".into(),
            payload: "side=sell,amount=1000000,slices=1,ref_price=1000000,exec_price=1030000"
                .into(),
        });
        assert_eq!(err, Err(InteropError::SpreadCapExceeded));
    }

    #[test]
    fn liquidity_endpoint_register_and_accrue() {
        let reg = handle_rest_call(&AsyncApiRequest {
            endpoint: "/api/v1/liquidity".into(),
            payload: "action=register,node_id=genesis-test-1".into(),
        })
        .unwrap();
        assert!(reg.contains("genesis-test-1"));

        // Ensure miner is offline for accrual
        signal_internet_disconnect();
        let acc = handle_rest_call(&AsyncApiRequest {
            endpoint: "/api/v1/liquidity".into(),
            payload: "action=accrue,node_id=genesis-test-1,epochs=2".into(),
        })
        .unwrap();
        assert!(acc.contains("points_gained"));
    }

    #[test]
    fn unknown_route_still_refused() {
        let err = handle_rest_call(&AsyncApiRequest {
            endpoint: "/api/v1/nope".into(),
            payload: String::new(),
        });
        assert_eq!(err, Err(InteropError::ConnectionRefused));
    }
}
