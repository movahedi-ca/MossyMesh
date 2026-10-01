//! Build script for the `bundled-engine` feature (issue #146).
//!
//! `sandbox/src/engine_bundle.rs` embeds `assets/engine.wasm` with
//! `include_bytes!`, a compile-time macro on a gitignored asset that is
//! absent on a fresh checkout. This script probes for the asset and sets
//! the `mossymesh_engine_wasm_bundled` cfg only when it exists, so
//! `cargo build -p sandbox --features bundled-engine` degrades gracefully
//! (the loader returns `None` and callers read from disk as before) instead
//! of failing to compile. To actually bundle the bytes, generate the asset
//! first: `./devops/build-engine-wasm.sh`.

fn main() {
    println!("cargo:rerun-if-changed=assets/engine.wasm");
    println!("cargo:rustc-check-cfg=cfg(mossymesh_engine_wasm_bundled)");
    if std::path::Path::new("assets/engine.wasm").is_file() {
        println!("cargo:rustc-cfg=mossymesh_engine_wasm_bundled");
    } else if std::env::var("CARGO_FEATURE_BUNDLED_ENGINE").is_ok() {
        println!(
            "cargo:warning=bundled-engine is on but assets/engine.wasm is missing; \
             run ./devops/build-engine-wasm.sh to bundle it, \
             the loader falls back to disk reads until then"
        );
    }
}
