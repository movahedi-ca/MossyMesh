//! OpenAPI 3.1 documentation for the MossyMesh interop HTTP gateway.
//!
//! The gateway handlers take axum extractors (`ConnectInfo`, `HeaderMap`)
//! that utoipa's `#[utoipa::path]` macro cannot introspect, so the paths
//! are documented on these empty doc functions instead of on the real
//! handlers. The DTOs mirror the hand-built JSON the handlers return.

use serde::Serialize;
use utoipa::{OpenApi, ToSchema};
use utoipa_swagger_ui::SwaggerUi;

/// `GET /api/v1/health` response.
#[derive(Serialize, ToSchema)]
pub struct HealthResponse {
    /// Always `"ok"` when the gateway is live.
    pub status: String,
}

/// `POST /api/v1/submit_job` request body.
#[derive(Serialize, ToSchema)]
pub struct JobSubmission {
    /// Job action, e.g. `"move"`.
    pub action: String,
    /// Origin island node.
    pub from: String,
    /// Destination island node.
    pub to: String,
    /// Chess position in FEN notation (required for `"move"` jobs).
    pub fen: String,
}

/// `POST /api/v1/submit_job` response.
#[derive(Serialize, ToSchema)]
pub struct JobAccepted {
    /// `"Job Accepted"` or `"Job Rejected"`.
    pub result: String,
}

/// `POST /api/v1/liquidity` request body.
#[derive(Serialize, ToSchema)]
pub struct LiquidityAction {
    /// One of `"register"`, `"accrue"`, `"claim"`.
    pub action: String,
    /// Genesis node id.
    pub node_id: String,
    /// Offline epochs to accrue (accrue only).
    pub epochs: u64,
}

/// `POST /api/v1/liquidity` response.
#[derive(Serialize, ToSchema)]
pub struct LiquidityResponse {
    /// Echo of the node id.
    pub node_id: String,
    /// Tokens moved by this call.
    pub tokens_airdropped: u64,
    /// `"claimed"` or `"nothing_to_claim"` (idempotent claim).
    pub status: String,
}

/// Error body returned with 4xx/5xx statuses.
#[derive(Serialize, ToSchema)]
pub struct ErrorResponse {
    /// Machine-readable error, e.g. `"bad_request"`.
    pub error: String,
}

/// Gateway liveness probe.
#[utoipa::path(
    get,
    path = "/api/v1/health",
    tag = "health",
    responses(
        (status = 200, description = "Gateway is live", body = HealthResponse),
    )
)]
#[allow(dead_code)]
fn health_doc() {}

/// Submit a mesh job for DHT dispatch.
#[utoipa::path(
    post,
    path = "/api/v1/submit_job",
    tag = "interop",
    request_body = JobSubmission,
    responses(
        (status = 200, description = "Job accepted for dispatch", body = JobAccepted),
        (status = 400, description = "Malformed job payload", body = ErrorResponse),
    )
)]
#[allow(dead_code)]
fn submit_job_doc() {}

/// Liquidity mining: register, accrue offline epochs, claim airdrop.
#[utoipa::path(
    post,
    path = "/api/v1/liquidity",
    tag = "interop",
    request_body = LiquidityAction,
    responses(
        (status = 200, description = "Action applied", body = LiquidityResponse),
        (status = 400, description = "Malformed action or unknown node", body = ErrorResponse),
        (status = 503, description = "Gateway dormant (offline) for this action", body = ErrorResponse),
    )
)]
#[allow(dead_code)]
fn liquidity_doc() {}

/// The interop gateway's OpenAPI document.
#[derive(OpenApi)]
#[openapi(
    paths(health_doc, submit_job_doc, liquidity_doc),
    components(schemas(
        HealthResponse,
        JobSubmission,
        JobAccepted,
        LiquidityAction,
        LiquidityResponse,
        ErrorResponse,
    )),
    tags(
        (name = "health", description = "Gateway liveness"),
        (name = "interop", description = "Job submission and liquidity mining"),
    ),
    info(
        title = "MossyMesh Interop API",
        version = "0.1.0",
        description = "HTTP surface of the MossyMesh interop gateway: health, job submission, and liquidity mining."
    )
)]
pub struct ApiDoc;

/// Render the OpenAPI document as a JSON string.
pub fn openapi_json() -> String {
    ApiDoc::openapi()
        .to_json()
        .expect("OpenAPI document serialization cannot fail")
}

/// Axum handler serving the OpenAPI document.
pub async fn serve_openapi_json() -> String {
    openapi_json()
}

/// Swagger UI explorer, served at `/swagger-ui`.
pub fn swagger_ui() -> SwaggerUi {
    SwaggerUi::new("/swagger-ui").url("/api-docs/openapi.json", ApiDoc::openapi())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openapi_document_lists_all_gateway_paths() {
        let doc: serde_json::Value =
            serde_json::from_str(&openapi_json()).expect("spec must be valid JSON");
        let paths = doc
            .get("paths")
            .expect("spec must have paths")
            .as_object()
            .unwrap();
        for path in ["/api/v1/health", "/api/v1/submit_job", "/api/v1/liquidity"] {
            assert!(paths.contains_key(path), "missing path {path}");
        }
    }
}
