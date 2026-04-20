//! Ephemera Tauri 2.x library crate.
//!
//! Provides the Tauri command handlers and application state used by both the
//! desktop binary and the Android/iOS mobile targets. The mobile targets call
//! [`run`] from their platform-specific entry points.

use ephemera_config::NodeConfig;
use ephemera_node::api::build_router_with_network;
use ephemera_node::debug_log::{DebugLogHandle, DebugLogLayer};
use ephemera_node::rpc::{JsonRpcRequest, JsonRpcResponse, Router};
use ephemera_node::EphemeraNode;
use serde_json::Value;
use std::sync::Arc;
use tauri::Manager;
use tokio::sync::{Mutex, OnceCell};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

/// Inner state that becomes available once the node finishes booting.
struct NodeState {
    /// JSON-RPC method router wired to the embedded node's services.
    router: Router,
    /// The embedded Ephemera node (behind a Mutex for shutdown access).
    #[allow(dead_code)]
    node: Mutex<EphemeraNode>,
}

/// Shared application state managed by Tauri.
///
/// Registered synchronously during `setup` so that Tauri commands can always
/// extract it. The inner [`OnceCell`] is populated asynchronously once the
/// node finishes booting.
pub struct AppState {
    inner: OnceCell<NodeState>,
}

// -----------------------------------------------------------------------
// Tauri commands
// -----------------------------------------------------------------------

/// JSON-RPC bridge command invoked from the frontend via `invoke('rpc', ...)`.
///
/// Receives a full JSON-RPC 2.0 request object, dispatches it through the
/// node's router, and returns the response. No HTTP or auth tokens needed --
/// the Tauri IPC channel is already trusted.
///
/// Returns a JSON-RPC error if the node has not finished booting yet.
#[tauri::command]
async fn rpc(
    request: Value,
    state: tauri::State<'_, AppState>,
) -> Result<Value, String> {
    let node_state = state
        .inner
        .get()
        .ok_or_else(|| "node is still starting up, please wait".to_string())?;

    let parsed: JsonRpcRequest = serde_json::from_value(request)
        .map_err(|e| format!("invalid JSON-RPC request: {e}"))?;

    tracing::debug!(method = %parsed.method, "tauri rpc");

    let response: JsonRpcResponse = node_state.router.dispatch(parsed).await;

    serde_json::to_value(&response)
        .map_err(|e| format!("failed to serialize RPC response: {e}"))
}

// -----------------------------------------------------------------------
// App builder
// -----------------------------------------------------------------------

/// Build and run the Tauri application.
///
/// Initializes tracing with both the fmt layer (for console/logcat output)
/// and the [`DebugLogLayer`] (for the in-app debug console). This single
/// entry point is used by both `main.rs` (desktop) and the mobile targets.
///
/// # Errors
///
/// Returns an error if the Tauri runtime or the embedded node fail to start.
pub fn run() {
    // Create the shared debug log handle BEFORE tracing init so ALL startup
    // logs (including Iroh init failures) are captured for the debug console.
    let debug_log = DebugLogHandle::new();

    // Initialize tracing with BOTH the fmt layer and the DebugLogLayer.
    // This works for desktop (stdout) and mobile (logcat) alike.
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));

    tracing_subscriber::registry()
        .with(env_filter)
        .with(tracing_subscriber::fmt::layer())
        .with(DebugLogLayer::new(debug_log.clone()))
        .init();

    tracing::info!("Ephemera Tauri starting");

    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![rpc])
        .setup(move |app| {
            // Register the state container immediately so that Tauri commands
            // can always extract it. The inner OnceCell starts empty and is
            // populated once the node finishes booting.
            let app_state = AppState {
                inner: OnceCell::new(),
            };
            app.manage(app_state);

            // Determine the data directory using Tauri's path resolver,
            // which maps to the correct location on each platform:
            //   - Windows: %APPDATA%/social.ephemera.app
            //   - macOS:   ~/Library/Application Support/social.ephemera.app
            //   - Linux:   $XDG_DATA_HOME/social.ephemera.app
            //   - Android: /data/data/social.ephemera.app/files
            //   - iOS:     <sandbox>/Documents
            let data_dir = app
                .path()
                .app_data_dir()
                .expect("failed to resolve app data directory");

            std::fs::create_dir_all(&data_dir)
                .expect("failed to create app data directory");

            tracing::info!(data_dir = %data_dir.display(), "ephemera data directory");

            // Boot the embedded node on the async runtime.
            let handle = app.handle().clone();
            let debug_log_inner = debug_log.clone();
            tauri::async_runtime::spawn(async move {
                match boot_node(data_dir, debug_log_inner).await {
                    Ok(node_state) => {
                        let state: tauri::State<'_, AppState> = handle.state();
                        let _ = state.inner.set(node_state);
                        tracing::info!("ephemera node ready");
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "failed to boot ephemera node");
                    }
                }
            });

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error running ephemera tauri app");
}

/// Initialize the node and return the [`NodeState`].
async fn boot_node(
    data_dir: std::path::PathBuf,
    debug_log: DebugLogHandle,
) -> Result<NodeState, Box<dyn std::error::Error>> {
    let mut config = NodeConfig::load_or_create(&data_dir)?;

    // Default P2P listen address for the desktop/mobile client.
    if config.listen_addr.is_none() {
        config.listen_addr = Some("0.0.0.0:9100".parse().expect("valid addr"));
    }

    let mut node = EphemeraNode::with_debug_log(config, debug_log.clone())?;
    node.start().await?;

    // Try auto-unlock from cached session key ("remember me" feature).
    // If a session key exists, this decrypts the keystore and starts the
    // network without requiring the user to re-enter their passphrase.
    match node.services().identity.auto_unlock().await {
        Ok(result) => {
            if result.get("auto_unlocked").and_then(|v| v.as_bool()) == Some(true) {
                tracing::info!("auto-unlocked from cached session key");
                if let Err(e) = node.services().start_network().await {
                    tracing::warn!(error = %e, "failed to start network after auto-unlock");
                }
            }
        }
        Err(e) => tracing::debug!(error = %e, "auto-unlock not available"),
    }

    // Spawn a 30-second heartbeat task to monitor network connectivity.
    // On mobile, this detects when the app resumes from background and
    // the network connection has dropped.
    if let Some(net) = node.network() {
        let net = Arc::clone(net);
        tokio::spawn(async move {
            connectivity_heartbeat(net).await;
        });
        tracing::info!("spawned connectivity heartbeat task (30s interval)");
    }

    let router = build_router_with_network(
        Arc::clone(node.services()),
        Some(debug_log),
    );

    let node_state = NodeState {
        router,
        node: Mutex::new(node),
    };

    // In debug builds, spawn a lightweight HTTP test server on port 3520
    // so integration tests can reach the RPC router via `adb forward`.
    #[cfg(debug_assertions)]
    {
        let router_clone = node_state.router.clone();
        tokio::spawn(async move {
            start_test_http_server(router_clone).await;
        });
    }

    Ok(node_state)
}

/// Debug-only HTTP server for integration testing via ADB port forwarding.
///
/// Listens on `0.0.0.0:3520` and accepts JSON-RPC 2.0 requests via POST.
/// No authentication — this is only compiled in debug builds.
#[cfg(debug_assertions)]
async fn start_test_http_server(router: Router) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let addr = "0.0.0.0:3520";
    let listener = match TcpListener::bind(addr).await {
        Ok(l) => {
            tracing::info!("debug test HTTP server listening on {addr}");
            l
        }
        Err(e) => {
            tracing::warn!(error = %e, "failed to start debug test server on {addr}");
            return;
        }
    };

    loop {
        let (mut stream, _) = match listener.accept().await {
            Ok(s) => s,
            Err(_) => continue,
        };
        let router = router.clone();

        tokio::spawn(async move {
            let mut buf = vec![0u8; 65536];
            let n = match stream.read(&mut buf).await {
                Ok(n) if n > 0 => n,
                _ => return,
            };
            let request_str = String::from_utf8_lossy(&buf[..n]);

            // Find the JSON body after the empty line in HTTP request
            let body = request_str
                .find("\r\n\r\n")
                .map(|i| &request_str[i + 4..])
                .unwrap_or(&request_str);

            let response_json = if let Ok(req) = serde_json::from_str::<
                ephemera_node::rpc::JsonRpcRequest,
            >(body)
            {
                let resp = router.dispatch(req).await;
                serde_json::to_string(&resp).unwrap_or_default()
            } else {
                r#"{"jsonrpc":"2.0","error":{"code":-32700,"message":"parse error"},"id":null}"#
                    .to_string()
            };

            let http_response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\n\r\n{}",
                response_json.len(),
                response_json
            );
            let _ = stream.write_all(http_response.as_bytes()).await;
        });
    }
}

/// Background task that checks network connectivity every 30 seconds.
///
/// Logs the current peer count and transport status. On mobile platforms
/// (Android/iOS) the OS may suspend the process or kill network sockets
/// when the app goes to background. This heartbeat detects when
/// connectivity drops so we can log the state for debugging.
///
/// The Iroh transport handles reconnection to relay servers internally,
/// but this heartbeat gives us visibility into the connectivity state.
async fn connectivity_heartbeat(network: Arc<ephemera_node::network::NetworkSubsystem>) {
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
    // Don't pile up missed ticks if we're suspended/delayed.
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        interval.tick().await;

        let peer_count = network.peer_count();

        if peer_count == 0 {
            tracing::warn!(
                "heartbeat: no connected peers -- gossip messages will not be delivered"
            );
        } else {
            tracing::info!(
                peer_count = peer_count,
                "heartbeat: network OK (Iroh QUIC)"
            );
        }
    }
}

// -----------------------------------------------------------------------
// Mobile entry point (Tauri 2.x)
// -----------------------------------------------------------------------

/// Mobile entry point using Tauri's macro.
/// This generates the correct JNI symbols for Android and Swift bridge for iOS.
///
/// On mobile, `run()` handles tracing initialization and the rest of the
/// application lifecycle.
#[cfg(mobile)]
#[tauri::mobile_entry_point]
fn mobile_main() {
    run();
}
