use std::path::Path;
use std::process::Stdio;

use serde::Deserialize;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::task::JoinHandle;

pub struct DevToolsManager {
    dart_command: String,
    server: Option<RunningDevTools>,
}

struct RunningDevTools {
    child: Option<Child>,
    stdout_task: Option<JoinHandle<()>>,
    host: String,
    port: u16,
}

#[derive(Debug, Deserialize)]
struct DevToolsMachineEvent {
    event: Option<String>,
    method: Option<String>,
    params: Option<DevToolsServerStartedParams>,
}

#[derive(Debug, Deserialize)]
struct DevToolsServerStartedParams {
    host: String,
    port: u16,
}

impl DevToolsManager {
    pub fn new(adapter_command: &str) -> Self {
        Self {
            dart_command: derive_dart_command(adapter_command),
            server: None,
        }
    }

    pub async fn devtools_url(&mut self, vm_service_ws_uri: &str) -> Result<String, String> {
        if self.server.is_none() {
            self.server = Some(start_devtools_server(&self.dart_command).await?);
        }

        let server = self.server.as_ref().expect("server initialized");
        let vm_service_http_uri = vm_service_http_uri(vm_service_ws_uri)?;
        Ok(build_local_devtools_url(
            &server.host,
            server.port,
            &vm_service_http_uri,
        ))
    }

    pub async fn shutdown(&mut self) {
        if let Some(mut server) = self.server.take() {
            if let Some(child) = server.child.as_mut() {
                let _ = child.kill().await;
                let _ = child.wait().await;
            }
            if let Some(stdout_task) = server.stdout_task.take() {
                stdout_task.abort();
            }
        }
    }

    #[cfg(test)]
    pub fn with_server_for_test(host: &str, port: u16) -> Self {
        Self {
            dart_command: "dart".to_string(),
            server: Some(RunningDevTools {
                child: None,
                stdout_task: None,
                host: host.to_string(),
                port,
            }),
        }
    }
}

pub fn derive_dart_command(adapter_command: &str) -> String {
    Path::new(adapter_command)
        .with_file_name("dart")
        .to_string_lossy()
        .into_owned()
}

fn build_local_devtools_url(host: &str, port: u16, vm_service_http_uri: &str) -> String {
    format!(
        "http://{host}:{port}?uri={}",
        urlencoded(vm_service_http_uri)
    )
}

fn parse_server_started_event(line: &str) -> Option<(String, u16)> {
    let event: DevToolsMachineEvent = serde_json::from_str(line).ok()?;
    let name = event.event.or(event.method)?;
    if name != "server.started" {
        return None;
    }
    let params = event.params?;
    Some((params.host, params.port))
}

fn vm_service_http_uri(vm_service_uri: &str) -> Result<String, String> {
    if let Some(rest) = vm_service_uri.strip_prefix("ws://") {
        let base = rest
            .strip_suffix("/ws")
            .map(|s| format!("{s}/"))
            .unwrap_or_else(|| rest.to_string());
        return Ok(format!("http://{base}"));
    }

    if let Some(rest) = vm_service_uri.strip_prefix("wss://") {
        let base = rest
            .strip_suffix("/ws")
            .map(|s| format!("{s}/"))
            .unwrap_or_else(|| rest.to_string());
        return Ok(format!("https://{base}"));
    }

    if vm_service_uri.starts_with("http://") || vm_service_uri.starts_with("https://") {
        return Ok(vm_service_uri.to_string());
    }

    Err(format!("Unsupported VM service URI: {vm_service_uri}"))
}

async fn start_devtools_server(dart_command: &str) -> Result<RunningDevTools, String> {
    let mut child = Command::new(dart_command)
        .args([
            "devtools",
            "--machine",
            "--no-launch-browser",
            "--port",
            "0",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("Failed to start local DevTools with '{dart_command}': {e}"))?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "Failed to capture local DevTools stdout".to_string())?;
    let mut lines = BufReader::new(stdout).lines();

    while let Some(line) = lines
        .next_line()
        .await
        .map_err(|e| format!("Failed reading local DevTools startup output: {e}"))?
    {
        if let Some((host, port)) = parse_server_started_event(&line) {
            let stdout_task =
                tokio::spawn(async move { while let Ok(Some(_)) = lines.next_line().await {} });
            return Ok(RunningDevTools {
                child: Some(child),
                stdout_task: Some(stdout_task),
                host,
                port,
            });
        }
    }

    let status = child
        .wait()
        .await
        .map_err(|e| format!("Local DevTools exited before startup completed: {e}"))?;
    Err(format!(
        "Local DevTools exited before reporting its server URL (status: {status})"
    ))
}

fn urlencoded(s: &str) -> String {
    let mut result = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                result.push(b as char);
            }
            _ => {
                result.push_str(&format!("%{b:02X}"));
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_dart_command_from_flutter_binary() {
        assert_eq!(
            derive_dart_command("/opt/flutter/bin/flutter"),
            "/opt/flutter/bin/dart"
        );
    }

    #[test]
    fn derive_dart_command_from_plain_flutter_command() {
        assert_eq!(derive_dart_command("flutter"), "dart");
    }

    #[test]
    fn parse_server_started_event_line() {
        let line = r#"{"event":"server.started","method":"server.started","params":{"host":"127.0.0.1","port":9100,"pid":1,"protocolVersion":"1.2.0"}}"#;
        assert_eq!(
            parse_server_started_event(line),
            Some(("127.0.0.1".to_string(), 9100))
        );
    }

    #[test]
    fn vm_service_http_uri_converts_ws_to_http() {
        assert_eq!(
            vm_service_http_uri("ws://127.0.0.1:62412/token=/ws").unwrap(),
            "http://127.0.0.1:62412/token=/"
        );
    }

    #[test]
    fn vm_service_http_uri_converts_wss_to_https() {
        assert_eq!(
            vm_service_http_uri("wss://example.com/token/ws").unwrap(),
            "https://example.com/token/"
        );
    }

    #[test]
    fn build_local_devtools_url_uses_http_service_uri() {
        assert_eq!(
            build_local_devtools_url("127.0.0.1", 9100, "http://127.0.0.1:62412/token=/"),
            "http://127.0.0.1:9100?uri=http%3A%2F%2F127.0.0.1%3A62412%2Ftoken%3D%2F"
        );
    }

    #[tokio::test]
    async fn manager_reuses_running_server() {
        let mut manager = DevToolsManager::with_server_for_test("127.0.0.1", 9100);
        let url = manager
            .devtools_url("ws://127.0.0.1:62412/token=/ws")
            .await
            .unwrap();
        assert_eq!(
            url,
            "http://127.0.0.1:9100?uri=http%3A%2F%2F127.0.0.1%3A62412%2Ftoken%3D%2F"
        );
    }
}
