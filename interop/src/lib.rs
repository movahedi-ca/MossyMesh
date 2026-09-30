//! Interop Module for MossyMesh
//!
//! Phase 5: AsyncAPI / OpenAPI gateway, TWAMM orchestration (2% max-spread),
//! and retroactive AMM liquidity mining for genesis offline nodes.

pub mod liquidity;
pub mod openapi_gateway;
pub mod twamm;

use std::sync::{Mutex, OnceLock};
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::time::{Duration, Instant};
use axum::{
    extract::{ConnectInfo, State},
    http::{header, HeaderMap, StatusCode},
    routing::{get, post},
    Router, Json,
};
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

/// Starts an Axum HTTP server for the frontend.
///
/// Binds loopback by default (override with `MESH_GATEWAY_BIND`); the old
/// 0.0.0.0 bind exposed an unauthenticated remote surface on shared LANs.
pub async fn run_http_server() {
    let app = Router::new()
        .route("/api/v1/health", get(health_handler).post(health_handler))
        .route("/api/v1/submit_job", post(submit_job_handler));

    let bind = std::env::var("MESH_GATEWAY_BIND").unwrap_or_else(|_| "127.0.0.1:8080".into());
    let listener = tokio::net::TcpListener::bind(&bind).await.unwrap();
    println!("Interop: HTTP Server listening on {}", bind);
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .unwrap();
}

async fn health_handler() -> &'static str {
    "Mesh Island Active"
}

/// Bearer token for the job API. Read from `MESH_GATEWAY_TOKEN`; when unset,
/// the loopback-only bind is the access control (local daemon and UI only).
fn gateway_token() -> Option<String> {
    std::env::var("MESH_GATEWAY_TOKEN").ok().filter(|t| !t.is_empty())
}

fn check_auth(headers: &HeaderMap) -> bool {
    let Some(expected) = gateway_token() else {
        return true;
    };
    let Some(value) = headers.get(header::AUTHORIZATION) else {
        return false;
    };
    let Ok(value) = value.to_str() else {
        return false;
    };
    let Some(presented) = value.strip_prefix("Bearer ") else {
        return false;
    };
    presented.len() == expected.len()
        && presented.bytes().zip(expected.bytes()).all(|(a, b)| a == b)
}

/// Bounded per-peer rate limiter: 60 requests per 60 seconds per IP.
/// The table is capped at 1024 peers so a LAN-wide scan cannot grow it
/// without bound.
fn rate_limited_peers() -> &'static Mutex<HashMap<IpAddr, Vec<Instant>>> {
    static RL: OnceLock<Mutex<HashMap<IpAddr, Vec<Instant>>>> = OnceLock::new();
    RL.get_or_init(|| Mutex::new(HashMap::new()))
}

const RATE_LIMIT: usize = 60;
const RATE_WINDOW: Duration = Duration::from_secs(60);

fn check_rate_limit(ip: IpAddr) -> bool {
    let now = Instant::now();
    let mut table = match rate_limited_peers().lock() {
        Ok(t) => t,
        Err(_) => return false,
    };
    if table.len() > 1024 && !table.contains_key(&ip) {
        return false;
    }
    let entry = table.entry(ip).or_default();
    entry.retain(|t| now.duration_since(*t) < RATE_WINDOW);
    if entry.len() >= RATE_LIMIT {
        return false;
    }
    entry.push(now);
    true
}

/// Payload digest for logs: never print raw request bodies (log injection
/// on a shared LAN, and bodies may carry private job data).
fn payload_digest(body: &str) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    body.hash(&mut h);
    h.finish()
}

async fn submit_job_handler(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: String,
) -> Result<&'static str, StatusCode> {
    if !check_auth(&headers) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    if !check_rate_limit(addr.ip()) {
        return Err(StatusCode::TOO_MANY_REQUESTS);
    }
    println!(
        "Routing job payload ({} bytes, digest {:016x}) into Kademlia DHT/Sandbox...",
        body.len(),
        payload_digest(&body)
    );
    Ok("Job Accepted")
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
            println!(
                "Routing job payload ({} bytes, digest {:016x}) into Kademlia DHT...",
                req.payload.len(),
                payload_digest(&req.payload)
            );
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

/// Handle for one WebSocket peer connection.
///
/// Dropping the handle closes the connection. This is the guarantee behind
/// the fix for #48: no code path can orphan a live connection, because the
/// connection dies with its handle.
pub struct WsConnection {
    id: u64,
    peer: String,
    closed: bool,
}

impl WsConnection {
    fn new(id: u64, peer: impl Into<String>) -> Self {
        Self {
            id,
            peer: peer.into(),
            closed: false,
        }
    }

    /// Close the connection immediately. Idempotent.
    pub fn close(&mut self) {
        if !self.closed {
            self.closed = true;
            println!("WebSocket connection {} ({}) closed.", self.id, self.peer);
        }
    }

    pub fn is_closed(&self) -> bool {
        self.closed
    }
}

impl Drop for WsConnection {
    fn drop(&mut self) {
        self.close();
    }
}

/// Registry of live WebSocket connections.
///
/// Connections that die abruptly (peer disconnect without a clean close) are
/// removed here, so they cannot accumulate. Fixes #48.
#[derive(Default)]
pub struct WsRegistry {
    next_id: u64,
    live: Vec<WsConnection>,
}

impl WsRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a new outbound connection.
    pub fn register(&mut self, peer: impl Into<String>) -> u64 {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        self.live.push(WsConnection::new(id, peer));
        id
    }

    /// Remove and close a connection by id (abrupt peer disconnect path).
    /// Returns true if a live connection was found and removed.
    pub fn abrupt_disconnect(&mut self, id: u64) -> bool {
        if let Some(pos) = self.live.iter().position(|c| c.id == id) {
            let mut conn = self.live.remove(pos);
            conn.close();
            true
        } else {
            false
        }
    }

    /// Close and drop every live connection.
    pub fn close_all(&mut self) {
        while let Some(mut conn) = self.live.pop() {
            conn.close();
        }
    }

    pub fn live_count(&self) -> usize {
        self.live.len()
    }
}

/// Ongoing WebSocket event loop syncing state to the external internet.
///
/// Connections are tracked in a [`WsRegistry`] and pruned on disconnect, so an
/// abrupt peer drop can no longer leak a live connection.
pub fn handle_websocket(connection_alive: bool) {
    let mut registry = WsRegistry::new();
    let id = registry.register("uplink-gateway");
    let mut tick = 0;
    let mut alive = connection_alive;
    while alive && tick < 3 {
        println!("WebSocket Sync Tick {}...", tick);
        tick += 1;
        if tick == 2 {
            // Simulated abrupt peer disconnect: prune from the registry so the
            // connection is dropped instead of leaking.
            registry.abrupt_disconnect(id);
            alive = false;
        }
    }
    registry.close_all();
    debug_assert_eq!(registry.live_count(), 0, "connection leaked");
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

    #[test]
    fn abrupt_disconnect_prunes_connection() {
        let mut registry = WsRegistry::new();
        let a = registry.register("peer-a");
        let _b = registry.register("peer-b");
        assert_eq!(registry.live_count(), 2);
        assert!(registry.abrupt_disconnect(a));
        assert_eq!(registry.live_count(), 1);
        // Unknown id is a no-op, never panics.
        assert!(!registry.abrupt_disconnect(9999));
    }

    #[test]
    fn close_all_drains_registry() {
        let mut registry = WsRegistry::new();
        registry.register("peer-a");
        registry.register("peer-b");
        registry.close_all();
        assert_eq!(registry.live_count(), 0);
    }

    #[test]
    fn websocket_sync_loop_leaks_nothing() {
        handle_websocket(true);
        handle_websocket(false);
    }
}
