#!/usr/bin/env bash
# Build engine crate for wasm32-wasip1 without changing host defaults.
# Usage (workspace root):
#   ./devops/build-engine-wasm.sh           # release
#   ./devops/build-engine-wasm.sh --debug   # debug
#   ./devops/build-engine-wasm.sh --release --features ''  # extra cargo args after mode
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

PROFILE="release"
EXTRA=()

for arg in "$@"; do
  case "$arg" in
    --debug|-d) PROFILE="debug" ;;
    --release|-r) PROFILE="release" ;;
    *) EXTRA+=("$arg") ;;
  esac
done

if ! command -v rustup >/dev/null 2>&1; then
  echo "error: rustup not found on PATH" >&2
  exit 1
fi

rustup target add wasm32-wasip1

BUILD_ARGS=(build -p engine --target wasm32-wasip1)
if [[ "$PROFILE" == "release" ]]; then
  BUILD_ARGS+=(--release)
fi
BUILD_ARGS+=("${EXTRA[@]+"${EXTRA[@]}"}")

echo "+ cargo ${BUILD_ARGS[*]}"
cargo "${BUILD_ARGS[@]}"

echo
echo "OK: engine built for wasm32-wasip1 ($PROFILE)"
echo "    target/wasm32-wasip1/${PROFILE}/libengine.rlib"
