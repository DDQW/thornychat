<#
Builds the three release binaries, each in its own target/ subdirectory so
they don't overwrite each other:

  target/x86_64-pc-windows-msvc/release/thornychat.exe   (generic)
  target/znver4/x86_64-pc-windows-msvc/release/thornychat.exe
  target/znver5/x86_64-pc-windows-msvc/release/thornychat.exe

Why these three (see README.md "Release builds" for the fuller rationale):
  generic - baseline x86-64: runs on any 64-bit CPU, the variant to hand to
            someone else or ship as a download.
  znver4  - AVX-512 (incl. IFMA) + DDR5, on Zen 4's double-pumped 256-bit
            units; this dev machine's native CPU.
  znver5  - Zen 5: a native full-width 512-bit AVX-512 datapath plus wider
            dispatch - the newest generation.

Never ship a znverN (or target-cpu=native) binary to unknown hardware - a
CPU without those instructions dies with an illegal-instruction fault. The
generic build is the one that's safe everywhere.

This is the standard release-build approach for ThornyChat - prefer this
script over a bare `cargo build --release` (which produces only the generic
variant).
#>

$ErrorActionPreference = "Stop"

$running = Get-Process thornychat -ErrorAction SilentlyContinue
if ($running) {
    Write-Error "thornychat.exe is running (PID $($running.Id)) - close it first, the linker can't overwrite a running exe."
    exit 1
}

# Generic first: no target-cpu flag, default target dir - identical to what a
# plain `cargo build --release` produces.
Write-Host "=== Building generic (baseline x86-64) ===" -ForegroundColor Cyan
Remove-Item Env:\RUSTFLAGS -ErrorAction SilentlyContinue
Remove-Item Env:\CARGO_TARGET_DIR -ErrorAction SilentlyContinue
cargo build --release
if ($LASTEXITCODE -ne 0) {
    Write-Error "generic build failed"
    exit 1
}

$variants = "znver4", "znver5"

foreach ($v in $variants) {
    Write-Host "=== Building $v ===" -ForegroundColor Cyan
    $env:RUSTFLAGS = "-C target-cpu=$v"
    $env:CARGO_TARGET_DIR = "target/$v"
    cargo build --release
    if ($LASTEXITCODE -ne 0) {
        Write-Error "$v build failed"
        exit 1
    }
}

Remove-Item Env:\RUSTFLAGS -ErrorAction SilentlyContinue
Remove-Item Env:\CARGO_TARGET_DIR -ErrorAction SilentlyContinue

Write-Host ""
Write-Host "Built binaries:" -ForegroundColor Green
Write-Host "  target/x86_64-pc-windows-msvc/release/thornychat.exe   (generic)"
foreach ($v in $variants) {
    Write-Host "  target/$v/x86_64-pc-windows-msvc/release/thornychat.exe"
}
