//! [`TestNode`]: a self-contained Ephemera node for testing.

use ephemera_config::NodeConfig;
use ephemera_crypto::{NodeIdentity, SigningKeyPair};
use ephemera_events::EventBus;
use ephemera_node::rpc::{JsonRpcRequest, JsonRpcResponse, Router};
use ephemera_node::services::ServiceContainer;
use ephemera_node::EphemeraNode;
use serde_json::Value;
use std::sync::Arc;
use tempfile::TempDir;

/// A self-contained Ephemera node backed by a temporary directory.
///
/// The temp directory and all data within it are deleted when the
/// `TestNode` is dropped. Every `TestNode` gets a fresh identity,
/// config, and storage.
pub struct TestNode {
    /// The live node instance.
    node: EphemeraNode,
    /// The JSON-RPC router wired to this node's services.
    router: Router,
    /// The node's network identity (Ed25519 keypair).
    identity: NodeIdentity,
    /// Temporary directory — cleaned up on drop.
    _temp_dir: TempDir,
    /// A monotonically increasing RPC request ID.
    next_rpc_id: std::sync::atomic::AtomicU64,
}

impl TestNode {
    /// Create a new test node with default test configuration.
    ///
    /// The node is constructed and validated but **not** started.
    /// Call [`start`](Self::start) to run the full startup sequence.
    ///
    /// # Errors
    ///
    /// Returns an error if the temp directory cannot be created or
    /// the node fails to initialize.
    pub async fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let temp_dir = TempDir::new()?;
        let config = Self::test_config(temp_dir.path());
        let identity = NodeIdentity::generate();

        let node = EphemeraNode::new(config.clone())?;

        let event_bus = EventBus::new();
        let services = Arc::new(ServiceContainer::new(&config, event_bus)?);
        let router = ephemera_node::api::build_router(services);

        Ok(Self {
            node,
            router,
            identity,
            _temp_dir: temp_dir,
            next_rpc_id: std::sync::atomic::AtomicU64::new(1),
        })
    }

    /// Create a new test node and immediately start it.
    ///
    /// Equivalent to `TestNode::new()` followed by `node.start()`.
    ///
    /// # Errors
    ///
    /// Returns an error if creation or startup fails.
    pub async fn started() -> Result<Self, Box<dyn std::error::Error>> {
        let mut node = Self::new().await?;
        node.start().await?;
        Ok(node)
    }

    /// Create a new test node, start it, and create a test identity
    /// (unlocking the keystore so that post creation and other
    /// identity-dependent operations work out of the box).
    ///
    /// This is the recommended way to create test nodes for most tests.
    ///
    /// # Errors
    ///
    /// Returns an error if creation, startup, or identity setup fails.
    pub async fn ready() -> Result<Self, Box<dyn std::error::Error>> {
        let mut node = Self::new().await?;
        node.start().await?;
        node.ensure_identity().await?;
        Ok(node)
    }

    /// Ensure the node has an active identity. If none exists, creates
    /// one with a default test passphrase.
    ///
    /// # Errors
    ///
    /// Returns an error if identity creation fails.
    pub async fn ensure_identity(&self) -> Result<(), Box<dyn std::error::Error>> {
        let active = self
            .node_rpc("identity.get_active", serde_json::json!({}))
            .await;
        if active.error.is_some() {
            // No active identity — create one.
            let result = self
                .create_identity("test-passphrase")
                .await
                .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
            tracing::debug!(pubkey = ?result.get("pseudonym_pubkey"), "test identity created");
        }
        Ok(())
    }

    /// Start the node (networking, GC, background tasks).
    ///
    /// # Errors
    ///
    /// Returns an error if the startup sequence fails.
    pub async fn start(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        self.node.start().await?;
        Ok(())
    }

    /// Gracefully shut down the node.
    ///
    /// # Errors
    ///
    /// Returns an error if shutdown signaling fails.
    pub async fn shutdown(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        self.node.shutdown().await?;
        Ok(())
    }

    /// Whether the node is currently running.
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.node.is_running()
    }

    /// Access the underlying `EphemeraNode`.
    #[must_use]
    pub fn inner(&self) -> &EphemeraNode {
        &self.node
    }

    /// The node's network identity.
    #[must_use]
    pub fn identity(&self) -> &NodeIdentity {
        &self.identity
    }

    /// The path to the node's data directory (temporary).
    #[must_use]
    pub fn data_dir(&self) -> &std::path::Path {
        self.node.config().data_dir.as_path()
    }

    /// Access the event bus for subscribing to node events.
    #[must_use]
    pub fn event_bus(&self) -> &EventBus {
        self.node.event_bus()
    }

    /// Access the node configuration.
    #[must_use]
    pub fn config(&self) -> &NodeConfig {
        self.node.config()
    }

    // -- High-level helpers --------------------------------------------------

    /// Create a text post via the JSON-RPC API.
    ///
    /// Returns the full JSON-RPC result object, which includes
    /// `content_hash`, `created_at`, `expires_at`, etc.
    ///
    /// # Errors
    ///
    /// Returns an error if the RPC call fails.
    pub async fn create_post(&self, body: &str) -> Result<Value, String> {
        self.create_post_with_ttl(body, None).await
    }

    /// Create a text post with an explicit TTL (in seconds).
    ///
    /// # Errors
    ///
    /// Returns an error if the RPC call fails.
    pub async fn create_post_with_ttl(
        &self,
        body: &str,
        ttl_seconds: Option<u64>,
    ) -> Result<Value, String> {
        let mut params = serde_json::json!({ "body": body });
        if let Some(ttl) = ttl_seconds {
            params["ttl_seconds"] = serde_json::json!(ttl);
        }
        let resp = self.node_rpc("posts.create", params).await;
        resp.result.ok_or_else(|| {
            resp.error
                .map(|e| e.message)
                .unwrap_or_else(|| "unknown error".into())
        })
    }

    /// Read the connections feed via the JSON-RPC API.
    ///
    /// # Errors
    ///
    /// Returns an error if the RPC call fails.
    pub async fn get_feed(&self) -> Result<Value, String> {
        let resp = self
            .node_rpc("feed.connections", serde_json::json!({}))
            .await;
        resp.result.ok_or_else(|| {
            resp.error
                .map(|e| e.message)
                .unwrap_or_else(|| "unknown error".into())
        })
    }

    /// Create a new identity via the JSON-RPC API.
    ///
    /// # Errors
    ///
    /// Returns an error if the RPC call fails.
    pub async fn create_identity(&self, passphrase: &str) -> Result<Value, String> {
        let resp = self
            .node_rpc(
                "identity.create",
                serde_json::json!({ "passphrase": passphrase }),
            )
            .await;
        resp.result.ok_or_else(|| {
            resp.error
                .map(|e| e.message)
                .unwrap_or_else(|| "unknown error".into())
        })
    }

    /// Send a raw JSON-RPC request and return the response.
    pub async fn node_rpc(&self, method: &str, params: Value) -> JsonRpcResponse {
        let id = self
            .next_rpc_id
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: method.to_string(),
            params,
            id: Value::Number(id.into()),
        };
        self.router.dispatch(request).await
    }

    /// Create two connected test nodes suitable for P2P testing.
    ///
    /// Both nodes are started and share no state (separate temp dirs,
    /// separate identities). They are "connected" at the service level
    /// (actual network-layer connection is a placeholder until transport
    /// is implemented).
    ///
    /// # Errors
    ///
    /// Returns an error if either node fails to start.
    pub async fn pair() -> Result<(Self, Self), Box<dyn std::error::Error>> {
        let node_a = Self::ready().await?;
        let node_b = Self::ready().await?;

        tracing::info!(
            a_peer = %node_a.identity().node_id(),
            b_peer = %node_b.identity().node_id(),
            "test node pair created"
        );

        Ok((node_a, node_b))
    }

    // -- Private helpers -----------------------------------------------------

    /// Build a test-friendly `NodeConfig` rooted in the given temp directory.
    fn test_config(data_dir: &std::path::Path) -> NodeConfig {
        let mut config = NodeConfig::default_for(data_dir);
        // Use smaller storage limits for faster tests.
        config.storage.max_storage_bytes = 64 * 1024 * 1024; // 64 MiB
        // Fast GC for expiry tests (minimum allowed is 5s).
        config.storage.gc_interval_secs = 5;
        // Use a random port so parallel tests don't fight over the same port.
        config.listen_addr = Some("127.0.0.1:0".parse().unwrap());
        config
    }
}

/// Produce a fresh `SigningKeyPair` for ad-hoc signing in tests.
///
/// This is a convenience re-export so test code does not need to depend
/// on `ephemera-crypto` directly.
#[must_use]
pub fn generate_signing_keypair() -> SigningKeyPair {
    SigningKeyPair::generate()
}

#[cfg(test)]
#[path = "node_tests.rs"]
mod tests;
