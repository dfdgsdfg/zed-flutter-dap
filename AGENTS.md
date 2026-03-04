# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build & Test Commands

This repo has **two separate crates** (not a workspace) targeting different platforms:

### WASM Extension (root crate)
```bash
cargo test                                      # 94 unit tests
cargo clippy --all-targets -- -D warnings       # lint
cargo fmt -- --check                            # format check
cargo build --target wasm32-wasip1              # build extension
```

### dap-proxy (native binary)
```bash
cd dap-proxy
cargo test                                      # 16 unit tests
cargo clippy --all-targets -- -D warnings
cargo fmt -- --check
cargo build --release                           # build for current platform
cargo build --release --target aarch64-apple-darwin  # cross-compile
```

Run a single test: `cargo test test_name` in the respective crate directory.

## Architecture

Zed extension for Dart/Flutter debugging with a DAP proxy for hot reload support.

**Two crates, two targets:**
- `zed-flutter-dap` (root) — WASM extension (`cdylib`, `wasm32-wasip1`). Handles config validation, SDK resolution, adapter selection. Single file: `src/lib.rs`.
- `dap-proxy/` — Native async Rust binary (tokio). Sits between Zed and the Flutter debug adapter, proxying all DAP traffic while listening on a Unix socket for hot reload/restart commands.

**Data flow for Flutter sessions:**
```
Zed ←stdin/stdout→ dap-proxy ←stdin/stdout→ flutter debug-adapter
                       ↕
                 Unix socket (/tmp/zed-dap-{pid}.sock)
                       ↕
                 flutter-reload.sh (sends hotReload/hotRestart)
```

Dart CLI sessions bypass the proxy entirely — no hot reload needed.

**Sequence number routing:** The proxy allocates injected request seq numbers starting at 100,000. Responses with `request_seq >= 100_000` are routed to the socket client; everything else passes through to Zed transparently.

**Proxy binary distribution:** The WASM extension downloads the native proxy binary from GitHub releases at runtime via `ensure_proxy_binary()`. Asset naming: `dap-proxy-{target}.tar.gz`. Release triggered by `dap-proxy-v*` tags.

## Key Design Decisions

- **Not a Cargo workspace** — WASM extension and native binary have incompatible targets.
- **Adapter names in extension.toml** are `Flutter` and `FlutterCLI` (for Dart CLI). The internal constants `ADAPTER_DART_CLI = "FlutterCLI"` and `ADAPTER_DART_FLUTTER = "Flutter"` must match extension.toml keys AND the `"adapter"` enum in the JSON schemas.
- **Config validation happens in the extension** before spawning the adapter — fail fast with actionable error messages.
- **Test mode** is auto-detected from file path (`_test.dart`, `test/`, `integration_test/`) but can be overridden with `testMode: true`. Test mode + attach is rejected.
- **SDK resolution priority:** config override (`flutterSdkPath`/`dartSdkPath`) > `worktree.which()` PATH lookup.
- **`ensure_proxy_binary()` returns an absolute path** — Zed spawns processes from the project's cwd, not the extension working directory.

## Extension Config Flow (src/lib.rs)

`get_dap_binary()` → `classify_target()` → `validate_config()` → `resolve_*_binary()` → `build_[proxied_]debug_adapter_binary()`

The `TargetKind` enum (6 variants: Dart/Flutter × Launch/Attach/TestLaunch) is the central classification that drives all downstream logic.

## dap-proxy Internals

Four concurrent tokio tasks in `main.rs`:
1. **zed_to_adapter** — reads DAP from stdin, forwards via mpsc channel
2. **adapter_to_zed** — reads DAP from child stdout, routes responses by seq number
3. **stdin_writer** — drains mpsc channel to child stdin (serializes writes)
4. **socket listener** — accepts JSON commands on Unix socket, injects DAP requests, waits for response via oneshot channel

Signal forwarding (SIGTERM/SIGINT → child) and socket cleanup on exit.
