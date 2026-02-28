use std::collections::HashMap;
use std::sync::Arc;

use tokio::io::{AsyncWrite, BufReader};
use tokio::process::ChildStdout;
use tokio::sync::{oneshot, Mutex, RwLock};

use crate::dap;
use crate::seq::SeqAllocator;

/// Pending response map: injected seq -> oneshot sender to deliver the response.
pub type PendingMap = Arc<Mutex<HashMap<i64, oneshot::Sender<serde_json::Value>>>>;

/// Shared state extracted from adapter events.
#[derive(Default)]
pub struct AdapterState {
    /// VM service URI from `dart.debuggerUris` event.
    pub vm_service_uri: Option<String>,
}

pub type SharedState = Arc<RwLock<AdapterState>>;

/// Read DAP messages from Zed's stdin and forward to the child adapter's stdin via channel.
pub async fn zed_to_adapter(
    stdin: tokio::io::Stdin,
    tx: tokio::sync::mpsc::Sender<Vec<u8>>,
) -> std::io::Result<()> {
    let mut reader = BufReader::new(stdin);
    while let Some(body) = dap::read_message(&mut reader).await {
        if tx.send(body).await.is_err() {
            break; // child stdin closed
        }
    }
    Ok(())
}

/// Read DAP messages from the child adapter's stdout and route them.
///
/// - Responses with `request_seq >= 100_000` are routed to the socket handler via `pending`.
/// - Events like `dart.debuggerUris` are intercepted to capture state.
/// - Everything is forwarded to Zed's stdout (events are not consumed).
pub async fn adapter_to_zed<W: AsyncWrite + Unpin>(
    child_stdout: ChildStdout,
    mut zed_stdout: W,
    pending: PendingMap,
    state: SharedState,
) -> std::io::Result<()> {
    let mut reader = BufReader::new(child_stdout);
    while let Some(body) = dap::read_message(&mut reader).await {
        if let Ok(msg) = serde_json::from_slice::<serde_json::Value>(&body) {
            let msg_type = msg.get("type").and_then(|t| t.as_str());

            // Intercept events to capture state (but still forward them)
            if msg_type == Some("event") {
                extract_state(&msg, &state).await;
            }

            // Route injected responses to socket handler (don't forward to Zed)
            if msg_type == Some("response") {
                if let Some(request_seq) = msg.get("request_seq").and_then(|s| s.as_i64()) {
                    if SeqAllocator::is_injected(request_seq) {
                        let mut map = pending.lock().await;
                        if let Some(sender) = map.remove(&request_seq) {
                            let _ = sender.send(msg);
                        }
                        continue;
                    }
                }
            }
        }

        // Forward to Zed
        dap::write_message(&mut zed_stdout, &body).await?;
    }
    Ok(())
}

/// Extract useful state from adapter events.
async fn extract_state(msg: &serde_json::Value, state: &SharedState) {
    let event = msg.get("event").and_then(|e| e.as_str()).unwrap_or("");
    let body = msg.get("body");

    // dart.debuggerUris: {"vmServiceUri": "ws://127.0.0.1:XXXXX/XXXXX=/ws"}
    if event == "dart.debuggerUris" {
        if let Some(uri) = body
            .and_then(|b| b.get("vmServiceUri"))
            .and_then(|u| u.as_str())
        {
            let mut s = state.write().await;
            s.vm_service_uri = Some(uri.to_string());
            eprintln!("[dap-proxy] captured vmServiceUri: {uri}");
        }
    }
}

/// Drain the mpsc channel and write messages to the child adapter's stdin.
///
/// Serializes writes from both the Zed proxy task and the socket handler.
pub async fn stdin_writer<W: AsyncWrite + Unpin>(
    mut rx: tokio::sync::mpsc::Receiver<Vec<u8>>,
    mut child_stdin: W,
) -> std::io::Result<()> {
    while let Some(body) = rx.recv().await {
        dap::write_message(&mut child_stdin, &body).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dap;
    use std::io::Cursor;
    use tokio::io::BufReader;

    #[tokio::test]
    async fn adapter_to_zed_forwards_normal_response() {
        let msg = serde_json::json!({
            "seq": 5,
            "type": "response",
            "request_seq": 1,
            "success": true,
            "command": "initialize"
        });
        let body = serde_json::to_vec(&msg).unwrap();
        let mut framed = Vec::new();
        dap::write_message(&mut framed, &body).await.unwrap();

        let child_stdout = Cursor::new(framed);
        let pending: PendingMap = Arc::new(Mutex::new(HashMap::new()));

        let mut reader = BufReader::new(child_stdout);
        let body = dap::read_message(&mut reader).await.unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();

        let request_seq = parsed["request_seq"].as_i64().unwrap();
        assert!(!SeqAllocator::is_injected(request_seq));
        assert!(pending.lock().await.is_empty());
    }

    #[tokio::test]
    async fn routing_injected_response() {
        let pending: PendingMap = Arc::new(Mutex::new(HashMap::new()));
        let (tx, rx) = oneshot::channel();
        pending.lock().await.insert(100_000, tx);

        let msg = serde_json::json!({
            "seq": 10,
            "type": "response",
            "request_seq": 100_000,
            "success": true,
            "command": "hotReload"
        });

        let request_seq = msg["request_seq"].as_i64().unwrap();
        assert!(SeqAllocator::is_injected(request_seq));

        let mut map = pending.lock().await;
        if let Some(sender) = map.remove(&request_seq) {
            sender.send(msg.clone()).unwrap();
        }

        let result = rx.await.unwrap();
        assert_eq!(result["command"], "hotReload");
        assert_eq!(result["success"], true);
    }

    #[tokio::test]
    async fn stdin_writer_drains_channel() {
        let (tx, rx) = tokio::sync::mpsc::channel(16);
        let mut output = Vec::new();

        tx.send(b"hello".to_vec()).await.unwrap();
        tx.send(b"world".to_vec()).await.unwrap();
        drop(tx);

        stdin_writer(rx, &mut output).await.unwrap();

        let mut reader = BufReader::new(Cursor::new(output));
        let msg1 = dap::read_message(&mut reader).await.unwrap();
        let msg2 = dap::read_message(&mut reader).await.unwrap();
        assert_eq!(msg1, b"hello");
        assert_eq!(msg2, b"world");
        assert!(dap::read_message(&mut reader).await.is_none());
    }

    #[tokio::test]
    async fn extract_vm_service_uri() {
        let state: SharedState = Arc::new(RwLock::new(AdapterState::default()));
        let event = serde_json::json!({
            "seq": 42,
            "type": "event",
            "event": "dart.debuggerUris",
            "body": {"vmServiceUri": "ws://127.0.0.1:8181/abc=/ws"}
        });
        extract_state(&event, &state).await;
        assert_eq!(
            state.read().await.vm_service_uri.as_deref(),
            Some("ws://127.0.0.1:8181/abc=/ws")
        );
    }

    #[tokio::test]
    async fn extract_ignores_unrelated_events() {
        let state: SharedState = Arc::new(RwLock::new(AdapterState::default()));
        let event = serde_json::json!({
            "seq": 1,
            "type": "event",
            "event": "stopped",
            "body": {"reason": "breakpoint"}
        });
        extract_state(&event, &state).await;
        assert!(state.read().await.vm_service_uri.is_none());
    }
}
