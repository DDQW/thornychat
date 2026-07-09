<#
Builds two release binaries, each tuned for a recent AMD Zen generation and
placed in its own target/ subdirectory so they don't overwrite each other:

  target/znver4/x86_64-pc-windows-msvc/release/thornychat.exe
  target/znver5/x86_64-pc-windows-msvc/release/thornychat.exe

Why these two (see README.md "Release builds" for the fuller rationale):
  znver4 - AVX-512 (incl. IFMA) + DDR5, on Zen 4's double-pumped 256-bit
           units; this dev machine's native CPU.
  znver5 - Zen 5: a native full-width 512-bit AVX-512 datapath plus wider
           dispatch - the newest generation.

This is the standard release-build approach for ThornyChat - prefer this
script over a bare `cargo build --release`.
#>

$ErrorActionPreference = "Stop"

$running = Get-Process thornychat -ErrorAction SilentlyContinue
if ($running) {
    Write-Error "thornychat.exe is running (PID $($running.Id)) - close it first, the linker can't overwrite a running exe."
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
foreach ($v in $variants) {
    Write-Host "  target/$v/x86_64-pc-windows-msvc/release/thornychat.exe"
}
