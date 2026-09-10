# ThornyChat

Windows first Matrix Client with lots of preview features and connectors to steam and other stuff to autosend messages about your activities.

This is basically tailormade for myself but maybe someone else has fun with this too

Kind Regards

Wölki

## Building

Windows-only workspace; everything targets `x86_64-pc-windows-msvc`.

Prerequisites:

- **Rust stable.** `rust-toolchain.toml` pins the channel and pulls in the
  `x86_64-pc-windows-msvc` target plus `clippy`, so rustup provisions it on the
  first build.
- **MSVC build tools** — Visual Studio (or the standalone Build Tools) with the
  "Desktop development with C++" workload. `libsqlite3-sys` compiles SQLite from
  source (matrix-sdk-sqlite's `bundled` feature) and `aws-lc-sys` needs it too.
- **CMake** — also for `aws-lc-sys`.

### Development

```
cargo build
cargo build --release
```

Plain, generic binary at `target/x86_64-pc-windows-msvc/release/thornychat.exe`.
Fine for day-to-day work and for a PR.

### Release

`cargo xtask` is the standard release build — prefer it over a bare
`cargo build --release`, which produces only the generic variant. It builds
three binaries, each in its own `target/` subdirectory so they don't overwrite
each other:

| Variant  | Path                                                  | What it is |
| -------- | ----------------------------------------------------- | ---------- |
| generic  | `target/x86_64-pc-windows-msvc/release/thornychat.exe` | Baseline x86-64: runs on any 64-bit CPU. The one to hand to someone else or ship as a download. |
| znver4   | `target/znver4/x86_64-pc-windows-msvc/release/thornychat.exe` | AVX-512 (incl. IFMA) + DDR5, on Zen 4's double-pumped 256-bit units; the dev machine's native CPU. |
| znver5   | `target/znver5/x86_64-pc-windows-msvc/release/thornychat.exe` | Zen 5: a native full-width 512-bit AVX-512 datapath plus wider dispatch — the newest generation. |

**Never ship a znverN (or `target-cpu=native`) binary to unknown hardware.** A CPU
without those instructions dies with an illegal-instruction fault. The generic
build is the one that's safe everywhere.

## License

ThornyChat is free software: you can redistribute it and/or modify it under the terms of the GNU General Public License as published by the Free Software Foundation, either version 3 of the License, or (at your option) any later version. See [LICENSE](LICENSE) for the full text.

Copyright © 2026 Dominik Wölki
