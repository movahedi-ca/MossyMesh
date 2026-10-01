# Build engine crate for wasm32-wasip1 without changing host defaults.
# Usage (workspace root):
#   .\devops\build-engine-wasm.ps1
#   .\devops\build-engine-wasm.ps1 -Debug
#   .\devops\build-engine-wasm.ps1 -CargoArgs @('--verbose')
param(
    [switch]$Debug,
    [string[]]$CargoArgs = @()
)

$ErrorActionPreference = "Stop"
$Root = Resolve-Path (Join-Path $PSScriptRoot "..")
Set-Location $Root

$cargoBin = Join-Path $env:USERPROFILE ".cargo\bin"
if (Test-Path $cargoBin) {
    $env:PATH = "$cargoBin;$env:PATH"
}

if (-not (Get-Command rustup -ErrorAction SilentlyContinue)) {
    Write-Error "rustup not found on PATH"
}
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    Write-Error "cargo not found on PATH"
}

rustup target add wasm32-wasip1 | Out-Host

$buildArgs = @("build", "-p", "engine", "--target", "wasm32-wasip1")
if (-not $Debug) {
    $buildArgs += "--release"
}
if ($CargoArgs.Count -gt 0) {
    $buildArgs += $CargoArgs
}

Write-Host "+ cargo $($buildArgs -join ' ')"
& cargo @buildArgs
if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}

$profile = if ($Debug) { "debug" } else { "release" }

$wasmOut = "target/wasm32-wasip1/$profile/engine.wasm"
if (-not (Test-Path $wasmOut)) {
    Write-Host "error: expected artifact $wasmOut was not produced." -ForegroundColor Red
    Write-Host "hint: engine must build a cdylib for wasm32-wasip1 (see engine/Cargo.toml)."
    exit 1
}
New-Item -ItemType Directory -Force -Path sandbox/assets | Out-Null
Copy-Item $wasmOut sandbox/assets/engine.wasm -Force

Write-Host ""
Write-Host "OK: engine built for wasm32-wasip1 ($profile)"
Write-Host "    target/wasm32-wasip1/$profile/libengine.rlib"
Write-Host "    $wasmOut -> sandbox/assets/engine.wasm"
