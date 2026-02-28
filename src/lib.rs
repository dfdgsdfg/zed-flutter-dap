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

    /// Human-readable display name for error messages.
    fn display_name(&self) -> &'static str {
        match self {
            TargetKind::DartLaunch => "Dart launch",
            TargetKind::DartAttach => "Dart attach",
            TargetKind::DartTestLaunch => "Dart test",
            TargetKind::FlutterLaunch => "Flutter launch",
            TargetKind::FlutterAttach => "Flutter attach",
            TargetKind::FlutterTestLaunch => "Flutter test",
        }
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

/// Validate config fields required by the target kind before handing off to the debug adapter.
///
/// Returns `Ok(())` if valid, or an actionable error message if required fields are missing.
fn validate_config(target: TargetKind, config: &serde_json::Value) -> Result<(), String> {
    match target.request_kind() {
        StartDebuggingRequestArgumentsRequest::Launch => {
            // Test mode doesn't require 'program' — the test runner discovers tests.
            if !target.is_test() {
                match config.get("program").and_then(|v| v.as_str()) {
                    None => {
                        return Err(format!(
                            "{}: Launch configuration requires a 'program' field. \
                             Set it to the Dart file to run (e.g., \"bin/main.dart\").",
                            target.display_name()
                        ));
                    }
                    Some(p) if p.is_empty() => {
                        return Err(format!(
                            "{}: 'program' field must not be empty. \
                             Set it to the Dart file to run (e.g., \"bin/main.dart\").",
                            target.display_name()
                        ));
                    }
                    _ => {}
                }
            }
        }
        StartDebuggingRequestArgumentsRequest::Attach => {
            match config.get("vmServiceUri").and_then(|v| v.as_str()) {
                None => {
                    return Err(format!(
                        "{}: Attach configuration requires a 'vmServiceUri' field. \
                         Set it to the Dart VM service URI (e.g., \"ws://127.0.0.1:8181/abcd=/ws\").",
                        target.display_name()
                    ));
                }
                Some(uri) if uri.is_empty() => {
                    return Err(format!(
                        "{}: 'vmServiceUri' must not be empty. \
                         Set it to the Dart VM service URI (e.g., \"ws://127.0.0.1:8181/abcd=/ws\").",
                        target.display_name()
                    ));
                }
                _ => {}
            }
        }
    }

    Ok(())
}

/// Build a `DebugAdapterBinary` from a resolved binary path, target kind, and config.
///
/// This function is independent of `Worktree`, making it testable in unit tests.
fn build_debug_adapter_binary(
    command: String,
    target: TargetKind,
    config: &serde_json::Value,
    raw_config: String,
) -> DebugAdapterBinary {
    let mut arguments = vec![target.adapter_subcommand().to_string()];
    if target.is_test() {
        arguments.push("--test".to_string());
    }

    DebugAdapterBinary {
        command: Some(command),
        arguments,
        envs: collect_env(config),
        cwd: config
            .get("cwd")
            .and_then(|v| v.as_str())
            .map(String::from),
        connection: None,
        request_args: StartDebuggingRequestArguments {
            configuration: raw_config,
            request: target.request_kind(),
        },
    }
}

/// Extract the configured SDK binary path from config, if present and non-empty.
fn sdk_path_override(config: &serde_json::Value, key: &str) -> Option<String> {
    config
        .get(key)
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from)
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

        if !config_value.is_object() {
            return Err("Debug configuration must be a JSON object.".to_string());
        }

        let target = classify_target(&adapter_name, &config_value)?;
        validate_config(target, &config_value)?;

        let command = match target {
            TargetKind::DartLaunch | TargetKind::DartAttach | TargetKind::DartTestLaunch => {
                resolve_dart_binary(&config_value, worktree)?
            }
            TargetKind::FlutterLaunch | TargetKind::FlutterAttach | TargetKind::FlutterTestLaunch => {
                resolve_flutter_binary(&config_value, worktree)?
            }
        };

        Ok(build_debug_adapter_binary(
            command,
            target,
            &config_value,
            config.config,
        ))
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
    if let Some(path) = sdk_path_override(config, "dartSdkPath") {
        return Ok(path);
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
    if let Some(path) = sdk_path_override(config, "flutterSdkPath") {
        return Ok(path);
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

    // --- Schema validation tests ---

    const DART_CLI_SCHEMA: &str = include_str!("../debug_adapter_schemas/DartCLI.json");
    const DART_FLUTTER_SCHEMA: &str = include_str!("../debug_adapter_schemas/DartFlutter.json");

    /// Helper: parse a schema and return its Value.
    fn parse_schema(raw: &str) -> serde_json::Value {
        serde_json::from_str(raw).expect("Schema must be valid JSON")
    }

    #[test]
    fn schema_dart_cli_is_valid_json() {
        let schema = parse_schema(DART_CLI_SCHEMA);
        assert_eq!(schema["type"], "object");
        assert_eq!(schema["$schema"], "http://json-schema.org/draft-07/schema#");
        assert!(schema["title"].as_str().unwrap().contains("Dart CLI"));
    }

    #[test]
    fn schema_dart_flutter_is_valid_json() {
        let schema = parse_schema(DART_FLUTTER_SCHEMA);
        assert_eq!(schema["type"], "object");
        assert_eq!(schema["$schema"], "http://json-schema.org/draft-07/schema#");
        assert!(schema["title"].as_str().unwrap().contains("Flutter"));
    }

    #[test]
    fn schema_dart_cli_required_fields() {
        let schema = parse_schema(DART_CLI_SCHEMA);
        let required: Vec<&str> = schema["required"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert!(required.contains(&"adapter"));
        assert!(required.contains(&"label"));
        assert!(required.contains(&"request"));
    }

    #[test]
    fn schema_dart_flutter_required_fields() {
        let schema = parse_schema(DART_FLUTTER_SCHEMA);
        let required: Vec<&str> = schema["required"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert!(required.contains(&"adapter"));
        assert!(required.contains(&"label"));
        assert!(required.contains(&"request"));
    }

    #[test]
    fn schema_dart_cli_adapter_enum_matches_constant() {
        let schema = parse_schema(DART_CLI_SCHEMA);
        let adapter_enum = schema["properties"]["adapter"]["enum"].as_array().unwrap();
        assert_eq!(adapter_enum.len(), 1);
        assert_eq!(adapter_enum[0], ADAPTER_DART_CLI);
    }

    #[test]
    fn schema_dart_flutter_adapter_enum_matches_constant() {
        let schema = parse_schema(DART_FLUTTER_SCHEMA);
        let adapter_enum = schema["properties"]["adapter"]["enum"].as_array().unwrap();
        assert_eq!(adapter_enum.len(), 1);
        assert_eq!(adapter_enum[0], ADAPTER_DART_FLUTTER);
    }

    #[test]
    fn schema_dart_cli_request_enum() {
        let schema = parse_schema(DART_CLI_SCHEMA);
        let request_enum = schema["properties"]["request"]["enum"].as_array().unwrap();
        let values: Vec<&str> = request_enum.iter().map(|v| v.as_str().unwrap()).collect();
        assert_eq!(values.len(), 2);
        assert!(values.contains(&"launch"));
        assert!(values.contains(&"attach"));
    }

    #[test]
    fn schema_dart_flutter_request_enum() {
        let schema = parse_schema(DART_FLUTTER_SCHEMA);
        let request_enum = schema["properties"]["request"]["enum"].as_array().unwrap();
        let values: Vec<&str> = request_enum.iter().map(|v| v.as_str().unwrap()).collect();
        assert_eq!(values.len(), 2);
        assert!(values.contains(&"launch"));
        assert!(values.contains(&"attach"));
    }

    /// Helper: find a oneOf variant by request const value.
    fn find_one_of_variant<'a>(
        schema: &'a serde_json::Value,
        request_value: &str,
    ) -> &'a serde_json::Value {
        schema["oneOf"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["properties"]["request"]["const"] == request_value)
            .unwrap_or_else(|| panic!("oneOf variant for '{request_value}' not found"))
    }

    #[test]
    fn schema_dart_cli_one_of_has_two_variants() {
        let schema = parse_schema(DART_CLI_SCHEMA);
        assert_eq!(schema["oneOf"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn schema_dart_cli_launch_requires_program() {
        let schema = parse_schema(DART_CLI_SCHEMA);
        let launch = find_one_of_variant(&schema, "launch");
        let required: Vec<&str> = launch["required"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert!(required.contains(&"program"));
    }

    #[test]
    fn schema_dart_cli_attach_requires_vm_service_uri() {
        let schema = parse_schema(DART_CLI_SCHEMA);
        let attach = find_one_of_variant(&schema, "attach");
        let required: Vec<&str> = attach["required"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert!(required.contains(&"vmServiceUri"));
    }

    #[test]
    fn schema_dart_flutter_one_of_has_two_variants() {
        let schema = parse_schema(DART_FLUTTER_SCHEMA);
        assert_eq!(schema["oneOf"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn schema_dart_flutter_launch_requires_program() {
        let schema = parse_schema(DART_FLUTTER_SCHEMA);
        let launch = find_one_of_variant(&schema, "launch");
        let required: Vec<&str> = launch["required"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert!(required.contains(&"program"));
    }

    #[test]
    fn schema_dart_flutter_attach_requires_vm_service_uri() {
        let schema = parse_schema(DART_FLUTTER_SCHEMA);
        let attach = find_one_of_variant(&schema, "attach");
        let required: Vec<&str> = attach["required"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert!(required.contains(&"vmServiceUri"));
    }

    #[test]
    fn schema_dart_cli_defaults() {
        let schema = parse_schema(DART_CLI_SCHEMA);
        let props = &schema["properties"];
        assert_eq!(props["cwd"]["default"], "$ZED_WORKTREE_ROOT");
        assert_eq!(props["args"]["default"], serde_json::json!([]));
        assert_eq!(props["env"]["default"], serde_json::json!({}));
        assert_eq!(props["testMode"]["default"], false);
        assert_eq!(props["stopOnEntry"]["default"], false);
    }

    #[test]
    fn schema_dart_flutter_defaults() {
        let schema = parse_schema(DART_FLUTTER_SCHEMA);
        let props = &schema["properties"];
        assert_eq!(props["cwd"]["default"], "$ZED_WORKTREE_ROOT");
        assert_eq!(props["args"]["default"], serde_json::json!([]));
        assert_eq!(props["env"]["default"], serde_json::json!({}));
        assert_eq!(props["testMode"]["default"], false);
        assert_eq!(props["stopOnEntry"]["default"], false);
        assert_eq!(props["program"]["default"], "lib/main.dart");
    }

    #[test]
    fn schema_dart_cli_property_types() {
        let schema = parse_schema(DART_CLI_SCHEMA);
        let props = &schema["properties"];
        assert_eq!(props["program"]["type"], "string");
        assert_eq!(props["cwd"]["type"], "string");
        assert_eq!(props["args"]["type"], "array");
        assert_eq!(props["env"]["type"], "object");
        assert_eq!(props["vmServiceUri"]["type"], "string");
        assert_eq!(props["testMode"]["type"], "boolean");
        assert_eq!(props["dartSdkPath"]["type"], "string");
        assert_eq!(props["stopOnEntry"]["type"], "boolean");
    }

    #[test]
    fn schema_dart_flutter_property_types() {
        let schema = parse_schema(DART_FLUTTER_SCHEMA);
        let props = &schema["properties"];
        assert_eq!(props["program"]["type"], "string");
        assert_eq!(props["cwd"]["type"], "string");
        assert_eq!(props["args"]["type"], "array");
        assert_eq!(props["env"]["type"], "object");
        assert_eq!(props["vmServiceUri"]["type"], "string");
        assert_eq!(props["testMode"]["type"], "boolean");
        assert_eq!(props["flutterSdkPath"]["type"], "string");
        assert_eq!(props["stopOnEntry"]["type"], "boolean");
    }

    #[test]
    fn schema_dart_cli_has_dart_sdk_path_not_flutter() {
        let schema = parse_schema(DART_CLI_SCHEMA);
        let props = schema["properties"].as_object().unwrap();
        assert!(props.contains_key("dartSdkPath"));
        assert!(!props.contains_key("flutterSdkPath"));
    }

    #[test]
    fn schema_dart_flutter_has_flutter_sdk_path_not_dart() {
        let schema = parse_schema(DART_FLUTTER_SCHEMA);
        let props = schema["properties"].as_object().unwrap();
        assert!(props.contains_key("flutterSdkPath"));
        assert!(!props.contains_key("dartSdkPath"));
    }

    // --- sdk_path_override tests ---

    #[test]
    fn sdk_path_override_returns_configured_path() {
        let config = serde_json::json!({"dartSdkPath": "/usr/local/dart/bin/dart"});
        assert_eq!(
            sdk_path_override(&config, "dartSdkPath"),
            Some("/usr/local/dart/bin/dart".to_string())
        );
    }

    #[test]
    fn sdk_path_override_returns_none_for_empty() {
        let config = serde_json::json!({"dartSdkPath": ""});
        assert_eq!(sdk_path_override(&config, "dartSdkPath"), None);
    }

    #[test]
    fn sdk_path_override_returns_none_for_missing() {
        let config = serde_json::json!({"program": "main.dart"});
        assert_eq!(sdk_path_override(&config, "dartSdkPath"), None);
    }

    #[test]
    fn sdk_path_override_returns_none_for_non_string() {
        let config = serde_json::json!({"dartSdkPath": 42});
        assert_eq!(sdk_path_override(&config, "dartSdkPath"), None);
    }

    #[test]
    fn sdk_path_override_flutter() {
        let config = serde_json::json!({"flutterSdkPath": "/opt/flutter/bin/flutter"});
        assert_eq!(
            sdk_path_override(&config, "flutterSdkPath"),
            Some("/opt/flutter/bin/flutter".to_string())
        );
    }

    // --- build_debug_adapter_binary descriptor snapshot tests ---

    /// Helper to build a descriptor for a given target kind with standard test config.
    fn make_descriptor(target: TargetKind) -> DebugAdapterBinary {
        let config = serde_json::json!({
            "request": if target.request_kind() == StartDebuggingRequestArgumentsRequest::Launch {
                "launch"
            } else {
                "attach"
            },
            "program": "bin/main.dart",
            "cwd": "/workspace/my_app",
            "env": {"DART_VM_OPTIONS": "--enable-asserts"},
            "testMode": target.is_test(),
        });
        let raw = config.to_string();
        build_debug_adapter_binary("/usr/bin/dart".to_string(), target, &config, raw)
    }

    #[test]
    fn descriptor_dart_launch() {
        let bin = make_descriptor(TargetKind::DartLaunch);
        assert_eq!(bin.command, Some("/usr/bin/dart".to_string()));
        assert_eq!(bin.arguments, vec!["debug_adapter"]);
        assert_eq!(bin.cwd, Some("/workspace/my_app".to_string()));
        assert_eq!(bin.envs, vec![("DART_VM_OPTIONS".to_string(), "--enable-asserts".to_string())]);
        assert!(bin.connection.is_none());
        assert_eq!(bin.request_args.request, StartDebuggingRequestArgumentsRequest::Launch);
    }

    #[test]
    fn descriptor_dart_attach() {
        let bin = make_descriptor(TargetKind::DartAttach);
        assert_eq!(bin.command, Some("/usr/bin/dart".to_string()));
        assert_eq!(bin.arguments, vec!["debug_adapter"]);
        assert_eq!(bin.request_args.request, StartDebuggingRequestArgumentsRequest::Attach);
    }

    #[test]
    fn descriptor_dart_test_launch() {
        let bin = make_descriptor(TargetKind::DartTestLaunch);
        assert_eq!(bin.command, Some("/usr/bin/dart".to_string()));
        assert_eq!(bin.arguments, vec!["debug_adapter", "--test"]);
        assert_eq!(bin.request_args.request, StartDebuggingRequestArgumentsRequest::Launch);
    }

    #[test]
    fn descriptor_flutter_launch() {
        let bin = make_descriptor(TargetKind::FlutterLaunch);
        assert_eq!(bin.command, Some("/usr/bin/dart".to_string()));
        assert_eq!(bin.arguments, vec!["debug-adapter"]);
        assert_eq!(bin.request_args.request, StartDebuggingRequestArgumentsRequest::Launch);
    }

    #[test]
    fn descriptor_flutter_attach() {
        let bin = make_descriptor(TargetKind::FlutterAttach);
        assert_eq!(bin.command, Some("/usr/bin/dart".to_string()));
        assert_eq!(bin.arguments, vec!["debug-adapter"]);
        assert_eq!(bin.request_args.request, StartDebuggingRequestArgumentsRequest::Attach);
    }

    #[test]
    fn descriptor_flutter_test_launch() {
        let bin = make_descriptor(TargetKind::FlutterTestLaunch);
        assert_eq!(bin.command, Some("/usr/bin/dart".to_string()));
        assert_eq!(bin.arguments, vec!["debug-adapter", "--test"]);
        assert_eq!(bin.request_args.request, StartDebuggingRequestArgumentsRequest::Launch);
    }

    #[test]
    fn descriptor_preserves_raw_config_in_request_args() {
        let config = serde_json::json!({"request": "launch", "program": "main.dart"});
        let raw = config.to_string();
        let bin = build_debug_adapter_binary(
            "dart".to_string(),
            TargetKind::DartLaunch,
            &config,
            raw.clone(),
        );
        assert_eq!(bin.request_args.configuration, raw);
    }

    #[test]
    fn descriptor_no_cwd_when_absent() {
        let config = serde_json::json!({"request": "launch", "program": "main.dart"});
        let raw = config.to_string();
        let bin = build_debug_adapter_binary(
            "dart".to_string(),
            TargetKind::DartLaunch,
            &config,
            raw,
        );
        assert!(bin.cwd.is_none());
    }

    #[test]
    fn descriptor_empty_env_when_absent() {
        let config = serde_json::json!({"request": "launch", "program": "main.dart"});
        let raw = config.to_string();
        let bin = build_debug_adapter_binary(
            "dart".to_string(),
            TargetKind::DartLaunch,
            &config,
            raw,
        );
        assert!(bin.envs.is_empty());
    }

    // --- validate_config tests ---

    #[test]
    fn validate_launch_missing_program() {
        let config = serde_json::json!({"request": "launch"});
        let err = validate_config(TargetKind::DartLaunch, &config).unwrap_err();
        assert!(err.contains("Dart launch"), "error should name the target: {err}");
        assert!(err.contains("program"), "error should mention 'program': {err}");
    }

    #[test]
    fn validate_launch_empty_program() {
        let config = serde_json::json!({"request": "launch", "program": ""});
        let err = validate_config(TargetKind::DartLaunch, &config).unwrap_err();
        assert!(err.contains("must not be empty"), "error should say empty: {err}");
    }

    #[test]
    fn validate_launch_valid_program() {
        let config = serde_json::json!({"request": "launch", "program": "bin/main.dart"});
        assert!(validate_config(TargetKind::DartLaunch, &config).is_ok());
    }

    #[test]
    fn validate_launch_program_non_string_treated_as_missing() {
        let config = serde_json::json!({"request": "launch", "program": 42});
        let err = validate_config(TargetKind::DartLaunch, &config).unwrap_err();
        assert!(err.contains("program"), "error should mention 'program': {err}");
    }

    #[test]
    fn validate_attach_missing_vm_service_uri() {
        let config = serde_json::json!({"request": "attach"});
        let err = validate_config(TargetKind::DartAttach, &config).unwrap_err();
        assert!(err.contains("Dart attach"), "error should name the target: {err}");
        assert!(err.contains("vmServiceUri"), "error should mention 'vmServiceUri': {err}");
    }

    #[test]
    fn validate_attach_empty_vm_service_uri() {
        let config = serde_json::json!({"request": "attach", "vmServiceUri": ""});
        let err = validate_config(TargetKind::DartAttach, &config).unwrap_err();
        assert!(err.contains("must not be empty"), "error should say empty: {err}");
    }

    #[test]
    fn validate_attach_valid_vm_service_uri() {
        let config = serde_json::json!({"request": "attach", "vmServiceUri": "ws://127.0.0.1:8181/ws"});
        assert!(validate_config(TargetKind::DartAttach, &config).is_ok());
    }

    #[test]
    fn validate_attach_vm_service_uri_non_string_treated_as_missing() {
        let config = serde_json::json!({"request": "attach", "vmServiceUri": true});
        let err = validate_config(TargetKind::DartAttach, &config).unwrap_err();
        assert!(err.contains("vmServiceUri"), "error should mention 'vmServiceUri': {err}");
    }

    #[test]
    fn validate_flutter_launch_missing_program() {
        let config = serde_json::json!({"request": "launch"});
        let err = validate_config(TargetKind::FlutterLaunch, &config).unwrap_err();
        assert!(err.contains("Flutter launch"), "error should name the target: {err}");
    }

    #[test]
    fn validate_flutter_attach_missing_vm_service_uri() {
        let config = serde_json::json!({"request": "attach"});
        let err = validate_config(TargetKind::FlutterAttach, &config).unwrap_err();
        assert!(err.contains("Flutter attach"), "error should name the target: {err}");
    }

    #[test]
    fn validate_test_launch_skips_program_check() {
        // Test mode doesn't require 'program' — tests are discovered automatically
        let config = serde_json::json!({"request": "launch", "testMode": true});
        assert!(validate_config(TargetKind::DartTestLaunch, &config).is_ok());
    }

    #[test]
    fn validate_flutter_test_launch_skips_program_check() {
        let config = serde_json::json!({"request": "launch", "testMode": true});
        assert!(validate_config(TargetKind::FlutterTestLaunch, &config).is_ok());
    }

    // --- TargetKind::display_name tests ---

    #[test]
    fn target_kind_display_names() {
        assert_eq!(TargetKind::DartLaunch.display_name(), "Dart launch");
        assert_eq!(TargetKind::DartAttach.display_name(), "Dart attach");
        assert_eq!(TargetKind::DartTestLaunch.display_name(), "Dart test");
        assert_eq!(TargetKind::FlutterLaunch.display_name(), "Flutter launch");
        assert_eq!(TargetKind::FlutterAttach.display_name(), "Flutter attach");
        assert_eq!(TargetKind::FlutterTestLaunch.display_name(), "Flutter test");
    }
}
