use zed_extension_api::{
    self as zed, DebugAdapterBinary, DebugConfig, DebugRequest, DebugScenario,
    DebugTaskDefinition, StartDebuggingRequestArguments,
    StartDebuggingRequestArgumentsRequest, Worktree,
};

/// Adapter name for Dart CLI debugging (launch, attach, test).
const ADAPTER_DART_CLI: &str = "DartCLI";

/// Adapter name for Flutter debugging (launch, attach, test).
const ADAPTER_DART_FLUTTER: &str = "DartFlutter";

/// Normalized target classification combining adapter family, request kind, and test mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TargetKind {
    DartLaunch,
    DartAttach,
    DartTestLaunch,
    FlutterLaunch,
    FlutterAttach,
    FlutterTestLaunch,
}

impl TargetKind {
    /// The adapter binary subcommand (e.g. `debug_adapter` or `debug-adapter`).
    fn adapter_subcommand(&self) -> &'static str {
        match self {
            TargetKind::DartLaunch | TargetKind::DartAttach | TargetKind::DartTestLaunch => {
                "debug_adapter"
            }
            TargetKind::FlutterLaunch | TargetKind::FlutterAttach | TargetKind::FlutterTestLaunch => {
                "debug-adapter"
            }
        }
    }

    /// Whether the `--test` flag should be appended.
    fn is_test(&self) -> bool {
        matches!(self, TargetKind::DartTestLaunch | TargetKind::FlutterTestLaunch)
    }

    /// The request kind for DAP initialization.
    fn request_kind(&self) -> StartDebuggingRequestArgumentsRequest {
        match self {
            TargetKind::DartLaunch | TargetKind::DartTestLaunch | TargetKind::FlutterLaunch | TargetKind::FlutterTestLaunch => {
                StartDebuggingRequestArgumentsRequest::Launch
            }
            TargetKind::DartAttach | TargetKind::FlutterAttach => {
                StartDebuggingRequestArgumentsRequest::Attach
            }
        }
    }
}

/// Classify a debug configuration into a normalized target kind.
fn classify_target(
    adapter_name: &str,
    config: &serde_json::Value,
) -> Result<TargetKind, String> {
    let request = config
        .get("request")
        .and_then(|v| v.as_str())
        .ok_or("Missing required 'request' field in debug configuration.")?;

    let test_mode = config
        .get("testMode")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    match (adapter_name, request, test_mode) {
        (ADAPTER_DART_CLI, "launch", false) => Ok(TargetKind::DartLaunch),
        (ADAPTER_DART_CLI, "attach", false) => Ok(TargetKind::DartAttach),
        (ADAPTER_DART_CLI, "launch", true) => Ok(TargetKind::DartTestLaunch),
        (ADAPTER_DART_CLI, "attach", true) => Err(
            "Test mode is not supported with attach requests for DartCLI.".to_string(),
        ),
        (ADAPTER_DART_FLUTTER, "launch", false) => Ok(TargetKind::FlutterLaunch),
        (ADAPTER_DART_FLUTTER, "attach", false) => Ok(TargetKind::FlutterAttach),
        (ADAPTER_DART_FLUTTER, "launch", true) => Ok(TargetKind::FlutterTestLaunch),
        (ADAPTER_DART_FLUTTER, "attach", true) => Err(
            "Test mode is not supported with attach requests for DartFlutter.".to_string(),
        ),
        (_, request, _) if request != "launch" && request != "attach" => Err(format!(
            "Invalid 'request' value: '{request}'. Expected 'launch' or 'attach'."
        )),
        (adapter, _, _) => Err(format!("Unknown debug adapter: {adapter}")),
    }
}

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

        let target = classify_target(&adapter_name, &config_value)?;

        let command = match target {
            TargetKind::DartLaunch | TargetKind::DartAttach | TargetKind::DartTestLaunch => {
                resolve_dart_binary(&config_value, worktree)?
            }
            TargetKind::FlutterLaunch | TargetKind::FlutterAttach | TargetKind::FlutterTestLaunch => {
                resolve_flutter_binary(&config_value, worktree)?
            }
        };

        let mut arguments = vec![target.adapter_subcommand().to_string()];
        if target.is_test() {
            arguments.push("--test".to_string());
        }

        Ok(DebugAdapterBinary {
            command: Some(command),
            arguments,
            envs: collect_env(&config_value),
            cwd: config_value
                .get("cwd")
                .and_then(|v| v.as_str())
                .map(String::from),
            connection: None,
            request_args: StartDebuggingRequestArguments {
                configuration: config.config,
                request: target.request_kind(),
            },
        })
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
        let adapter = if config.adapter == ADAPTER_DART_FLUTTER {
            ADAPTER_DART_FLUTTER
        } else {
            ADAPTER_DART_CLI
        };

        let scenario_config = match &config.request {
            DebugRequest::Launch(launch) => {
                let test_mode = looks_like_test(&launch.program);

                serde_json::json!({
                    "request": "launch",
                    "program": launch.program,
                    "args": launch.args,
                    "cwd": launch.cwd,
                    "env": launch.envs.iter()
                        .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
                        .collect::<serde_json::Map<String, serde_json::Value>>(),
                    "testMode": test_mode,
                    "stopOnEntry": config.stop_on_entry.unwrap_or(false),
                })
            }
            DebugRequest::Attach(attach) => {
                let mut cfg = serde_json::json!({
                    "request": "attach",
                    "vmServiceUri": "",
                    "stopOnEntry": config.stop_on_entry.unwrap_or(false),
                });

                if let Some(pid) = attach.process_id {
                    cfg["processId"] = serde_json::Value::Number(pid.into());
                }

                cfg
            }
        };

        Ok(DebugScenario {
            label: config.label,
            adapter: adapter.to_string(),
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

/// Heuristic: does this program path look like a test file?
fn looks_like_test(program: &str) -> bool {
    program.ends_with("_test.dart")
        || program.starts_with("test/")
        || program.starts_with("test\\")
        || program.contains("/test/")
        || program.contains("\\test\\")
        || program.starts_with("integration_test/")
        || program.starts_with("integration_test\\")
        || program.contains("/integration_test/")
        || program.contains("\\integration_test\\")
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

#[cfg(test)]
mod tests {
    use super::*;
    use zed_extension_api::Extension;

    // --- resolve_request_kind tests ---

    #[test]
    fn resolve_request_kind_launch() {
        let config = serde_json::json!({"request": "launch"});
        assert_eq!(
            resolve_request_kind(&config).unwrap(),
            StartDebuggingRequestArgumentsRequest::Launch
        );
    }

    #[test]
    fn resolve_request_kind_attach() {
        let config = serde_json::json!({"request": "attach"});
        assert_eq!(
            resolve_request_kind(&config).unwrap(),
            StartDebuggingRequestArgumentsRequest::Attach
        );
    }

    #[test]
    fn resolve_request_kind_invalid_value() {
        let config = serde_json::json!({"request": "run"});
        let err = resolve_request_kind(&config).unwrap_err();
        assert!(err.contains("run"), "error should mention the invalid value");
    }

    #[test]
    fn resolve_request_kind_missing_field() {
        let config = serde_json::json!({"program": "main.dart"});
        let err = resolve_request_kind(&config).unwrap_err();
        assert!(err.contains("Missing"), "error should indicate missing field");
    }

    #[test]
    fn resolve_request_kind_null_value() {
        let config = serde_json::json!({"request": null});
        let err = resolve_request_kind(&config).unwrap_err();
        assert!(err.contains("Missing"));
    }

    #[test]
    fn resolve_request_kind_numeric_value() {
        let config = serde_json::json!({"request": 42});
        let err = resolve_request_kind(&config).unwrap_err();
        assert!(err.contains("Missing"));
    }

    // --- classify_target tests ---

    #[test]
    fn classify_dart_launch() {
        let config = serde_json::json!({"request": "launch"});
        assert_eq!(classify_target("DartCLI", &config).unwrap(), TargetKind::DartLaunch);
    }

    #[test]
    fn classify_dart_attach() {
        let config = serde_json::json!({"request": "attach"});
        assert_eq!(classify_target("DartCLI", &config).unwrap(), TargetKind::DartAttach);
    }

    #[test]
    fn classify_dart_test_launch() {
        let config = serde_json::json!({"request": "launch", "testMode": true});
        assert_eq!(classify_target("DartCLI", &config).unwrap(), TargetKind::DartTestLaunch);
    }

    #[test]
    fn classify_dart_test_false_is_normal_launch() {
        let config = serde_json::json!({"request": "launch", "testMode": false});
        assert_eq!(classify_target("DartCLI", &config).unwrap(), TargetKind::DartLaunch);
    }

    #[test]
    fn classify_dart_attach_test_mode_rejected() {
        let config = serde_json::json!({"request": "attach", "testMode": true});
        let err = classify_target("DartCLI", &config).unwrap_err();
        assert!(err.contains("Test mode is not supported"));
    }

    #[test]
    fn classify_flutter_launch() {
        let config = serde_json::json!({"request": "launch"});
        assert_eq!(classify_target("DartFlutter", &config).unwrap(), TargetKind::FlutterLaunch);
    }

    #[test]
    fn classify_flutter_attach() {
        let config = serde_json::json!({"request": "attach"});
        assert_eq!(classify_target("DartFlutter", &config).unwrap(), TargetKind::FlutterAttach);
    }

    #[test]
    fn classify_flutter_test_launch() {
        let config = serde_json::json!({"request": "launch", "testMode": true});
        assert_eq!(classify_target("DartFlutter", &config).unwrap(), TargetKind::FlutterTestLaunch);
    }

    #[test]
    fn classify_flutter_attach_test_mode_rejected() {
        let config = serde_json::json!({"request": "attach", "testMode": true});
        let err = classify_target("DartFlutter", &config).unwrap_err();
        assert!(err.contains("Test mode is not supported"));
    }

    #[test]
    fn classify_unknown_adapter() {
        let config = serde_json::json!({"request": "launch"});
        let err = classify_target("UnknownAdapter", &config).unwrap_err();
        assert!(err.contains("Unknown debug adapter"));
    }

    #[test]
    fn classify_invalid_request() {
        let config = serde_json::json!({"request": "debug"});
        let err = classify_target("DartCLI", &config).unwrap_err();
        assert!(err.contains("Invalid 'request' value"));
    }

    #[test]
    fn classify_missing_request() {
        let config = serde_json::json!({"program": "main.dart"});
        let err = classify_target("DartCLI", &config).unwrap_err();
        assert!(err.contains("Missing"));
    }

    // --- TargetKind method tests ---

    #[test]
    fn target_kind_subcommands() {
        assert_eq!(TargetKind::DartLaunch.adapter_subcommand(), "debug_adapter");
        assert_eq!(TargetKind::DartAttach.adapter_subcommand(), "debug_adapter");
        assert_eq!(TargetKind::DartTestLaunch.adapter_subcommand(), "debug_adapter");
        assert_eq!(TargetKind::FlutterLaunch.adapter_subcommand(), "debug-adapter");
        assert_eq!(TargetKind::FlutterAttach.adapter_subcommand(), "debug-adapter");
        assert_eq!(TargetKind::FlutterTestLaunch.adapter_subcommand(), "debug-adapter");
    }

    #[test]
    fn target_kind_is_test() {
        assert!(!TargetKind::DartLaunch.is_test());
        assert!(!TargetKind::DartAttach.is_test());
        assert!(TargetKind::DartTestLaunch.is_test());
        assert!(!TargetKind::FlutterLaunch.is_test());
        assert!(!TargetKind::FlutterAttach.is_test());
        assert!(TargetKind::FlutterTestLaunch.is_test());
    }

    #[test]
    fn target_kind_request_kinds() {
        assert_eq!(TargetKind::DartLaunch.request_kind(), StartDebuggingRequestArgumentsRequest::Launch);
        assert_eq!(TargetKind::DartAttach.request_kind(), StartDebuggingRequestArgumentsRequest::Attach);
        assert_eq!(TargetKind::DartTestLaunch.request_kind(), StartDebuggingRequestArgumentsRequest::Launch);
        assert_eq!(TargetKind::FlutterLaunch.request_kind(), StartDebuggingRequestArgumentsRequest::Launch);
        assert_eq!(TargetKind::FlutterAttach.request_kind(), StartDebuggingRequestArgumentsRequest::Attach);
        assert_eq!(TargetKind::FlutterTestLaunch.request_kind(), StartDebuggingRequestArgumentsRequest::Launch);
    }

    // --- collect_env tests ---

    #[test]
    fn collect_env_with_values() {
        let config = serde_json::json!({"env": {"FOO": "bar", "BAZ": "qux"}});
        let mut envs = collect_env(&config);
        envs.sort();
        assert_eq!(envs, vec![
            ("BAZ".to_string(), "qux".to_string()),
            ("FOO".to_string(), "bar".to_string()),
        ]);
    }

    #[test]
    fn collect_env_empty_object() {
        let config = serde_json::json!({"env": {}});
        assert!(collect_env(&config).is_empty());
    }

    #[test]
    fn collect_env_missing_field() {
        let config = serde_json::json!({"program": "main.dart"});
        assert!(collect_env(&config).is_empty());
    }

    #[test]
    fn collect_env_skips_non_string_values() {
        let config = serde_json::json!({"env": {"GOOD": "value", "BAD": 42, "ALSO_BAD": true}});
        let envs = collect_env(&config);
        assert_eq!(envs, vec![("GOOD".to_string(), "value".to_string())]);
    }

    #[test]
    fn collect_env_null_field() {
        let config = serde_json::json!({"env": null});
        assert!(collect_env(&config).is_empty());
    }

    // --- looks_like_test tests ---

    #[test]
    fn looks_like_test_suffix() {
        assert!(looks_like_test("widget_test.dart"));
        assert!(looks_like_test("my_app_test.dart"));
    }

    #[test]
    fn looks_like_test_in_test_dir() {
        assert!(looks_like_test("test/widget_test.dart"));
        assert!(looks_like_test("test/unit/parser_test.dart"));
    }

    #[test]
    fn looks_like_test_integration() {
        assert!(looks_like_test("integration_test/app_test.dart"));
        assert!(looks_like_test("src/integration_test/smoke_test.dart"));
    }

    #[test]
    fn looks_like_test_nested_test_dir() {
        assert!(looks_like_test("packages/core/test/utils_test.dart"));
    }

    #[test]
    fn looks_like_test_negative() {
        assert!(!looks_like_test("lib/main.dart"));
        assert!(!looks_like_test("bin/server.dart"));
        assert!(!looks_like_test("lib/testing_utils.dart"));
        assert!(!looks_like_test("lib/contest.dart"));
    }

    // --- dap_config_to_scenario tests ---

    /// Helper to build a DebugConfig with a launch request.
    fn make_launch_config(adapter: &str, program: &str) -> DebugConfig {
        DebugConfig {
            label: "Test".to_string(),
            adapter: adapter.to_string(),
            request: DebugRequest::Launch(zed::LaunchRequest {
                program: program.to_string(),
                cwd: Some("/workspace".to_string()),
                args: vec!["--verbose".to_string()],
                envs: vec![("DART_VM_OPTIONS".to_string(), "--enable-asserts".to_string())],
            }),
            stop_on_entry: Some(true),
        }
    }

    /// Helper to build a DebugConfig with an attach request.
    fn make_attach_config(adapter: &str, process_id: Option<u32>) -> DebugConfig {
        DebugConfig {
            label: "Attach".to_string(),
            adapter: adapter.to_string(),
            request: DebugRequest::Attach(zed::AttachRequest { process_id }),
            stop_on_entry: None,
        }
    }

    #[test]
    fn scenario_launch_dart_non_test() {
        let mut ext = DartDapExtension;
        let config = make_launch_config(ADAPTER_DART_CLI, "bin/main.dart");
        let scenario = ext.dap_config_to_scenario(config).unwrap();

        assert_eq!(scenario.adapter, ADAPTER_DART_CLI);
        let cfg: serde_json::Value = serde_json::from_str(&scenario.config).unwrap();
        assert_eq!(cfg["request"], "launch");
        assert_eq!(cfg["program"], "bin/main.dart");
        assert_eq!(cfg["testMode"], false);
        assert_eq!(cfg["stopOnEntry"], true);
        assert_eq!(cfg["cwd"], "/workspace");
        assert_eq!(cfg["args"], serde_json::json!(["--verbose"]));
        assert_eq!(cfg["env"]["DART_VM_OPTIONS"], "--enable-asserts");
    }

    #[test]
    fn scenario_launch_dart_test_file() {
        let mut ext = DartDapExtension;
        let config = make_launch_config(ADAPTER_DART_CLI, "test/widget_test.dart");
        let scenario = ext.dap_config_to_scenario(config).unwrap();

        assert_eq!(scenario.adapter, ADAPTER_DART_CLI);
        let cfg: serde_json::Value = serde_json::from_str(&scenario.config).unwrap();
        assert_eq!(cfg["testMode"], true);
    }

    #[test]
    fn scenario_launch_flutter_non_test() {
        let mut ext = DartDapExtension;
        let config = make_launch_config(ADAPTER_DART_FLUTTER, "lib/main.dart");
        let scenario = ext.dap_config_to_scenario(config).unwrap();

        assert_eq!(scenario.adapter, ADAPTER_DART_FLUTTER);
        let cfg: serde_json::Value = serde_json::from_str(&scenario.config).unwrap();
        assert_eq!(cfg["testMode"], false);
    }

    #[test]
    fn scenario_launch_flutter_test_file() {
        let mut ext = DartDapExtension;
        let config = make_launch_config(ADAPTER_DART_FLUTTER, "integration_test/app_test.dart");
        let scenario = ext.dap_config_to_scenario(config).unwrap();

        assert_eq!(scenario.adapter, ADAPTER_DART_FLUTTER);
        let cfg: serde_json::Value = serde_json::from_str(&scenario.config).unwrap();
        assert_eq!(cfg["testMode"], true);
    }

    #[test]
    fn scenario_launch_defaults_to_dart_cli() {
        let mut ext = DartDapExtension;
        let config = make_launch_config("SomeOther", "main.dart");
        let scenario = ext.dap_config_to_scenario(config).unwrap();

        assert_eq!(scenario.adapter, ADAPTER_DART_CLI);
    }

    #[test]
    fn scenario_attach_includes_vm_service_placeholder() {
        let mut ext = DartDapExtension;
        let config = make_attach_config(ADAPTER_DART_CLI, None);
        let scenario = ext.dap_config_to_scenario(config).unwrap();

        assert_eq!(scenario.adapter, ADAPTER_DART_CLI);
        let cfg: serde_json::Value = serde_json::from_str(&scenario.config).unwrap();
        assert_eq!(cfg["request"], "attach");
        assert_eq!(cfg["vmServiceUri"], "");
        assert_eq!(cfg["stopOnEntry"], false);
        assert!(cfg.get("processId").is_none());
    }

    #[test]
    fn scenario_attach_with_process_id() {
        let mut ext = DartDapExtension;
        let config = make_attach_config(ADAPTER_DART_CLI, Some(12345));
        let scenario = ext.dap_config_to_scenario(config).unwrap();

        let cfg: serde_json::Value = serde_json::from_str(&scenario.config).unwrap();
        assert_eq!(cfg["processId"], 12345);
    }

    #[test]
    fn scenario_attach_flutter() {
        let mut ext = DartDapExtension;
        let config = make_attach_config(ADAPTER_DART_FLUTTER, None);
        let scenario = ext.dap_config_to_scenario(config).unwrap();

        assert_eq!(scenario.adapter, ADAPTER_DART_FLUTTER);
        let cfg: serde_json::Value = serde_json::from_str(&scenario.config).unwrap();
        assert_eq!(cfg["request"], "attach");
        assert_eq!(cfg["vmServiceUri"], "");
    }

    #[test]
    fn scenario_attach_stop_on_entry() {
        let mut ext = DartDapExtension;
        let mut config = make_attach_config(ADAPTER_DART_CLI, None);
        config.stop_on_entry = Some(true);
        let scenario = ext.dap_config_to_scenario(config).unwrap();

        let cfg: serde_json::Value = serde_json::from_str(&scenario.config).unwrap();
        assert_eq!(cfg["stopOnEntry"], true);
    }

    #[test]
    fn scenario_preserves_label() {
        let mut ext = DartDapExtension;
        let mut config = make_launch_config(ADAPTER_DART_CLI, "main.dart");
        config.label = "My Custom Label".to_string();
        let scenario = ext.dap_config_to_scenario(config).unwrap();

        assert_eq!(scenario.label, "My Custom Label");
    }
}
