//! Application state for the Ephemera desktop client.
//!
//! Holds the embedded [`EphemeraNode`] and the JSON-RPC router. Shared
//! across all axum request handlers via `Arc<AppState>`.

use ephemera_config::NodeConfig;
use ephemera_node::api::build_router_with_network;
use ephemera_node::debug_log::DebugLogHandle;
use ephemera_node::rpc::Router;
use ephemera_node::rpc_auth::RpcAuth;
use ephemera_node::EphemeraNode;
use std::path::PathBuf;
use std::sync::Arc;

/// Application state shared across all HTTP handlers.
///
/// Constructed during application startup and wrapped in `Arc` for
/// thread-safe sharing with axum's handler extractors.
pub struct AppState {
    /// The embedded Ephemera node (owns networking, storage, crypto).
    #[allow(dead_code)]
    pub node: EphemeraNode,
    /// JSON-RPC method router built from the node's service container.
    pub router: Router,
    /// RPC authentication token manager for validating incoming requests.
    pub rpc_auth: RpcAuth,
}

impl AppState {
    /// Initialize the application state.
    ///
    /// Loads or creates configuration in `data_dir`, boots the embedded
    /// node, and builds the JSON-RPC router from the node's services.
    /// The `debug_log` handle is wired into both the node and the router
    /// so the in-app debug console can retrieve captured log entries.
    ///
    /// # Errors
    ///
    /// Returns an error if the node fails to initialize or start.
    pub async fn initialize(
        data_dir: PathBuf,
        debug_log: DebugLogHandle,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let mut config = NodeConfig::load_or_create(&data_dir)?;
        // Default P2P listen address for the desktop client.
        if config.listen_addr.is_none() {
            config.listen_addr = Some("0.0.0.0:9100".parse().expect("valid addr"));
        }

        let mut node = EphemeraNode::with_debug_log(config, debug_log.clone())?;
        node.start().await?;

        // Try auto-unlock from cached session key ("remember me" feature).
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

        let router = build_router_with_network(
            Arc::clone(node.services()),
            Some(debug_log),
        );
        let rpc_auth = node.rpc_auth().clone();

        Ok(Self {
            node,
            router,
            rpc_auth,
        })
    }
}
