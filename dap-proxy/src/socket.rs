use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;
use tokio::sync::{mpsc, oneshot};

use crate::proxy::{PendingMap, SharedState};
use crate::seq::SeqAllocator;

const RESPONSE_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Deserialize)]
struct SocketCommand {
    command: String,
    #[serde(default)]
    arguments: serde_json::Value,
}

/// Build the socket path for this process: `/tmp/zed-dap-{pid}.sock`
pub fn socket_path() -> PathBuf {
    PathBuf::from(format!("/tmp/zed-dap-{}.sock", std::process::id()))
}

/// Write the socket path to `/tmp/zed-dap-latest` for easy discovery.
fn write_latest_pointer(path: &Path) {
    let _ = std::fs::write("/tmp/zed-dap-latest", path.display().to_string());
}

/// Clean up socket file and latest pointer.
pub fn cleanup(path: &Path) {
    let _ = std::fs::remove_file(path);
    if let Ok(contents) = std::fs::read_to_string("/tmp/zed-dap-latest") {
        if contents.trim() == path.display().to_string() {
            let _ = std::fs::remove_file("/tmp/zed-dap-latest");
        }
    }
}

/// Run the Unix socket listener.
///
/// Accepts connections, reads a JSON command, injects a DAP request,
/// waits for the response, and writes it back to the client.
pub async fn listen(
    path: PathBuf,
    seq: Arc<SeqAllocator>,
    child_stdin_tx: mpsc::Sender<Vec<u8>>,
    pending: PendingMap,
    state: SharedState,
) -> std::io::Result<()> {
    // Remove stale socket if it exists
    let _ = std::fs::remove_file(&path);

    let listener = UnixListener::bind(&path)?;
    write_latest_pointer(&path);
    eprintln!("[dap-proxy] listening on {}", path.display());

    loop {
        let (stream, _) = listener.accept().await?;
        let seq = Arc::clone(&seq);
        let child_stdin_tx = child_stdin_tx.clone();
        let pending = Arc::clone(&pending);
        let state = Arc::clone(&state);

        tokio::spawn(async move {
            if let Err(e) = handle_client(stream, seq, child_stdin_tx, pending, state).await {
                eprintln!("[dap-proxy] socket client error: {e}");
            }
        });
    }
}

async fn handle_client(
    stream: tokio::net::UnixStream,
    seq: Arc<SeqAllocator>,
    child_stdin_tx: mpsc::Sender<Vec<u8>>,
    pending: PendingMap,
    state: SharedState,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let (reader, mut writer) = stream.into_split();
    let mut lines = BufReader::new(reader).lines();

    while let Some(line) = lines.next_line().await? {
        let cmd: SocketCommand = serde_json::from_str(&line)?;

        // Handle meta-commands (proxy-local, not forwarded to adapter)
        match cmd.command.as_str() {
            "status" => {
                let s = state.read().await;
                let resp = serde_json::json!({
                    "vmServiceUri": s.vm_service_uri,
                });
                writer.write_all(format!("{resp}\n").as_bytes()).await?;
                continue;
            }
            "devtools" => {
                let s = state.read().await;
                let resp = match &s.vm_service_uri {
                    Some(ws_uri) => {
                        // Convert ws:// URI to DevTools URL
                        let http_uri = ws_uri.replace("ws://", "http://").replace("/ws", "");
                        let devtools_url = format!(
                            "https://devtools.flutter.dev/#/?uri={}",
                            urlencoded(&http_uri)
                        );
                        serde_json::json!({
                            "devtoolsUrl": devtools_url,
                            "vmServiceUri": ws_uri,
                        })
                    }
                    None => {
                        serde_json::json!({"error": "VM service URI not available yet"})
                    }
                };
                writer.write_all(format!("{resp}\n").as_bytes()).await?;
                continue;
            }
            _ => {} // Fall through to DAP request injection
        }

        // Inject as DAP request to the adapter
        let request_seq = seq.next();
        let arguments = if cmd.arguments.is_null() {
            serde_json::json!({})
        } else {
            cmd.arguments
        };
        let dap_request = serde_json::json!({
            "seq": request_seq,
            "type": "request",
            "command": cmd.command,
            "arguments": arguments
        });
        let body = serde_json::to_vec(&dap_request)?;

        // Register pending response
        let (tx, rx) = oneshot::channel();
        pending.lock().await.insert(request_seq, tx);

        // Send to child adapter
        child_stdin_tx
            .send(body)
            .await
            .map_err(|_| "child stdin closed")?;

        // Wait for response with timeout
        let response = match tokio::time::timeout(RESPONSE_TIMEOUT, rx).await {
            Ok(Ok(resp)) => resp,
            Ok(Err(_)) => serde_json::json!({"error": "response channel dropped"}),
            Err(_) => {
                pending.lock().await.remove(&request_seq);
                serde_json::json!({"error": "timeout waiting for response"})
            }
        };

        writer.write_all(format!("{response}\n").as_bytes()).await?;
    }

    Ok(())
}

/// Minimal percent-encoding for URL query parameters.
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
    use crate::proxy::AdapterState;
    use tokio::sync::{Mutex, RwLock};

    #[test]
    fn socket_path_contains_pid() {
        let path = socket_path();
        let expected = format!("/tmp/zed-dap-{}.sock", std::process::id());
        assert_eq!(path.to_str().unwrap(), expected);
    }

    #[test]
    fn parse_hot_reload_command() {
        let cmd: SocketCommand = serde_json::from_str(r#"{"command": "hotReload"}"#).unwrap();
        assert_eq!(cmd.command, "hotReload");
        assert!(cmd.arguments.is_null());
    }

    #[test]
    fn parse_hot_restart_command() {
        let cmd: SocketCommand = serde_json::from_str(r#"{"command": "hotRestart"}"#).unwrap();
        assert_eq!(cmd.command, "hotRestart");
    }

    #[test]
    fn parse_command_with_arguments() {
        let cmd: SocketCommand = serde_json::from_str(
            r#"{"command": "callService", "arguments": {"method": "ext.flutter.inspector.show"}}"#,
        )
        .unwrap();
        assert_eq!(cmd.command, "callService");
        assert_eq!(cmd.arguments["method"], "ext.flutter.inspector.show");
    }

    #[test]
    fn urlencoded_basic() {
        assert_eq!(
            urlencoded("http://127.0.0.1:8181/abc=/"),
            "http%3A%2F%2F127.0.0.1%3A8181%2Fabc%3D%2F"
        );
    }

    #[tokio::test]
    async fn socket_listen_and_respond() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.sock");
        let seq = Arc::new(SeqAllocator::new());
        let pending: PendingMap = Arc::new(Mutex::new(std::collections::HashMap::new()));
        let state: SharedState = Arc::new(RwLock::new(AdapterState::default()));
        let (child_tx, mut child_rx) = mpsc::channel::<Vec<u8>>(16);

        let listen_path = path.clone();
        let listen_pending = Arc::clone(&pending);
        let listen_seq = Arc::clone(&seq);
        let listen_state = Arc::clone(&state);
        let server = tokio::spawn(async move {
            listen(
                listen_path,
                listen_seq,
                child_tx,
                listen_pending,
                listen_state,
            )
            .await
            .ok();
        });

        // Give server time to bind
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Connect as client
        let stream = tokio::net::UnixStream::connect(&path).await.unwrap();
        let (reader, mut writer) = stream.into_split();
        let mut reader = BufReader::new(reader).lines();

        // Send a hotReload command
        writer
            .write_all(b"{\"command\": \"hotReload\"}\n")
            .await
            .unwrap();

        // The proxy should have forwarded a DAP request to child stdin
        let dap_body = child_rx.recv().await.unwrap();
        let dap_msg: serde_json::Value = serde_json::from_slice(&dap_body).unwrap();
        assert_eq!(dap_msg["command"], "hotReload");
        assert_eq!(dap_msg["type"], "request");
        let injected_seq = dap_msg["seq"].as_i64().unwrap();
        assert!(SeqAllocator::is_injected(injected_seq));

        // Simulate adapter response by delivering via pending map
        let response = serde_json::json!({
            "seq": 50,
            "type": "response",
            "request_seq": injected_seq,
            "success": true,
            "command": "hotReload"
        });
        {
            let mut map = pending.lock().await;
            if let Some(sender) = map.remove(&injected_seq) {
                sender.send(response).unwrap();
            }
        }

        // Read response from socket
        let line = reader.next_line().await.unwrap().unwrap();
        let resp: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(resp["success"], true);
        assert_eq!(resp["command"], "hotReload");

        server.abort();
    }

    #[tokio::test]
    async fn socket_status_command() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.sock");
        let seq = Arc::new(SeqAllocator::new());
        let pending: PendingMap = Arc::new(Mutex::new(std::collections::HashMap::new()));
        let state: SharedState = Arc::new(RwLock::new(AdapterState {
            vm_service_uri: Some("ws://127.0.0.1:8181/abc=/ws".to_string()),
        }));
        let (child_tx, _child_rx) = mpsc::channel::<Vec<u8>>(16);

        let listen_path = path.clone();
        let listen_pending = Arc::clone(&pending);
        let listen_seq = Arc::clone(&seq);
        let listen_state = Arc::clone(&state);
        let server = tokio::spawn(async move {
            listen(
                listen_path,
                listen_seq,
                child_tx,
                listen_pending,
                listen_state,
            )
            .await
            .ok();
        });

        tokio::time::sleep(Duration::from_millis(50)).await;

        let stream = tokio::net::UnixStream::connect(&path).await.unwrap();
        let (reader, mut writer) = stream.into_split();
        let mut reader = BufReader::new(reader).lines();

        // Send status command (meta-command, not forwarded to adapter)
        writer
            .write_all(b"{\"command\": \"status\"}\n")
            .await
            .unwrap();

        let line = reader.next_line().await.unwrap().unwrap();
        let resp: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(resp["vmServiceUri"], "ws://127.0.0.1:8181/abc=/ws");

        server.abort();
    }

    #[tokio::test]
    async fn socket_devtools_command() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.sock");
        let seq = Arc::new(SeqAllocator::new());
        let pending: PendingMap = Arc::new(Mutex::new(std::collections::HashMap::new()));
        let state: SharedState = Arc::new(RwLock::new(AdapterState {
            vm_service_uri: Some("ws://127.0.0.1:8181/abc=/ws".to_string()),
        }));
        let (child_tx, _child_rx) = mpsc::channel::<Vec<u8>>(16);

        let listen_path = path.clone();
        let listen_pending = Arc::clone(&pending);
        let listen_seq = Arc::clone(&seq);
        let listen_state = Arc::clone(&state);
        let server = tokio::spawn(async move {
            listen(
                listen_path,
                listen_seq,
                child_tx,
                listen_pending,
                listen_state,
            )
            .await
            .ok();
        });

        tokio::time::sleep(Duration::from_millis(50)).await;

        let stream = tokio::net::UnixStream::connect(&path).await.unwrap();
        let (reader, mut writer) = stream.into_split();
        let mut reader = BufReader::new(reader).lines();

        writer
            .write_all(b"{\"command\": \"devtools\"}\n")
            .await
            .unwrap();

        let line = reader.next_line().await.unwrap().unwrap();
        let resp: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert!(resp["devtoolsUrl"]
            .as_str()
            .unwrap()
            .starts_with("https://devtools.flutter.dev/#/?uri="));
        assert_eq!(resp["vmServiceUri"], "ws://127.0.0.1:8181/abc=/ws");

        server.abort();
    }

    #[tokio::test]
    async fn socket_devtools_no_uri() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.sock");
        let seq = Arc::new(SeqAllocator::new());
        let pending: PendingMap = Arc::new(Mutex::new(std::collections::HashMap::new()));
        let state: SharedState = Arc::new(RwLock::new(AdapterState::default()));
        let (child_tx, _child_rx) = mpsc::channel::<Vec<u8>>(16);

        let listen_path = path.clone();
        let listen_pending = Arc::clone(&pending);
        let listen_seq = Arc::clone(&seq);
        let listen_state = Arc::clone(&state);
        let server = tokio::spawn(async move {
            listen(
                listen_path,
                listen_seq,
                child_tx,
                listen_pending,
                listen_state,
            )
            .await
            .ok();
        });

        tokio::time::sleep(Duration::from_millis(50)).await;

        let stream = tokio::net::UnixStream::connect(&path).await.unwrap();
        let (reader, mut writer) = stream.into_split();
        let mut reader = BufReader::new(reader).lines();

        writer
            .write_all(b"{\"command\": \"devtools\"}\n")
            .await
            .unwrap();

        let line = reader.next_line().await.unwrap().unwrap();
        let resp: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert!(resp["error"].as_str().unwrap().contains("not available"));

        server.abort();
    }
}
