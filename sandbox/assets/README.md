# sandbox/assets/

This directory holds the compiled WebAssembly engine the sandbox loads at
runtime. The artifact is generated, never committed.

## Producing it

From the repo root:

```sh
./devops/build-engine-wasm.sh           # release -> sandbox/assets/engine.wasm
./devops/build-engine-wasm.sh --debug   # debug variant
```

The script builds the `engine` crate for `wasm32-wasip1` (as a cdylib, so
cargo emits `engine.wasm`) and copies the result here as `engine.wasm`.
CI rebuilds it on every run in the `engine-wasm-artifact` job, so a fresh
checkout always gets a current artifact before the daemon starts.

`engine.wasm` is gitignored (see `.gitignore` in this directory). Do not
commit build outputs here.
