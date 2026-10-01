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
use twamm::{parse_order_payload, OrderSide, TwammEngine, TwammError, MAX_SPREAD_BPS};

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

/// Build the HTTP router for the gateway.
///
/// Extracted so tests can construct the router without binding a socket.
/// Issue #234: `/api-docs/openapi.json` was registered both explicitly and
/// via `swagger_ui()`, which panicked axum at startup. The spec is now served
/// exactly once, through the SwaggerUi registration.
fn build_router() -> Router {
    Router::new()
        .route("/api/v1/health", get(health_handler).post(health_handler))
        .route("/api/v1/submit_job", post(submit_job_handler))
        // Issue #159: the OpenAPI spec documents these endpoints, so they
        // must be reachable over HTTP, not only through handle_rest_call.
        // (/api-docs/openapi.json is deliberately NOT registered here: it is
        // already served once via swagger_ui(), and a second registration
        // panics axum with "Overlapping method route" — issue #234.)
        .route("/api/v1/liquidity", post(rest_shim_handler))
        .route("/api/v1/twamm", post(rest_shim_handler))
        .route("/api/v1/gateway", post(rest_shim_handler))
        .merge(api_docs::swagger_ui())
}

/// Starts an Axum HTTP server for the frontend.
///
/// Binds loopback by default (override with `MESH_GATEWAY_BIND`); the old
/// 0.0.0.0 bind exposed an unauthenticated remote surface on shared LANs.
/// Returns an error instead of panicking so the daemon can log the failure
/// and exit non-zero (issue #150): an unwrap() here would take down the whole
/// process with an unlogged panic on something as mundane as a port clash.
pub async fn run_http_server() -> Result<(), HttpServerError> {
    // Note: the OpenAPI JSON is served by swagger_ui() via
    // `.url("/api-docs/openapi.json", ...)`; registering an extra explicit
    // route for the same path panics in axum ("Overlapping method route").
    let app = build_router();

    let bind = std::env::var("MESH_GATEWAY_BIND").unwrap_or_else(|_| "127.0.0.1:8080".into());
    let listener = tokio::net::TcpListener::bind(&bind)
        .await
        .map_err(|e| HttpServerError::Bind(format!("cannot bind gateway to {bind}: {e}")))?;
    println!("Interop: HTTP Server listening on {}", bind);
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .map_err(HttpServerError::Serve)?;
    Ok(())
}

/// Errors starting or serving the interop HTTP gateway.
#[derive(Debug)]
pub enum HttpServerError {
    /// The bind address could not be parsed, bound, or was refused by policy.
    Bind(String),
    /// Serving failed after a successful bind.
    Serve(std::io::Error),
}

impl std::fmt::Display for HttpServerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HttpServerError::Bind(msg) => write!(f, "gateway bind failed: {msg}"),
            HttpServerError::Serve(e) => write!(f, "gateway serve failed: {e}"),
        }
    }
}

impl std::error::Error for HttpServerError {}

/// Map an [`InteropError`] from the REST shim to an HTTP status.
fn map_interop_error(e: InteropError) -> StatusCode {
    match e {
        InteropError::BadRequest => StatusCode::BAD_REQUEST,
        InteropError::ConnectionRefused => StatusCode::NOT_FOUND,
        InteropError::Timeout => StatusCode::GATEWAY_TIMEOUT,
        InteropError::SpreadCapExceeded => StatusCode::UNPROCESSABLE_ENTITY,
        InteropError::GatewayDormant => StatusCode::SERVICE_UNAVAILABLE,
    }
}

/// Axum handler exposing the `/api/v1/{liquidity,twamm,gateway}` endpoints
/// over HTTP via the [`handle_rest_call`] shim (issue #159).
///
/// Same auth and rate-limit gates as `submit_job`; the path selects the shim
/// endpoint and the body is the shim payload.
async fn rest_shim_handler(
    uri: axum::http::Uri,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: String,
) -> Result<String, StatusCode> {
    if !check_auth(&headers) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    if !check_rate_limit(addr.ip()) {
        return Err(StatusCode::TOO_MANY_REQUESTS);
    }
    handle_rest_call(&AsyncApiRequest {
        endpoint: uri.path().to_string(),
        payload: body,
    })
    .map_err(map_interop_error)
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
///
/// Issue #160: the old table stored only timestamps and never evicted stale
/// entries, so once 1024 distinct IPs had ever been seen, every new IP was
/// rejected forever. Each entry now records a last-seen instant; when a new
/// IP arrives at a full table, entries whose last-seen is older than the
/// window are evicted first, and only a genuinely full table rejects.
#[derive(Debug, Default)]
struct RateLimiter {
    peers: HashMap<IpAddr, RatePeerState>,
}

/// Per-IP rate-limit state.
#[derive(Debug, Clone)]
struct RatePeerState {
    /// Request instants inside the current window (pruned on each check).
    stamps: Vec<Instant>,
    /// Last request seen from this IP, used for stale-entry eviction.
    last_seen: Instant,
}

/// Max distinct IPs tracked before eviction kicks in.
const RATE_LIMIT_PEERS_CAP: usize = 1024;

impl RateLimiter {
    fn check(&mut self, ip: IpAddr, now: Instant) -> bool {
        // A new IP at a full table: evict entries idle longer than the
        // window before turning anyone away.
        if self.peers.len() >= RATE_LIMIT_PEERS_CAP && !self.peers.contains_key(&ip) {
            self.peers
                .retain(|_, st| now.saturating_duration_since(st.last_seen) < RATE_WINDOW);
        }
        if self.peers.len() >= RATE_LIMIT_PEERS_CAP && !self.peers.contains_key(&ip) {
            return false;
        }
        let entry = self.peers.entry(ip).or_insert_with(|| RatePeerState {
            stamps: Vec::new(),
            last_seen: now,
        });
        entry.last_seen = now;
        entry
            .stamps
            .retain(|t| now.saturating_duration_since(*t) < RATE_WINDOW);
        if entry.stamps.len() >= RATE_LIMIT {
            return false;
        }
        entry.stamps.push(now);
        true
    }

    #[cfg(test)]
    fn peer_count(&self) -> usize {
        self.peers.len()
    }
}

fn rate_limited_peers() -> &'static Mutex<RateLimiter> {
    static RL: OnceLock<Mutex<RateLimiter>> = OnceLock::new();
    RL.get_or_init(|| Mutex::new(RateLimiter::default()))
}

const RATE_LIMIT: usize = 60;
const RATE_WINDOW: Duration = Duration::from_secs(60);

fn check_rate_limit(ip: IpAddr) -> bool {
    match rate_limited_peers().lock() {
        Ok(mut limiter) => limiter.check(ip, Instant::now()),
        Err(_) => false,
    }
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
            Err(match e {
                // Issue #162: backpressure surfaces as HTTP 429.
                JobDispatchError::OutboxFull => StatusCode::TOO_MANY_REQUESTS,
                _ => StatusCode::BAD_REQUEST,
            })
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
    /// The outbox is full: the job was NOT accepted; back off and retry.
    OutboxFull,
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

/// Push a validated job into the outbox.
///
/// Issue #162: when the outbox is full this returns
/// [`JobDispatchError::OutboxFull`] (HTTP 429 upstream) instead of silently
/// evicting the oldest pending job. The caller was never told a job vanished;
/// now the rejection is explicit and the queued jobs are untouched.
fn enqueue_job(
    outbox: &mut std::collections::VecDeque<MeshJob>,
    job: MeshJob,
) -> Result<(), JobDispatchError> {
    if outbox.len() >= JOB_OUTBOX_CAP {
        return Err(JobDispatchError::OutboxFull);
    }
    outbox.push_back(job);
    Ok(())
}
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
        enqueue_job(&mut outbox, job.clone())?;
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
            // Issue #162: backpressure is explicit, not a silent drop.
            Err(JobDispatchError::OutboxFull) => Err(InteropError::TooManyRequests),
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
            // Issue #161: stream every requested slice, not just the first;
            // previously the remaining slices were stranded in the book with
            // no REST path to reach them.
            let mut streamed = 0u32;
            let mut total_in = 0u64;
            let mut total_out = 0u64;
            let mut max_bps = 0u32;
            let mut partial = false;
            loop {
                match book.stream_slice(&id, exec) {
                    Ok(fill) => {
                        streamed += 1;
                        total_in = total_in.saturating_add(fill.amount_in);
                        total_out = total_out.saturating_add(fill.amount_out);
                        max_bps = max_bps.max(fill.spread_bps);
                    }
                    // A pruned (fully streamed) order reads back as
                    // OrderNotFound; either way, there is nothing left.
                    Err(TwammError::OrderExhausted) | Err(TwammError::OrderNotFound) => break,
                    Err(e) => {
                        println!("TWAMM stream error: {e}");
                        if streamed == 0 {
                            return Err(InteropError::SpreadCapExceeded);
                        }
                        // Later slices see a moved mid; report what filled.
                        partial = true;
                        break;
                    }
                }
            }
            return Ok(serde_json::json!({
                "order_id": id,
                "side": match side {
                    OrderSide::Buy => "buy",
                    OrderSide::Sell => "sell",
                },
                "slices_streamed": streamed,
                "amount_in": total_in,
                "amount_out": total_out,
                "max_spread_bps_seen": max_bps,
                "max_spread_bps": MAX_SPREAD_BPS,
                "partial": partial,
            })
            .to_string());
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
    /// Job outbox is full: backpressure, the client should retry later.
    TooManyRequests,
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

    /// Issue #150: a bind failure returns Err instead of panicking, so the
    /// daemon can log it and exit non-zero.
    #[tokio::test]
    async fn http_server_bind_conflict_returns_error_not_panic() {
        // Occupy a port, then point the gateway at it: the bind must fail
        // with HttpServerError::Bind rather than unwrap-panicking.
        let squatter = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = squatter.local_addr().unwrap().port();
        std::env::set_var("MESH_GATEWAY_BIND", format!("127.0.0.1:{port}"));

        let err = run_http_server().await.unwrap_err();
        assert!(
            matches!(err, HttpServerError::Bind(_)),
            "expected Bind error, got {err:?}"
        );
        assert!(err.to_string().contains(&port.to_string()));

        std::env::remove_var("MESH_GATEWAY_BIND");
        drop(squatter);
    }

    /// Issue #234: building the router must not panic on overlapping routes.
    /// Axum panics at registration time when two handlers claim the same
    /// path, which took the daemon down at boot; this test builds the exact
    /// router run_http_server serves.
    #[test]
    fn router_builds_without_overlapping_routes() {
        let _ = build_router();
    }

    /// Issue #159: the shim endpoints are reachable through the HTTP handler
    /// with the same auth/rate-limit gates as submit_job.
    ///
    /// Single test (not split): both phases touch the process-global
    /// MESH_GATEWAY_TOKEN, and parallel tests must not race on it.
    #[tokio::test]
    async fn rest_shim_handler_serves_endpoints_and_enforces_auth() {
        async fn call(path: &str, body: &str, headers: HeaderMap) -> Result<String, StatusCode> {
            let uri: axum::http::Uri = path.parse().unwrap();
            let addr: SocketAddr = "127.0.0.1:55991".parse().unwrap();
            rest_shim_handler(uri, ConnectInfo(addr), headers, body.into()).await
        }

        // Phase 1: no token configured, the documented endpoints serve.
        let liq = call("/api/v1/liquidity", "status", HeaderMap::new())
            .await
            .unwrap();
        assert!(liq.contains("internet_reconnected"), "got: {liq}");

        let tw = call("/api/v1/twamm", "status", HeaderMap::new())
            .await
            .unwrap();
        assert!(tw.contains("max_spread_bps"), "got: {tw}");

        let gw = call("/api/v1/gateway", "status", HeaderMap::new())
            .await
            .unwrap();
        assert!(gw.contains("active"), "got: {gw}");

        // Unknown paths 404 via the shim mapping.
        let err = call("/api/v1/nope", "", HeaderMap::new())
            .await
            .unwrap_err();
        assert_eq!(err, StatusCode::NOT_FOUND);

        // Phase 2: with a token configured, auth is enforced.
        std::env::set_var("MESH_GATEWAY_TOKEN", "test-token-159");
        let err = call("/api/v1/liquidity", "status", HeaderMap::new())
            .await
            .unwrap_err();
        assert_eq!(err, StatusCode::UNAUTHORIZED);

        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            "Bearer test-token-159".parse().unwrap(),
        );
        let ok = call("/api/v1/liquidity", "status", headers).await;
        assert!(ok.is_ok(), "authed call should pass: {ok:?}");

        std::env::remove_var("MESH_GATEWAY_TOKEN");
    }
    /// Issue #160: per-IP 60/60s limit still enforced.
    #[test]
    fn rate_limiter_enforces_per_ip_window() {
        let mut limiter = RateLimiter::default();
        let t0 = Instant::now();
        let ip: IpAddr = "10.9.0.1".parse().unwrap();
        for _ in 0..RATE_LIMIT {
            assert!(limiter.check(ip, t0));
        }
        assert!(
            !limiter.check(ip, t0),
            "61st request in the window is rejected"
        );
        // After the window passes, the IP is allowed again.
        let later = t0 + RATE_WINDOW + Duration::from_secs(1);
        assert!(limiter.check(ip, later));
        assert_eq!(limiter.peer_count(), 1);
    }

    /// Issue #160: stale entries are evicted so the 1024 cap cannot become a
    /// permanent blocklist.
    #[test]
    fn rate_limiter_evicts_stale_ips_at_cap() {
        let mut limiter = RateLimiter::default();
        let t0 = Instant::now();
        // Fill the table to the cap with distinct IPs.
        for i in 0..RATE_LIMIT_PEERS_CAP {
            let ip: IpAddr = format!("10.10.{}.{}", i / 256, i % 256).parse().unwrap();
            assert!(limiter.check(ip, t0));
        }
        assert_eq!(limiter.peer_count(), RATE_LIMIT_PEERS_CAP);

        // A fresh IP at a genuinely full table is still rejected...
        let fresh: IpAddr = "10.11.0.1".parse().unwrap();
        assert!(!limiter.check(fresh, t0 + Duration::from_secs(1)));

        // ...but after the window, all entries are stale, so the same IP is
        // admitted and the stale blocklist is gone.
        let later = t0 + RATE_WINDOW + Duration::from_secs(1);
        assert!(limiter.check(fresh, later));
        assert_eq!(limiter.peer_count(), 1);
    }

    /// Issue #160: an idle IP keeps its slot across the sweep while active
    /// IPs are retained; a returning stale IP starts with a clean window.
    #[test]
    fn rate_limiter_keeps_active_ips_during_eviction() {
        let mut limiter = RateLimiter::default();
        let t0 = Instant::now();
        let busy: IpAddr = "10.12.0.1".parse().unwrap();
        for i in 1..RATE_LIMIT_PEERS_CAP {
            let ip: IpAddr = format!("10.13.{}.{}", i / 256, i % 256).parse().unwrap();
            limiter.check(ip, t0);
        }
        // `busy` stays active past the window.
        limiter.check(busy, t0);
        limiter.check(busy, t0 + RATE_WINDOW + Duration::from_secs(1));
        assert_eq!(limiter.peer_count(), RATE_LIMIT_PEERS_CAP);

        // New IP arrives at a full table: the stale 1023 are evicted, `busy`
        // keeps its timestamps.
        let fresh: IpAddr = "10.12.0.2".parse().unwrap();
        assert!(limiter.check(fresh, t0 + RATE_WINDOW + Duration::from_secs(2)));
        assert_eq!(limiter.peer_count(), 2);

        // `busy` hit the per-IP cap long ago is irrelevant here; verify its
        // window accounting still works: 59 more allowed at this instant.
        let now = t0 + RATE_WINDOW + Duration::from_secs(2);
        for _ in 0..(RATE_LIMIT - 1) {
            assert!(limiter.check(busy, now));
        }
        assert!(!limiter.check(busy, now));
    }

    /// Issue #161: a REST TWAMM submit with slices > 1 streams every slice,
    /// and the settled order is pruned from the book.
    #[test]
    fn twamm_rest_submit_streams_all_slices() {
        let resp = handle_rest_call(&AsyncApiRequest {
            endpoint: "/api/v1/twamm".into(),
            payload: "side=sell,amount=1000000,slices=4,ref_price=1000000,exec_price=1000000"
                .into(),
        })
        .unwrap();
        let v: serde_json::Value = serde_json::from_str(&resp).unwrap();
        assert_eq!(v["slices_streamed"], 4, "response: {resp}");
        assert_eq!(v["amount_in"], 1_000_000);
        assert_eq!(v["partial"], false);
        // Order id from the response no longer exists in the book: pruned.
        let id = v["order_id"].as_str().unwrap();
        let book = twamm_book().lock().unwrap();
        assert!(book.get_order(id).is_none(), "order {id} should be pruned");
    }

    /// Issue #161: spread failure on the first slice still hard-rejects with
    /// no partial state (existing 2%-cap behavior preserved).
    #[test]
    fn twamm_rest_submit_spread_reject_unchanged() {
        let err = handle_rest_call(&AsyncApiRequest {
            endpoint: "/api/v1/twamm".into(),
            payload: "side=sell,amount=1000000,slices=4,ref_price=1000000,exec_price=1030000"
                .into(),
        });
        assert_eq!(err, Err(InteropError::SpreadCapExceeded));
    }

    fn dummy_job(i: u8) -> MeshJob {
        MeshJob {
            action: "fill".into(),
            from: "t".into(),
            to: "t".into(),
            fen: String::new(),
            route_key: [i; 32],
        }
    }

    /// Issue #162: the local enqueue rejects at the cap and never evicts the
    /// oldest job.
    #[test]
    #[allow(clippy::cast_possible_truncation)]
    fn enqueue_job_rejects_at_cap_without_eviction() {
        let mut outbox = std::collections::VecDeque::new();
        for i in 0..JOB_OUTBOX_CAP {
            enqueue_job(&mut outbox, dummy_job(i as u8)).unwrap();
        }
        assert_eq!(outbox.len(), JOB_OUTBOX_CAP);

        let before: Vec<[u8; 32]> = outbox.iter().map(|j| j.route_key).collect();
        assert_eq!(
            enqueue_job(&mut outbox, dummy_job(255)),
            Err(JobDispatchError::OutboxFull)
        );
        // Queue untouched: no silent drop of the oldest job.
        let after: Vec<[u8; 32]> = outbox.iter().map(|j| j.route_key).collect();
        assert_eq!(before, after);
        assert_eq!(outbox.len(), JOB_OUTBOX_CAP);
    }

    /// Issue #162: end to end, a full outbox surfaces backpressure through
    /// dispatch_job, the REST shim, and the HTTP handler (429).
    #[tokio::test]
    #[allow(clippy::cast_possible_truncation)]
    async fn outbox_full_surfaces_backpressure_everywhere() {
        {
            let mut outbox = job_outbox().lock().unwrap();
            outbox.clear();
            for i in 0..JOB_OUTBOX_CAP {
                outbox.push_back(dummy_job(i as u8));
            }
        }

        // dispatch_job rejects loudly; nothing evicted.
        assert_eq!(
            dispatch_job(r#"{"action":"ping"}"#).unwrap_err(),
            JobDispatchError::OutboxFull
        );
        assert_eq!(job_outbox().lock().unwrap().len(), JOB_OUTBOX_CAP);

        // REST shim maps it to backpressure.
        let rest = handle_rest_call(&AsyncApiRequest {
            endpoint: "/api/v1/submit_job".into(),
            payload: r#"{"action":"ping"}"#.into(),
        });
        assert_eq!(rest, Err(InteropError::TooManyRequests));

        // HTTP handler maps it to 429.
        let addr: SocketAddr = "127.0.0.1:55994".parse().unwrap();
        let err = submit_job_handler(
            ConnectInfo(addr),
            HeaderMap::new(),
            r#"{"action":"ping"}"#.into(),
        )
        .await
        .unwrap_err();
        assert_eq!(err, StatusCode::TOO_MANY_REQUESTS);

        // Cleanup so other tests sharing the global outbox are unaffected.
        let _ = drain_job_outbox();
        assert_eq!(job_outbox().lock().unwrap().len(), 0);
    }

}
