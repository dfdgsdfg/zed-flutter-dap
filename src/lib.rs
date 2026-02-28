use zed_extension_api::{
    self as zed, DebugAdapterBinary, DebugConfig, DebugRequest, DebugScenario,
    DebugTaskDefinition, StartDebuggingRequestArguments,
    StartDebuggingRequestArgumentsRequest, Worktree,
};

/// Adapter name for Dart CLI debugging (launch, attach, test).
const ADAPTER_DART_CLI: &str = "DartCLI";

/// Adapter name for Flutter debugging (launch, attach, test).
const ADAPTER_DART_FLUTTER: &str = "DartFlutter";

struct DartDapExtension;

impl zed::Extension for DartDapExtension {
    fn new() -> Self {
        DartDapExtension
    }

    fn get_dap_binary(
        &mut self,
        adapter_name: String,
        config: DebugTaskDefinition,
        _user_installed_path: Option<String>,
        worktree: &Worktree,
    ) -> Result<DebugAdapterBinary, String> {
        let config_value: serde_json::Value = serde_json::from_str(&config.config)
            .map_err(|e| format!("Failed to parse debug config JSON: {e}"))?;

        let request_kind = resolve_request_kind(&config_value)?;
        let test_mode = config_value
            .get("testMode")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        match adapter_name.as_str() {
            ADAPTER_DART_CLI => {
                let dart_path = resolve_dart_binary(&config_value, worktree)?;
                let mut arguments = vec!["debug_adapter".to_string()];
                if test_mode {
                    arguments.push("--test".to_string());
                }

                Ok(DebugAdapterBinary {
                    command: Some(dart_path),
                    arguments,
                    envs: collect_env(&config_value),
                    cwd: config_value
                        .get("cwd")
                        .and_then(|v| v.as_str())
                        .map(String::from),
                    connection: None,
                    request_args: StartDebuggingRequestArguments {
                        configuration: config.config,
                        request: request_kind,
                    },
                })
            }
            ADAPTER_DART_FLUTTER => {
                let flutter_path = resolve_flutter_binary(&config_value, worktree)?;
                let mut arguments = vec!["debug-adapter".to_string()];
                if test_mode {
                    arguments.push("--test".to_string());
                }

                Ok(DebugAdapterBinary {
                    command: Some(flutter_path),
                    arguments,
                    envs: collect_env(&config_value),
                    cwd: config_value
                        .get("cwd")
                        .and_then(|v| v.as_str())
                        .map(String::from),
                    connection: None,
                    request_args: StartDebuggingRequestArguments {
                        configuration: config.config,
                        request: request_kind,
                    },
                })
            }
            _ => Err(format!("Unknown debug adapter: {adapter_name}")),
        }
    }

    fn dap_request_kind(
        &mut self,
        _adapter_name: String,
        config: serde_json::Value,
    ) -> Result<StartDebuggingRequestArgumentsRequest, String> {
        resolve_request_kind(&config)
    }

    fn dap_config_to_scenario(
        &mut self,
        config: DebugConfig,
    ) -> Result<DebugScenario, String> {
        let (adapter, scenario_config) = match &config.request {
            DebugRequest::Launch(launch) => {
                let is_flutter = config.adapter == ADAPTER_DART_FLUTTER;
                let adapter = if is_flutter {
                    ADAPTER_DART_FLUTTER
                } else {
                    ADAPTER_DART_CLI
                };

                let cfg = serde_json::json!({
                    "request": "launch",
                    "program": launch.program,
                    "args": launch.args,
                    "cwd": launch.cwd,
                    "env": launch.envs.iter()
                        .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
                        .collect::<serde_json::Map<String, serde_json::Value>>(),
                    "stopOnEntry": config.stop_on_entry.unwrap_or(false),
                });

                (adapter.to_string(), cfg)
            }
            DebugRequest::Attach(_attach) => {
                let adapter = if config.adapter == ADAPTER_DART_FLUTTER {
                    ADAPTER_DART_FLUTTER
                } else {
                    ADAPTER_DART_CLI
                };

                let cfg = serde_json::json!({
                    "request": "attach",
                });

                (adapter.to_string(), cfg)
            }
        };

        Ok(DebugScenario {
            label: config.label,
            adapter,
            build: None,
            config: scenario_config.to_string(),
            tcp_connection: None,
        })
    }
}

/// Determine launch vs attach from the raw config JSON.
fn resolve_request_kind(
    config: &serde_json::Value,
) -> Result<StartDebuggingRequestArgumentsRequest, String> {
    match config.get("request").and_then(|v| v.as_str()) {
        Some("launch") => Ok(StartDebuggingRequestArgumentsRequest::Launch),
        Some("attach") => Ok(StartDebuggingRequestArgumentsRequest::Attach),
        Some(other) => Err(format!(
            "Invalid 'request' value: '{other}'. Expected 'launch' or 'attach'."
        )),
        None => Err("Missing required 'request' field in debug configuration.".to_string()),
    }
}

/// Resolve the `dart` binary path from config override or worktree PATH.
fn resolve_dart_binary(
    config: &serde_json::Value,
    worktree: &Worktree,
) -> Result<String, String> {
    if let Some(path) = config.get("dartSdkPath").and_then(|v| v.as_str()) {
        if !path.is_empty() {
            return Ok(path.to_string());
        }
    }
    worktree
        .which("dart")
        .ok_or_else(|| "Could not find 'dart' on PATH. Ensure the Dart SDK is installed and available in your shell environment.".to_string())
}

/// Resolve the `flutter` binary path from config override or worktree PATH.
fn resolve_flutter_binary(
    config: &serde_json::Value,
    worktree: &Worktree,
) -> Result<String, String> {
    if let Some(path) = config.get("flutterSdkPath").and_then(|v| v.as_str()) {
        if !path.is_empty() {
            return Ok(path.to_string());
        }
    }
    worktree
        .which("flutter")
        .ok_or_else(|| "Could not find 'flutter' on PATH. Ensure the Flutter SDK is installed and available in your shell environment.".to_string())
}

/// Collect environment variables from the config's "env" object.
fn collect_env(config: &serde_json::Value) -> Vec<(String, String)> {
    config
        .get("env")
        .and_then(|v| v.as_object())
        .map(|obj| {
            obj.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect()
        })
        .unwrap_or_default()
}

zed::register_extension!(DartDapExtension);
