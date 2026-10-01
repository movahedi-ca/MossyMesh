//! Bundled engine.wasm for fast daemon cold starts (issue #38).
//!
//! The #11 WASM build pipeline produces `sandbox/assets/engine.wasm`. Release
//! builds enable the `bundled-engine` Cargo feature, which embeds those bytes
//! directly into the binary via [`include_bytes!`] so the daemon skips disk
//! I/O on startup.
//!
//! The asset is gitignored and absent on a fresh checkout, and
//! [`include_bytes!`] is a compile-time macro, so the embed is gated behind
//! the `mossymesh_engine_wasm_bundled` cfg that `build.rs` sets only when
//! `assets/engine.wasm` exists at compile time. With the feature on but the
//! asset missing, the build still succeeds and the loader falls back to
//! `None` (callers read the file from disk as before); `build.rs` emits a
//! warning in that case.
//!
//! Hard prerequisite for an actual bundle: `./devops/build-engine-wasm.sh`
//! (owned by #11), then: `cargo build -p sandbox --features bundled-engine`.

/// Embedded engine.wasm bytes. Only present with `bundled-engine` and the
/// asset on disk at compile time (see build.rs).
#[cfg(all(feature = "bundled-engine", mossymesh_engine_wasm_bundled))]
pub const ENGINE_WASM_BYTES: &[u8] = include_bytes!("../assets/engine.wasm");

/// Empty stand-in so `sandbox::ENGINE_WASM_BYTES` still resolves when the
/// feature is on but the asset was absent at compile time (see build.rs).
/// Prefer [`engine_wasm_bytes`], which reports absence as `None`.
#[cfg(all(feature = "bundled-engine", not(mossymesh_engine_wasm_bundled)))]
pub const ENGINE_WASM_BYTES: &[u8] = &[];

/// Engine WASM bytes for the sandbox loader, or `None` when no bundle is
/// embedded (feature off, or asset missing at compile time).
///
/// Consumer (issue #171): [`crate::host::HostRuntime::load_engine`] calls
/// this first and feeds the bytes to the runtime, falling back to reading
/// `sandbox/assets/engine.wasm` from disk when this returns `None`.
pub fn engine_wasm_bytes() -> Option<&'static [u8]> {
    #[cfg(all(feature = "bundled-engine", mossymesh_engine_wasm_bundled))]
    {
        Some(ENGINE_WASM_BYTES)
    }
    #[cfg(not(all(feature = "bundled-engine", mossymesh_engine_wasm_bundled)))]
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
        #[cfg(not(all(feature = "bundled-engine", mossymesh_engine_wasm_bundled)))]
        assert!(engine_wasm_bytes().is_none());
        #[cfg(all(feature = "bundled-engine", mossymesh_engine_wasm_bundled))]
        {
            let bytes = engine_wasm_bytes().expect("bundled engine.wasm");
            assert!(bytes.starts_with(b"\0asm"), "must be a WASM module");
        }
    }
}
