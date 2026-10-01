//! Interop Module for MossyMesh
//!
//! Phase 5: AsyncAPI / OpenAPI gateway, TWAMM orchestration (2% max-spread),
//! and retroactive AMM liquidity mining for genesis offline nodes.

pub mod api_docs;
pub mod liquidity;
pub mod openapi_gateway;
pub mod twamm;

use axum::{
    extract::ConnectInfo,
    http::{header, HeaderMap, StatusCode},
    routing::{get, post},
    Router,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

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
pub use htlc::{hash_preimage, verify_preimage, Htlc, HtlcError, HtlcParams, HtlcState, MockVdf};

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
/// A non-loopback bind without `MESH_GATEWAY_TOKEN` set is refused outright
/// (fail closed); binding without a token is only allowed on loopback, where
/// the bind itself is the access control, and warns loudly (issue #186).
pub async fn run_http_server() {
    let app = Router::new()
        .route("/api/v1/health", get(health_handler).post(health_handler))
        .route("/api/v1/submit_job", post(submit_job_handler))
        // NOTE: /api-docs/openapi.json is served by api_docs::swagger_ui() below
        // (utoipa SwaggerUi::url). Registering it here as well panics Axum with
        // an overlapping-route error and kills the daemon on every boot.
        .merge(api_docs::swagger_ui());

    let bind = std::env::var("MESH_GATEWAY_BIND").unwrap_or_else(|_| "127.0.0.1:8080".into());
    match validate_gateway_bind(&bind, gateway_token().as_deref()) {
        GatewayBindDecision::Allow => {}
        GatewayBindDecision::AllowWithWarning => {
            eprintln!(
                "WARNING: MESH_GATEWAY_TOKEN is not set, so the job API on {bind} \
                 accepts unauthenticated requests. The loopback bind is the only \
                 access control. Set MESH_GATEWAY_TOKEN to require a Bearer <redacted>."
            );
        }
        GatewayBindDecision::Refuse => {
            eprintln!(
                "REFUSING to bind {bind}: a non-loopback MESH_GATEWAY_BIND without \
                 MESH_GATEWAY_TOKEN would expose the job API to the network \
                 unauthenticated. Set MESH_GATEWAY_TOKEN or bind loopback. Failing closed."
            );
            std::process::exit(1);
        }
    }
    let listener = tokio::net::TcpListener::bind(&bind).await.unwrap();
    println!("Interop: HTTP Server listening on {}", bind);
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .unwrap();
}

/// Outcome of validating the HTTP gateway bind address (issue #186).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GatewayBindDecision {
    /// Bind as requested.
    Allow,
    /// Bind, but authentication is disabled: warn loudly.
    AllowWithWarning,
    /// Refuse to start: fail closed.
    Refuse,
}

/// True when `bind` (a `host:port` string) targets the loopback interface.
/// Unparseable hosts are NOT loopback: validation fails closed.
fn bind_host_is_loopback(bind: &str) -> bool {
    let host = bind.rsplit_once(':').map(|(h, _)| h).unwrap_or(bind);
    let host = host.trim_matches(|c| c == '[' || c == ']');
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    host.parse::<IpAddr>()
        .map(|ip| ip.is_loopback())
        .unwrap_or(false)
}

/// Decide whether the gateway may bind `bind` given the auth configuration.
///
/// A non-loopback bind without a Bearer <redacted> would expose the job API to the
/// network unauthenticated, so it is refused outright. Binding without a
/// token is only acceptable on loopback, where the bind itself is the access
/// control, and even then the operator gets a loud warning at startup.
fn validate_gateway_bind(bind: &str, token: Option<&str>) -> GatewayBindDecision {
    let has_token = token.is_some_and(|t| !t.is_empty());
    if bind_host_is_loopback(bind) {
        if has_token {
            GatewayBindDecision::Allow
        } else {
            GatewayBindDecision::AllowWithWarning
        }
    } else if has_token {
        GatewayBindDecision::Allow
    } else {
        GatewayBindDecision::Refuse
    }
}

async fn health_handler() -> &'static str {
    "Mesh Island Active"
}

/// Bearer <redacted> for the job API. Read from `MESH_GATEWAY_TOKEN`; when unset,
/// the loopback-only bind is the access control (local daemon and UI only).
fn gateway_token() -> Option<String> {
    std::env::var("MESH_GATEWAY_TOKEN")
        .ok()
        .filter(|t| !t.is_empty())
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
    match dispatch_job(&body) {
        Ok(job) => {
            println!(
                "Dispatched '{}' job to DHT outbox (route key {:02x}, payload digest {:016x}).",
                job.action,
                job.route_key[0],
                payload_digest(&body)
            );
            Ok("Job Accepted")
        }
        Err(e) => {
            println!("Job rejected: {e:?}");
            Err(StatusCode::BAD_REQUEST)
        }
    }
}

/// A validated job accepted by the gateway, ready for DHT dispatch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeshJob {
    pub action: String,
    pub from: String,
    pub to: String,
    pub fen: String,
    /// Kademlia routing key: sha256(action || 0x00 || from || 0x00 || to || 0x00 || fen).
    /// The mesh-transport DHT publisher routes the job to the island nodes
    /// responsible for this key.
    pub route_key: [u8; 32],
}

/// Errors when turning an HTTP payload into a dispatchable [`MeshJob`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobDispatchError {
    /// Body is not valid JSON for the job schema.
    InvalidPayload,
    /// `action` is missing or empty; the router cannot classify the job.
    MissingAction,
    /// `move` jobs require a `fen` position.
    MissingFen,
}

/// Bounded outbox between the HTTP gateway and the mesh-transport DHT
/// publisher. The publisher drains it (see [`drain_job_outbox`]) and routes
/// each job under its `route_key`. Bounded so a flood of HTTP submissions
/// cannot grow memory without limit on edge nodes.
fn job_outbox() -> &'static Mutex<std::collections::VecDeque<MeshJob>> {
    static OUTBOX: OnceLock<Mutex<std::collections::VecDeque<MeshJob>>> = OnceLock::new();
    OUTBOX.get_or_init(|| Mutex::new(std::collections::VecDeque::new()))
}

/// Maximum jobs buffered for the DHT publisher.
pub const JOB_OUTBOX_CAP: usize = 1024;

/// Parse, validate, and route a `/api/v1/submit_job` body (issue #14).
///
/// The payload is decoded into the [`GenericPayload`] job struct, validated,
/// hashed to a Kademlia route key, and queued for the mesh-transport DHT
/// publisher instead of being printed to the console.
pub fn dispatch_job(body: &str) -> Result<MeshJob, JobDispatchError> {
    let payload: GenericPayload =
        serde_json::from_str(body).map_err(|_| JobDispatchError::InvalidPayload)?;
    if payload.action.trim().is_empty() {
        return Err(JobDispatchError::MissingAction);
    }
    if payload.action == "move" && payload.fen.trim().is_empty() {
        return Err(JobDispatchError::MissingFen);
    }

    let mut key_input = Vec::with_capacity(128);
    for part in [&payload.action, &payload.from, &payload.to, &payload.fen] {
        key_input.extend_from_slice(part.as_bytes());
        key_input.push(0x00);
    }
    let route_key: [u8; 32] = Sha256::digest(&key_input).into();

    let job = MeshJob {
        action: payload.action,
        from: payload.from,
        to: payload.to,
        fen: payload.fen,
        route_key,
    };

    if let Ok(mut outbox) = job_outbox().lock() {
        if outbox.len() >= JOB_OUTBOX_CAP {
            outbox.pop_front();
        }
        outbox.push_back(job.clone());
    }
    Ok(job)
}

/// Drain jobs queued for the mesh-transport DHT publisher.
pub fn drain_job_outbox() -> Vec<MeshJob> {
    job_outbox()
        .lock()
        .map(|mut outbox| outbox.drain(..).collect())
        .unwrap_or_default()
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
        "/api/v1/submit_job" => match dispatch_job(&req.payload) {
            Ok(_) => Ok("Job Accepted".to_string()),
            Err(_) => Err(InteropError::BadRequest),
        },
        "/api/v1/twamm" => handle_twamm(req),
        "/api/v1/liquidity" => handle_liquidity(req),
        "/api/v1/gateway" => handle_gateway(req),
        _ => Err(InteropError::ConnectionRefused),
    }
}

fn handle_twamm(req: &AsyncApiRequest) -> Result<String, InteropError> {
    let payload = req.payload.trim();
    if payload.is_empty() || payload.eq_ignore_ascii_case("status") {
        let book = twamm_book().lock().map_err(|_| InteropError::Timeout)?;
        return Ok(book.status_json());
    }

    // action=stream|submit + order fields
    if let Some((side, amount, slices, ref_price, exec_price)) = parse_order_payload(payload) {
        let mut book = twamm_book().lock().map_err(|_| InteropError::Timeout)?;
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

    for part in payload.split([',', '&', ';']) {
        let part = part
            .trim()
            .trim_matches(|c| c == '{' || c == '}' || c == '"');
        if part.is_empty() {
            continue;
        }
        let mut kv = part.splitn(2, ['=', ':']);
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
            m.account_json(&node_id)
                .map_err(|_| InteropError::BadRequest)
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
                // Idempotent claim (issue #46): a well-formed claim with
                // nothing left to claim is not a malformed request, so it
                // must not 400. Report zero tokens instead.
                Err(liquidity::LiquidityError::NothingToClaim) => Ok(format!(
                    "{{\"node_id\":\"{}\",\"tokens_airdropped\":0,\"status\":\"nothing_to_claim\"}}",
                    node_id
                )),
                Err(liquidity::LiquidityError::StillOffline) => Err(InteropError::GatewayDormant),
                Err(_) => Err(InteropError::BadRequest),
            }
        }
        "get" => {
            if node_id.is_empty() {
                return Err(InteropError::BadRequest);
            }
            m.account_json(&node_id)
                .map_err(|_| InteropError::BadRequest)
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

    for part in req.payload.split([',', '&', ';']) {
        let part = part
            .trim()
            .trim_matches(|c| c == '{' || c == '}' || c == '"');
        let mut kv = part.splitn(2, ['=', ':']);
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
            Err(openapi_gateway::GatewayError::Twamm(twamm::TwammError::SpreadExceeded {
                ..
            })) => Err(InteropError::SpreadCapExceeded),
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

/// Persistent WebSocket sync loop with exponential-backoff reconnects (issue #12).
///
/// `connection_alive` seeds the first link probe. Every iteration re-probes
/// the gateway's `internet_reconnected` flag, so a reconnect that happens
/// mid-session resumes ticking instead of dropping the loop. When the link
/// is down, the loop sleeps with exponential backoff (1s, 2s, 4s, ...) and
/// retries; the session ends after `WS_MAX_RECONNECT_ATTEMPTS` consecutive
/// failures or `WS_SESSION_TICKS` ticks. For a never-ending daemon loop,
/// drive [`run_websocket_sync`] instead.
///
/// Connections are tracked in a [`WsRegistry`] and pruned on disconnect, so an
/// abrupt peer drop can no longer leak a live connection.
pub fn handle_websocket(connection_alive: bool) {
    let mut registry = WsRegistry::new();
    let _id = registry.register("uplink-gateway");
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
    registry.close_all();
    debug_assert_eq!(registry.live_count(), 0, "connection leaked");
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

    /// Serializes tests that flip the process-global miner/gateway
    /// online/offline state. Rust runs tests in parallel, and without this
    /// lock one test's `signal_internet_disconnect()` can land between
    /// another test's `signal_internet_reconnect()` and its claim, flaking
    /// with `GatewayDormant` (seen in CI on PR #261).
    static NET_STATE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

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
            payload: r#"{"action":"move","fen":"rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1"}"#.into(),
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
        let _net_guard = NET_STATE_LOCK.lock().unwrap();
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
    fn liquidity_claim_is_idempotent_not_bad_request() {
        let _net_guard = NET_STATE_LOCK.lock().unwrap();
        // Issue #46: a well-formed claim must never 400 just because there
        // is nothing (left) to claim.
        let node = "genesis-claim-idem-1";
        signal_internet_disconnect();
        handle_rest_call(&AsyncApiRequest {
            endpoint: "/api/v1/liquidity".into(),
            payload: format!("action=register,node_id={node}"),
        })
        .unwrap();
        handle_rest_call(&AsyncApiRequest {
            endpoint: "/api/v1/liquidity".into(),
            payload: format!("action=accrue,node_id={node},epochs=2"),
        })
        .unwrap();

        signal_internet_reconnect();
        let first = handle_rest_call(&AsyncApiRequest {
            endpoint: "/api/v1/liquidity".into(),
            payload: format!("action=claim,node_id={node}"),
        })
        .unwrap();
        assert!(first.contains("\"status\":\"claimed\""));

        // Second claim: nothing left, but still a valid request (200, not 400).
        let second = handle_rest_call(&AsyncApiRequest {
            endpoint: "/api/v1/liquidity".into(),
            payload: format!("action=claim,node_id={node}"),
        })
        .unwrap();
        assert!(second.contains("\"status\":\"nothing_to_claim\""));
        assert!(second.contains("\"tokens_airdropped\":0"));
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
    fn dispatch_job_parses_routes_and_queues() {
        let _ = drain_job_outbox();
        let job = dispatch_job(
            r#"{"action":"move","from":"alice","to":"bob","fen":"rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1"}"#,
        )
        .expect("valid job");
        assert_eq!(job.action, "move");
        assert_eq!(job.from, "alice");
        // Route key is deterministic for the same payload.
        let again = dispatch_job(
            r#"{"action":"move","from":"alice","to":"bob","fen":"rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1"}"#,
        )
        .unwrap();
        assert_eq!(job.route_key, again.route_key);
        // Different payload -> different route key.
        let other = dispatch_job(
            r#"{"action":"move","from":"alice","to":"carol","fen":"8/8/8/8/8/8/8/8 w - - 0 1"}"#,
        )
        .unwrap();
        assert_ne!(job.route_key, other.route_key);
        // Queued for the DHT publisher drain (superset check: tests share
        // the process-wide outbox and run in parallel).
        let queued = drain_job_outbox();
        assert!(queued.len() >= 3);
        let keys: Vec<[u8; 32]> = queued.iter().map(|j| j.route_key).collect();
        assert!(keys.contains(&job.route_key));
        assert!(keys.contains(&other.route_key));
    }

    #[test]
    fn dispatch_job_rejects_bad_payloads() {
        assert_eq!(
            dispatch_job("not json"),
            Err(JobDispatchError::InvalidPayload)
        );
        assert_eq!(
            dispatch_job(r#"{"action":""}"#),
            Err(JobDispatchError::MissingAction)
        );
        assert_eq!(
            dispatch_job(r#"{"action":"move"}"#),
            Err(JobDispatchError::MissingFen)
        );
        // Rejected through the REST surface too.
        let err = handle_rest_call(&AsyncApiRequest {
            endpoint: "/api/v1/submit_job".into(),
            payload: "not json".into(),
        });
        assert_eq!(err, Err(InteropError::BadRequest));
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

    #[test]
    fn loopback_detection_covers_forms_and_fails_closed() {
        assert!(bind_host_is_loopback("127.0.0.1:8080"));
        assert!(bind_host_is_loopback("127.0.0.2:8080"));
        assert!(bind_host_is_loopback("[::1]:8080"));
        assert!(bind_host_is_loopback("localhost:8080"));
        assert!(bind_host_is_loopback("LOCALHOST:8080"));
        assert!(!bind_host_is_loopback("0.0.0.0:8080"));
        assert!(!bind_host_is_loopback("192.168.1.10:8080"));
        // Unparseable hosts are NOT loopback: fail closed.
        assert!(!bind_host_is_loopback("not-a-host:8080"));
        assert!(!bind_host_is_loopback(""));
    }

    #[test]
    fn gateway_bind_validation_matrix() {
        use GatewayBindDecision::{Allow, AllowWithWarning, Refuse};
        // Loopback without token: allowed, warns loudly.
        assert_eq!(
            validate_gateway_bind("127.0.0.1:8080", None),
            AllowWithWarning
        );
        assert_eq!(
            validate_gateway_bind("[::1]:8080", Some("")),
            AllowWithWarning
        );
        // Loopback with token: allowed.
        assert_eq!(
            validate_gateway_bind("127.0.0.1:8080", Some("secret")),
            Allow
        );
        // Non-loopback with token: allowed.
        assert_eq!(validate_gateway_bind("0.0.0.0:8080", Some("secret")), Allow);
        // Non-loopback without token: refused (fail closed).
        assert_eq!(validate_gateway_bind("0.0.0.0:8080", None), Refuse);
        assert_eq!(validate_gateway_bind("0.0.0.0:8080", Some("")), Refuse);
        assert_eq!(validate_gateway_bind("192.168.1.10:8080", None), Refuse);
    }
}
