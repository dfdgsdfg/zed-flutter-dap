mod dap;
mod proxy;
mod seq;
mod socket;

use std::collections::HashMap;
use std::process::ExitCode;
use std::sync::Arc;

use tokio::process::Command;
use tokio::signal::unix::{signal, SignalKind};
use tokio::sync::{mpsc, Mutex, RwLock};

use proxy::AdapterState;
use seq::SeqAllocator;

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("Usage: dap-proxy <command> [args...]");
        return ExitCode::FAILURE;
    }

    let (cmd, cmd_args) = args.split_first().unwrap();

    // Spawn the child debug adapter
    let mut child = match Command::new(cmd)
        .args(cmd_args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::inherit())
        .spawn()
    {
        Ok(child) => child,
        Err(e) => {
            eprintln!("[dap-proxy] failed to spawn {cmd}: {e}");
            return ExitCode::FAILURE;
        }
    };

    let child_stdin = child.stdin.take().expect("child stdin piped");
    let child_stdout = child.stdout.take().expect("child stdout piped");
    let child_id = child.id();

    // Shared state
    let seq = Arc::new(SeqAllocator::new());
    let pending: proxy::PendingMap = Arc::new(Mutex::new(HashMap::new()));
    let state: proxy::SharedState = Arc::new(RwLock::new(AdapterState::default()));
    let (child_stdin_tx, child_stdin_rx) = mpsc::channel::<Vec<u8>>(256);

    // Socket path
    let sock_path = socket::socket_path();

    // Task 1: Zed stdin → child stdin channel
    let zed_tx = child_stdin_tx.clone();
    let zed_to_adapter = tokio::spawn(async move {
        let stdin = tokio::io::stdin();
        let _ = proxy::zed_to_adapter(stdin, zed_tx).await;
    });

    // Task 2: Child stdout → Zed stdout (with injected response routing + state capture)
    let adapter_pending = Arc::clone(&pending);
    let adapter_state = Arc::clone(&state);
    let adapter_to_zed = tokio::spawn(async move {
        let stdout = tokio::io::stdout();
        let _ = proxy::adapter_to_zed(child_stdout, stdout, adapter_pending, adapter_state).await;
    });

    // Task 3: Channel drain → child stdin writes
    let stdin_writer = tokio::spawn(async move {
        let _ = proxy::stdin_writer(child_stdin_rx, child_stdin).await;
    });

    // Task 4: Unix socket listener
    let socket_seq = Arc::clone(&seq);
    let socket_pending = Arc::clone(&pending);
    let socket_tx = child_stdin_tx.clone();
    let socket_path_clone = sock_path.clone();
    let socket_state = Arc::clone(&state);
    let socket_listener = tokio::spawn(async move {
        let _ = socket::listen(
            socket_path_clone,
            socket_seq,
            socket_tx,
            socket_pending,
            socket_state,
        )
        .await;
    });

    // Drop our copy of child_stdin_tx so the channel closes when zed_to_adapter
    // and socket_listener finish
    drop(child_stdin_tx);

    // Signal forwarding (SIGTERM, SIGINT → forward to child)
    let signal_child_id = child_id;
    tokio::spawn(async move {
        let mut sigterm = signal(SignalKind::terminate()).expect("signal handler");
        let mut sigint = signal(SignalKind::interrupt()).expect("signal handler");

        tokio::select! {
            _ = sigterm.recv() => {},
            _ = sigint.recv() => {},
        }

        if let Some(pid) = signal_child_id {
            unsafe {
                libc::kill(pid as i32, libc::SIGTERM);
            }
        }
    });

    // Wait for child to exit
    let status = child.wait().await;

    // Cleanup
    socket::cleanup(&sock_path);
    zed_to_adapter.abort();
    adapter_to_zed.abort();
    stdin_writer.abort();
    socket_listener.abort();

    match status {
        Ok(s) => {
            if s.success() {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(s.code().unwrap_or(1) as u8)
            }
        }
        Err(e) => {
            eprintln!("[dap-proxy] error waiting for child: {e}");
            ExitCode::FAILURE
        }
    }
}
