# Contributing to ThornyChat

Contributions are welcome — bug reports, fixes, features, docs. This file is short on ceremony and long on the one thing we actually care about: that whoever submits a change understands it.

## AI-assisted contributions

AI tools are explicitly welcome here. This project itself is developed with heavy AI assistance, so there will be no pearl-clutching about how a patch came to exist. Use whatever makes you productive.

What matters is that **you are the author**. Submitting a change means you vouch for it:

- **You know what it does.** You can explain every hunk of your diff — why it's there, what it changes, and what would break without it. "The model wrote it that way" is not an explanation.
- **You read it.** Generated code gets the same skeptical read you'd give a stranger's PR before putting your name on it.
- **You ran it.** Build it, run the tests, and actually exercise the behavior you touched in the running app. Code that has only ever been *reasoned about* is not tested code.

A PR is reviewed as if you typed every character by hand — same bar, no discounts. If review questions get answered by pasting them back into a chatbot and relaying whatever comes out, that becomes obvious very quickly and is a waste of everyone's time.

**The hard line** (this is the policy [AGENTS.md](AGENTS.md) refers to): AI agents do not speak *as you* in this repository. No agent-opened pull requests, no generated issue or review comments posted on your behalf. Tools write code; humans do the talking.

## Before you open a PR

From the repository root, on Windows with a stable MSVC Rust toolchain:

```
cargo test --workspace
cargo clippy --workspace --all-targets
```

Both must come back clean. Two local quirks worth knowing:

- Clippy's MIR-based lints replay stale results on a warm cache — if you touched something subtle, a `cargo clean` before the final clippy run is the honest check.
- The three-variant release build is `cargo xtask` (see the README's Building section); you don't need it for a PR, `cargo build` + tests is fine.

## What fits this project

- **Windows-first, native-first.** The client is one self-contained iced/Rust executable. Features that would drag in a web view (beyond the existing inline-video player) or a background service are a hard sell.
- **Simple and predictable beats clever.** Behavior should be user-controlled and boringly consistent; heuristics that guess at intent tend to get ripped out here. If a behavior could surprise someone, gate it behind a setting — and ship it off by default.
- **Privacy is a default, not a toggle buried in docs.** Anything that shares presence or activity (receipts, typing, connectors) starts disabled and says clearly what it will share.
- **Match the code around you.** Comments explain constraints and *why*, not what the next line does. Keep the module layout: `client-core` never touches iced, `ui` never touches `matrix_sdk` types.

Check the README's "Not there yet" list and `ROADMAP.md` before starting something big, and for anything substantial, open an issue first so nobody builds the same thing twice.

## Commit messages

Describe the change and the reason — what a reader of `git log` needs a year from now. Tooling credits (human or machine) don't belong in them.

## License

ThornyChat is GPL-3.0-or-later. By contributing, you agree that your contribution is licensed under the same terms. Keep the developer credit and copyright notices intact.
