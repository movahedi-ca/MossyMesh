//! Interop Module for MossyMesh
//! This is a Phase 1 stub for mocking AsyncAPI endpoints.

pub fn init_interop() {
    println!("Interop (stub): Mocking AsyncAPI and OpenAPI gateway endpoints...");
}

pub struct AsyncApiRequest {
    pub endpoint: String,
    pub payload: String,
}
pub fn handle_rest_call() {}
pub fn handle_websocket() {}

pub enum InteropError {
    Timeout,
    ConnectionRefused,
}
