//! Bundled engine.wasm for fast daemon cold starts (issue #38).
//!
//! The #11 WASM build pipeline produces `sandbox/assets/engine.wasm`. Release
//! builds enable the `bundled-engine` Cargo feature, which embeds those bytes
//! directly into the binary via [`include_bytes!`] so the daemon skips disk
//! I/O on startup. Without the feature (or without the asset), the loader
//! falls back to `None` and callers read the file from disk as before.
//!
//! Build the asset with `./devops/build-engine-wasm.sh` (owned by #11), then:
//! `cargo build -p sandbox --features bundled-engine`.

/// Embedded engine.wasm bytes. Only present with `bundled-engine`.
#[cfg(feature = "bundled-engine")]
pub const ENGINE_WASM_BYTES: &[u8] = include_bytes!("../assets/engine.wasm");

/// Engine WASM bytes for the sandbox loader, or `None` when no bundle is
/// embedded (feature off or asset missing at compile time).
pub fn engine_wasm_bytes() -> Option<&'static [u8]> {
    #[cfg(feature = "bundled-engine")]
    {
        Some(ENGINE_WASM_BYTES)
    }
    #[cfg(not(feature = "bundled-engine"))]
    {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundle_absent_without_feature() {
        // Default builds (this environment) carry no embedded engine.wasm;
        // the loader must report that honestly instead of panicking.
        #[cfg(not(feature = "bundled-engine"))]
        assert!(engine_wasm_bytes().is_none());
        #[cfg(feature = "bundled-engine")]
        {
            let bytes = engine_wasm_bytes().expect("bundled engine.wasm");
            assert!(bytes.starts_with(b"\0asm"), "must be a WASM module");
        }
    }
}
