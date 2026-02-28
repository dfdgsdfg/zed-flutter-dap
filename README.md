# Dart & Flutter Debugger for Zed

Debug Adapter Protocol (DAP) extension for [Zed](https://zed.dev) that supports Dart CLI and Flutter debugging using official SDK debug adapters.

## Features

- **Dart CLI** — launch, attach, and test debugging via `dart debug_adapter`
- **Flutter** — launch, attach, and test debugging via `flutter debug-adapter`
- **Hot Reload & Hot Restart** — for Flutter sessions via the included DAP proxy

## Setup

Install the extension from Zed's extension marketplace, or as a dev extension:

1. Clone this repository
2. In Zed: Extensions → Install Dev Extension → select the cloned directory

### Supported Platforms

| Platform | Architecture | DAP Proxy | WASM Extension |
|----------|-------------|-----------|----------------|
| macOS | aarch64 (Apple Silicon) | Yes | Yes |
| macOS | x86_64 (Intel) | Yes | Yes |
| Linux | x86_64 | Yes | Yes |
| Linux | aarch64 | Yes | Yes |

The WASM extension runs on any platform that Zed supports. The DAP proxy (for hot reload) is a native binary built for the platforms above.

### Requirements

- [Dart SDK](https://dart.dev/get-dart) or [Flutter SDK](https://docs.flutter.dev/get-started/install) on your `PATH`

## Debug Configurations

Create `.zed/debug.json` in your project:

### Flutter Launch

```json
[
  {
    "label": "my app: dev",
    "adapter": "Flutter",
    "request": "launch",
    "program": "lib/main.dart",
    "cwd": "$ZED_WORKTREE_ROOT",
    "args": ["--flavor", "dev"]
  }
]
```

### Flutter Attach

```json
[
  {
    "label": "attach to running app",
    "adapter": "Flutter",
    "request": "attach",
    "vmServiceUri": "ws://127.0.0.1:8181/XXXXX=/ws"
  }
]
```

### Dart CLI Launch

```json
[
  {
    "label": "run server",
    "adapter": "FlutterCLI",
    "request": "launch",
    "program": "bin/server.dart"
  }
]
```

### Configuration Options

| Field | Type | Description |
|-------|------|-------------|
| `adapter` | `"Flutter"` or `"FlutterCLI"` | Debug adapter to use |
| `request` | `"launch"` or `"attach"` | Launch new process or attach to existing |
| `program` | string | Dart entrypoint file (required for launch) |
| `cwd` | string | Working directory (default: `$ZED_WORKTREE_ROOT`) |
| `args` | string[] | Additional arguments |
| `env` | object | Environment variables |
| `vmServiceUri` | string | VM service URI (required for attach) |
| `testMode` | boolean | Use test debug adapter (`--test` flag) |
| `stopOnEntry` | boolean | Pause at program entry point |
| `flutterSdkPath` | string | Explicit path to `flutter` binary |
| `dartSdkPath` | string | Explicit path to `dart` binary (FlutterCLI only) |

## DAP Proxy Commands

Flutter debug sessions automatically run through a DAP proxy that exposes a Unix socket. The proxy accepts any Flutter DAP command and also provides built-in meta-commands.

### Available Commands

| Command | Shorthand | Description |
|---------|-----------|-------------|
| `hotReload` | `reload` | Perform a hot reload |
| `hotRestart` | `restart` | Perform a hot restart |
| `devtools` | — | Get a Flutter DevTools URL (opens in browser via script) |
| `status` | — | Show the captured VM service URI |
| `callService` | — | Call a VM service extension (pass `arguments`) |
| `updateDebugOptions` | — | Update debug options at runtime (pass `arguments`) |
| *any DAP command* | — | Forward any custom request to the debug adapter |

**Meta-commands** (`status`, `devtools`) are handled by the proxy itself and are not forwarded to the debug adapter.

All other commands are injected as DAP requests to the Flutter debug adapter. You can pass additional arguments:

```bash
# Hot reload (shorthand)
echo '{"command": "hotReload"}' | nc -U /tmp/zed-dap-*.sock

# Call a VM service extension with arguments
echo '{"command": "callService", "arguments": {"method": "ext.flutter.inspector.show"}}' | nc -U /tmp/zed-dap-*.sock

# Get DevTools URL
echo '{"command": "devtools"}' | nc -U /tmp/zed-dap-*.sock
# → {"devtoolsUrl": "https://devtools.flutter.dev/#/?uri=...", "vmServiceUri": "ws://..."}
```

### Setup Tasks

Copy `scripts/flutter-reload.sh` to `.zed/flutter-reload.sh` in your project and make it executable:

```bash
cp scripts/flutter-reload.sh /path/to/your/project/.zed/flutter-reload.sh
chmod +x /path/to/your/project/.zed/flutter-reload.sh
```

Add to `.zed/tasks.json`:

```json
[
  {
    "label": "flutter: hot reload",
    "command": "$ZED_WORKTREE_ROOT/.zed/flutter-reload.sh",
    "args": ["reload"],
    "use_new_terminal": false,
    "allow_concurrent_runs": false,
    "reveal": "always"
  },
  {
    "label": "flutter: hot restart",
    "command": "$ZED_WORKTREE_ROOT/.zed/flutter-reload.sh",
    "args": ["restart"],
    "use_new_terminal": false,
    "allow_concurrent_runs": false,
    "reveal": "always"
  },
  {
    "label": "flutter: devtools",
    "command": "$ZED_WORKTREE_ROOT/.zed/flutter-reload.sh",
    "args": ["devtools"],
    "use_new_terminal": false,
    "allow_concurrent_runs": false,
    "reveal": "always"
  }
]
```

The script maps shorthands (`reload` → `hotReload`, `restart` → `hotRestart`) and for `devtools` automatically opens the URL in your browser.

### How It Works

```
Zed ←→ stdin/stdout ←→ dap-proxy ←→ stdin/stdout ←→ flutter debug-adapter
                            ↕
                     Unix socket (/tmp/zed-dap-{pid}.sock)
                            ↕
                     flutter-reload.sh / nc -U
```

The proxy sits between Zed and the Flutter debug adapter, passing all DAP traffic through transparently. It also listens on a Unix socket for commands, which it injects as DAP requests to the adapter. The proxy intercepts `dart.debuggerUris` events to capture the VM service URI for `status` and `devtools` commands.

Dart CLI sessions bypass the proxy entirely (hot reload is not applicable).

## Architecture

```
zed-flutter-dap/
├── src/lib.rs                    # WASM extension (adapter resolution, config validation)
├── dap-proxy/                    # Native Rust binary (separate crate)
│   ├── Cargo.toml
│   └── src/
│       ├── main.rs               # Entry point, child process orchestration
│       ├── dap.rs                # DAP Content-Length message framing
│       ├── proxy.rs              # Bidirectional proxy + response routing
│       ├── socket.rs             # Unix socket listener for reload commands
│       └── seq.rs                # Sequence number allocator (starts at 100,000)
├── scripts/
│   └── flutter-reload.sh         # Shell script to trigger reload via socket
├── debug_adapter_schemas/
│   ├── Flutter.json              # JSON schema for Flutter debug configs
│   └── FlutterCLI.json           # JSON schema for Dart CLI debug configs
└── extension.toml                # Zed extension manifest
```

The WASM extension and the native proxy binary are separate crates (not a Cargo workspace) because they target different platforms (wasm32-wasip1 vs native).

## Development

```bash
# Run extension tests
cargo test

# Build WASM extension
cargo build --target wasm32-wasip1

# Run proxy tests
cd dap-proxy && cargo test

# Build proxy binary
cd dap-proxy && cargo build --release
```

## License

MIT
