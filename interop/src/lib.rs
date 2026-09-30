//! Interop Module for MossyMesh
//!
//! Phase 5: AsyncAPI / OpenAPI gateway, TWAMM orchestration (2% max-spread),
//! and retroactive AMM liquidity mining for genesis offline nodes.

pub mod liquidity;
pub mod openapi_gateway;
pub mod twamm;

use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use axum::{routing::{get, post}, Router, Json, extract::State};
use serde::{Deserialize, Serialize};

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

#[derive(Deserialize)]
pub struct GenericPayload {
    #[serde(default)]
    action: String,
    #[serde(default)]
    from: String,
    #[serde(default)]
    to: String,
    #[serde(default)]
    fen: String,
}

/// Starts an Axum HTTP server on port 8080 for the frontend.
pub async fn run_http_server() {
    let app = Router::new()
        .route("/api/v1/health", get(health_handler).post(health_handler))
        .route("/api/v1/submit_job", post(submit_job_handler));

    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080").await.unwrap();
    println!("Interop: HTTP Server listening on 0.0.0.0:8080");
    axum::serve(listener, app).await.unwrap();
}

async fn health_handler() -> &'static str {
    "Mesh Island Active"
}

async fn submit_job_handler(body: String) -> &'static str {
    println!("Routing job payload [{}] into Kademlia DHT/Sandbox...", body);
    "Job Accepted"
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

/// Current upstream connectivity, from the gateway's `internet_reconnected` flag.
pub fn internet_reconnected() -> bool {
    gateway()
        .lock()
        .map(|gw| gw.internet_reconnected)
        .unwrap_or(false)
}

/// Backoff policy for the WebSocket sync loop (issue #12).
#[derive(Debug, Clone, Copy)]
pub struct WsBackoff {
    /// Delay before the first reconnect attempt.
    pub base: Duration,
    /// Cap for the exponential growth.
    pub max: Duration,
}

impl WsBackoff {
    /// Delay before attempt `n` (0-based): `base * 2^n`, saturated at `max`.
    /// The shift is clamped so very large attempt counts cannot overflow.
    pub fn delay_for(&self, attempt: u32) -> Duration {
        let shift = attempt.min(20);
        self.base.saturating_mul(1u32 << shift).min(self.max)
    }
}

impl Default for WsBackoff {
    fn default() -> Self {
        Self {
            base: Duration::from_secs(1),
            max: Duration::from_secs(60),
        }
    }
}

/// One step of the persistent sync state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WsStep {
    /// Link is up: emit a sync tick.
    Tick,
    /// Link is down: sleep, then retry.
    Retry(Duration),
}

/// Persistent WebSocket sync state machine with exponential-backoff reconnects.
#[derive(Debug)]
pub struct WsSyncLoop {
    backoff: WsBackoff,
    /// Consecutive failed attempts since the last successful tick.
    pub consecutive_failures: u32,
    /// Successful sync ticks this session.
    pub ticks: u64,
}

impl WsSyncLoop {
    pub fn new(backoff: WsBackoff) -> Self {
        Self {
            backoff,
            consecutive_failures: 0,
            ticks: 0,
        }
    }

    /// Advance the machine once against the current link state.
    pub fn step(&mut self, internet_up: bool) -> WsStep {
        if internet_up {
            self.consecutive_failures = 0;
            self.ticks += 1;
            WsStep::Tick
        } else {
            let delay = self.backoff.delay_for(self.consecutive_failures);
            self.consecutive_failures = self.consecutive_failures.saturating_add(1);
            WsStep::Retry(delay)
        }
    }
}

/// Consecutive failed reconnects before a sync session gives up.
pub const WS_MAX_RECONNECT_ATTEMPTS: u32 = 4;

/// Ticks per sync session; the daemon starts a new session afterwards.
pub const WS_SESSION_TICKS: u64 = 64;

/// Persistent WebSocket sync loop with exponential-backoff reconnects.
///
/// `connection_alive` seeds the first link probe. Every iteration re-probes
/// the gateway's `internet_reconnected` flag, so a reconnect that happens
/// mid-session resumes ticking instead of dropping the loop. When the link
/// is down, the loop sleeps with exponential backoff (1s, 2s, 4s, ...) and
/// retries; the session ends after `WS_MAX_RECONNECT_ATTEMPTS` consecutive
/// failures or `WS_SESSION_TICKS` ticks. For a never-ending daemon loop,
/// drive [`run_websocket_sync`] instead.
pub fn handle_websocket(connection_alive: bool) {
    let mut sm = WsSyncLoop::new(WsBackoff::default());
    let mut up = connection_alive;
    loop {
        // Pick up a reconnect that happened since the last iteration.
        if internet_reconnected() {
            up = true;
        }
        match sm.step(up) {
            WsStep::Tick => {
                println!("WebSocket Sync Tick {}...", sm.ticks);
                if sm.ticks >= WS_SESSION_TICKS {
                    break;
                }
            }
            WsStep::Retry(delay) => {
                println!(
                    "WebSocket link down; reconnect attempt {} in {}ms...",
                    sm.consecutive_failures,
                    delay.as_millis()
                );
                std::thread::sleep(delay);
                // Stay down until the gateway flag flips.
                up = false;
            }
        }
        if sm.consecutive_failures >= WS_MAX_RECONNECT_ATTEMPTS {
            break;
        }
    }
    println!("WebSocket Connection Closed.");
}

/// Never-ending async driver for the persistent sync loop.
///
/// Ticks once per second while the link is up; on a drop it backs off
/// exponentially and keeps probing until the link returns. Returns only
/// when `max_attempts` consecutive failures occur (`u32::MAX` retries forever).
pub async fn run_websocket_sync(
    mut internet_up: impl FnMut() -> bool,
    mut on_tick: impl FnMut(u64),
    max_attempts: u32,
) {
    let mut sm = WsSyncLoop::new(WsBackoff::default());
    loop {
        match sm.step(internet_up()) {
            WsStep::Tick => {
                let t = sm.ticks;
                on_tick(t);
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
            WsStep::Retry(delay) => {
                tokio::time::sleep(delay).await;
            }
        }
        if sm.consecutive_failures >= max_attempts {
            break;
        }
    }
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

    #[test]
    fn ws_backoff_is_exponential_and_capped() {
        let b = WsBackoff {
            base: Duration::from_secs(1),
            max: Duration::from_secs(60),
        };
        assert_eq!(b.delay_for(0), Duration::from_secs(1));
        assert_eq!(b.delay_for(1), Duration::from_secs(2));
        assert_eq!(b.delay_for(2), Duration::from_secs(4));
        assert_eq!(b.delay_for(10), Duration::from_secs(60));
        assert_eq!(b.delay_for(u32::MAX), Duration::from_secs(60));
    }

    #[test]
    fn ws_sync_loop_reconnects_with_backoff_then_resumes() {
        let mut sm = WsSyncLoop::new(WsBackoff::default());
        // Link up: ticks, failures reset.
        assert_eq!(sm.step(true), WsStep::Tick);
        assert_eq!(sm.step(true), WsStep::Tick);
        assert_eq!(sm.ticks, 2);
        // Link drops: exponential backoff, failures accumulate.
        assert_eq!(sm.step(false), WsStep::Retry(Duration::from_secs(1)));
        assert_eq!(sm.step(false), WsStep::Retry(Duration::from_secs(2)));
        assert_eq!(sm.consecutive_failures, 2);
        // Reconnect: backoff resets, ticking resumes.
        assert_eq!(sm.step(true), WsStep::Tick);
        assert_eq!(sm.consecutive_failures, 0);
        assert_eq!(sm.step(false), WsStep::Retry(Duration::from_secs(1)));
    }
}
