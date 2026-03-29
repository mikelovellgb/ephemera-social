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

    let network = node.network().cloned();
    let router = build_router_with_network(
        Arc::clone(node.services()),
        network,
        Some(debug_log),
    );

    Ok(NodeState {
        router,
        node: Mutex::new(node),
    })
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
