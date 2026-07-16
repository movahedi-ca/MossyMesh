# Cross-compile notes (ARM / ESP32)

MessyMash targets heterogeneous edge hardware: Raspberry Pi genesis nodes,
phones, and ESP32 + LoRa (SX1262) leaf radios. This document covers practical
Rust / C toolchain setup for those targets.

## 1. Host prerequisites

```bash
# Debian / Ubuntu / Raspberry Pi OS
sudo apt update
sudo apt install -y build-essential pkg-config git curl

# Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"
```

## 2. Raspberry Pi (aarch64 / armv7)

### Native build on the Pi

Often simplest for genesis nodes:

```bash
cd MossyMesh
cargo build -p mesh-transport --release
```

### Cross from x86_64 Linux (aarch64 Pi 3/4/5 / Zero 2 W 64-bit)

```bash
rustup target add aarch64-unknown-linux-gnu

# Linker (Debian/Ubuntu)
sudo apt install -y gcc-aarch64-linux-gnu

mkdir -p .cargo
cat >> .cargo/config.toml <<'EOF'
[target.aarch64-unknown-linux-gnu]
linker = "aarch64-linux-gnu-gcc"
EOF

cargo build -p mesh-transport --release --target aarch64-unknown-linux-gnu
# artifact: target/aarch64-unknown-linux-gnu/release/mesh-daemon
```

### 32-bit armv7 (older Pi OS)

```bash
rustup target add armv7-unknown-linux-gnueabihf
sudo apt install -y gcc-arm-linux-gnueabihf

# .cargo/config.toml
# [target.armv7-unknown-linux-gnueabihf]
# linker = "arm-linux-gnueabihf-gcc"

cargo build -p mesh-transport --release --target armv7-unknown-linux-gnueabihf
```

### Optional: `cross` containerized builds

```bash
cargo install cross --git https://github.com/cross-rs/cross
cross build -p mesh-transport --release --target aarch64-unknown-linux-gnu
```

Useful when host libraries (OpenSSL, etc.) would otherwise break linking.

## 3. ESP32 (Xtensa / RISC-V)

ESP32 work in this project is primarily **transport leaf** (LoRa MAC, BLE
mesh stubs). Prefer Espressif’s official tooling rather than inventing a
custom linker story.

### ESP-IDF + esp-rs overview

| MCU family | Rust target (typical) | Notes |
| --- | --- | --- |
| ESP32 (Xtensa) | `xtensa-esp32-none-elf` | Needs espup / Espressif clang |
| ESP32-C3 (RISC-V) | `riscv32imc-unknown-none-elf` | Cleaner rustc story |
| ESP32-S3 | `xtensa-esp32s3-none-elf` | Dual-core + more RAM |

Install toolchain (developer laptop):

```bash
# espup installs Xtensa rustc + llvm tools
cargo install espup
espup install
# then: source ~/export-esp.sh   (path printed by espup)

# For no_std / embedded HAL crates
rustup target add riscv32imc-unknown-none-elf
```

### Firmware layout guidance

- Keep ESP32 firmware **out of** the workspace `cargo check` critical path
  until a dedicated `esp32-*` crate exists; CI is best-effort for host crates.
- Flash size and **10 MB RAM edge budget** apply to Pi/phone ledger nodes —
  ESP32 leaves should only hold routing / LoRa framing, not full trie ledgers.
- Pair SX1262 via SPI; keep CSMA/CA timing in the leaf firmware, not the Pi.

### Build / flash (example pattern)

```bash
# Once an esp-idf or embassy-based crate lands under e.g. firmware/esp32-lora/
# cd firmware/esp32-lora
# cargo build --release --target riscv32imc-unknown-none-elf
# espflash flash target/riscv32imc-unknown-none-elf/release/<bin>
```

## 4. WASM engine target (`wasm32-wasip1`)

Chess engine / sandbox workstreams compile to WASI for deterministic replay.

**Full notes (build scripts, optional cargo config, Mnps bench how-to):**
[`engine-wasm.md`](engine-wasm.md)

```bash
rustup target add wasm32-wasip1
cargo build -p engine --release --target wasm32-wasip1
# or: ./devops/build-engine-wasm.sh
```

Do **not** change the workspace default Cargo target — host `cargo test -p engine`
must remain native. Bounded stack / fixed memory pools are enforced in the WAMR
sandbox workstream (`-z stack-size=N`, fixed-block pools) — do not raise RAM
without SLA review.

## 5. Docker on Pi

Captive portal image is multi-arch friendly via buildx:

```bash
docker buildx create --use || true
docker buildx build \
  --platform linux/arm64 \
  -f captive-portal/Dockerfile \
  -t messymash/captive-portal:pi \
  --load .
```

Or use `devops/deploy-pi.sh` on the device itself.

## 6. Constraints checklist

- [ ] Active ledger footprint ≤ **10 MB** RAM on edge nodes
- [ ] nginx **`client_max_body_size 150M`** for asset sneakernet
- [ ] No hard dependency on public DNS / cloud in runtime paths
- [ ] ESP32 leaves do not host full consensus state
