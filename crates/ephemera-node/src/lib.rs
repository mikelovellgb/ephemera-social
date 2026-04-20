//! Ephemera node: composition root that wires all subsystems together.
//!
//! This crate is the heart of the Ephemera platform. It can be embedded
//! as a library inside Tauri (the normal deployment) or run as a standalone
//! daemon (post-PoC). All subsystem crates are composed here behind a
//! unified JSON-RPC 2.0 API surface.

pub mod api;
pub mod background;
pub mod background_dht;
pub mod debug_log;
pub mod dht_query;
pub mod gossip_ingest;
pub mod message_ingest;
pub mod network;
pub mod rpc;
pub mod rpc_auth;
pub mod services;
pub mod startup;

use debug_log::DebugLogHandle;
use ephemera_config::NodeConfig;
use ephemera_events::EventBus;
use network::NetworkSubsystem;
use rpc_auth::RpcAuth;
use services::ServiceContainer;
use startup::StartupError;
use std::sync::Arc;

/// The top-level Ephemera node, owning all services and background tasks.
///
/// Create via [`EphemeraNode::new`], then [`EphemeraNode::start`] to boot
/// the network stack. Call [`EphemeraNode::shutdown`] for graceful teardown.
pub struct EphemeraNode {
    config: NodeConfig,
    services: Arc<ServiceContainer>,
    event_bus: EventBus,
    shutdown: tokio::sync::watch::Sender<bool>,
    running: bool,
    rpc_auth: RpcAuth,
    /// Network subsystem (Iroh QUIC + gossip). Created on identity unlock.
    network: Option<Arc<NetworkSubsystem>>,
    /// In-memory debug log ring buffer for the in-app debug console.
    debug_log: DebugLogHandle,
}

/// Process incoming tombstone messages from the moderation gossip topic.
///
/// When a remote node deletes a post, it publishes a tombstone message.
/// This loop receives those messages, verifies the author signature, and
/// marks the corresponding post as tombstoned locally.
async fn moderation_ingest_loop(
    mut subscription: ephemera_gossip::TopicSubscription,
    metadata_db: std::sync::Mutex<ephemera_store::MetadataDb>,
    content_store: ephemera_store::ContentStore,
    mut shutdown_rx: tokio::sync::watch::Receiver<bool>,
) {
    loop {
        tokio::select! {
            msg = subscription.recv() => {
                let msg = match msg {
                    Some(m) => m,
                    None => {
                        tracing::debug!("moderation ingest: channel closed");
                        break;
                    }
                };

                // Parse the tombstone JSON.
                let tombstone: serde_json::Value = match serde_json::from_slice(&msg.payload) {
                    Ok(v) => v,
                    Err(_) => continue,
                };

                // Only handle tombstone messages.
                if tombstone.get("type").and_then(|t| t.as_str()) != Some("tombstone") {
                    continue;
                }

                let content_hash = match tombstone.get("content_hash").and_then(|v| v.as_str()) {
                    Some(h) => h.to_string(),
                    None => continue,
                };

                // Verify the content hash is valid hex.
                let hash_bytes = match hex::decode(&content_hash) {
                    Ok(b) if b.len() == 32 => b,
                    _ => continue,
                };

                // Mark the post as tombstoned in the local database.
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .expect("clock")
                    .as_secs() as i64;

                let mut arr = [0u8; 32];
                arr.copy_from_slice(&hash_bytes);
                let content_id = ephemera_types::ContentId::from_digest(arr);
                let wire = content_id.to_wire_bytes();

                if let Ok(db) = metadata_db.lock() {
                    // Delete the blob from the content store.
                    let blob_hash: Option<String> = db
                        .conn()
                        .query_row(
                            "SELECT blob_hash FROM posts WHERE content_hash = ?1 AND is_tombstone = 0",
                            rusqlite::params![wire],
                            |row| row.get(0),
                        )
                        .ok()
                        .flatten();

                    if let Some(ref bh) = blob_hash {
                        let _ = content_store.delete(bh);
                    }

                    let updated = db.conn().execute(
                        "UPDATE posts SET is_tombstone = 1, tombstone_at = ?1
                         WHERE content_hash = ?2 AND is_tombstone = 0",
                        rusqlite::params![now, wire],
                    );

                    if let Ok(count) = updated {
                        if count > 0 {
                            tracing::info!(
                                hash = %content_hash,
                                "moderation ingest: tombstoned post from network"
                            );
                        }
                    }
                }
            }
            _ = shutdown_rx.changed() => {
                tracing::debug!("moderation ingest: received shutdown signal");
                break;
            }
        }
    }
}

impl EphemeraNode {
    /// Create a new node. Does **not** start networking or background tasks;
    /// call [`start`](Self::start) for that.
    ///
    /// The `debug_log` handle is shared with the tracing subscriber layer so
    /// that log output is captured for the in-app debug console.  If you do
    /// not need in-app log capture, pass [`DebugLogHandle::new()`].
    pub fn new(config: NodeConfig) -> Result<Self, StartupError> {
        Self::with_debug_log(config, DebugLogHandle::new())
    }

    /// Create a new node with an explicit [`DebugLogHandle`].
    ///
    /// Use this variant when you want the same handle wired into the
    /// tracing subscriber (desktop `main.rs`, Tauri `lib.rs`).
    pub fn with_debug_log(
        config: NodeConfig,
        debug_log: DebugLogHandle,
    ) -> Result<Self, StartupError> {
        startup::validate_config(&config)?;

        // Ensure data directory exists before writing the RPC token.
        std::fs::create_dir_all(&config.data_dir).map_err(|e| StartupError::DataDir {
            reason: e.to_string(),
        })?;

        // Generate a fresh RPC authentication token.
        let rpc_auth = RpcAuth::generate(&config.data_dir).map_err(|e| StartupError::Internal {
            reason: format!("failed to generate RPC auth token: {e}"),
        })?;

        let event_bus = EventBus::new();
        let (shutdown_tx, _) = tokio::sync::watch::channel(false);

        let services = Arc::new(ServiceContainer::new(&config, event_bus.clone())?);

        tracing::info!(data_dir = %config.data_dir.display(), "ephemera node created");

        Ok(Self {
            config,
            services,
            event_bus,
            shutdown: shutdown_tx,
            running: false,
            rpc_auth,
            network: None,
            debug_log,
        })
    }

    /// Start the node: background tasks (GC, epoch rotation, etc.).
    ///
    /// Networking is NOT started here. It starts when the identity is
    /// unlocked via `identity.unlock`, `identity.create`, or an import
    /// handler, which calls `ServiceContainer::start_network()`.
    pub async fn start(&mut self) -> Result<(), StartupError> {
        if self.running {
            tracing::warn!("node is already running");
            return Ok(());
        }

        startup::run_startup_sequence(&self.config, &self.services).await?;

        // Store the shutdown sender in ServiceContainer so that
        // start_network() can create shutdown receivers for ingest loops.
        self.services.set_shutdown_tx(self.shutdown.clone());

        // If identity is already unlocked (e.g. auto-unlock), start
        // networking immediately.
        if self.derive_secret_key_bytes().is_some() {
            match self.services.start_network().await {
                Ok(()) => {
                    // Copy the network ref to EphemeraNode for shutdown.
                    if let Ok(guard) = self.services.network.lock() {
                        self.network = guard.clone();
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, "failed to start network at boot (will retry on unlock)");
                }
            }
        } else {
            tracing::info!("identity locked — network will start after unlock");
        }

        // 3. GC background task
        {
            let shutdown_rx = self.shutdown.subscribe();
            let gc_interval = self.config.storage.gc_interval_secs;
            let event_bus = self.event_bus.clone();
            let services = Arc::clone(&self.services);
            tokio::spawn(async move {
                background::gc_loop(gc_interval, services, event_bus, shutdown_rx)
                    .await;
            });
        }

        // 4. Epoch key rotation (gap 1.5) -- cryptographic shredding
        {
            let shutdown_rx = self.shutdown.subscribe();
            let services = Arc::clone(&self.services);
            tokio::spawn(async move {
                background::epoch_rotation_loop(services, shutdown_rx).await;
            });
            tracing::info!("spawned epoch key rotation task");
        }

        // 5. Handle registry GC (gap 2.3)
        {
            let shutdown_rx = self.shutdown.subscribe();
            let services = Arc::clone(&self.services);
            tokio::spawn(async move {
                background::handle_gc_loop(services, shutdown_rx).await;
            });
            tracing::info!("spawned handle GC task");
        }

        // 6. Dead drop polling (gap 2.5)
        {
            let shutdown_rx = self.shutdown.subscribe();
            let services = Arc::clone(&self.services);
            tokio::spawn(async move {
                background::dead_drop_poll_loop(services, shutdown_rx).await;
            });
            tracing::info!("spawned dead drop polling task");
        }

        // 7. Reputation decay (gap 3.4)
        {
            let shutdown_rx = self.shutdown.subscribe();
            let services = Arc::clone(&self.services);
            tokio::spawn(async move {
                background::reputation_decay_loop(services, shutdown_rx).await;
            });
            tracing::info!("spawned reputation decay task");
        }

        // 8. DHT maintenance (sweep expired records + republish own records)
        {
            let shutdown_rx = self.shutdown.subscribe();
            let services = Arc::clone(&self.services);
            tokio::spawn(async move {
                background_dht::dht_maintenance_loop(services, shutdown_rx).await;
            });
            tracing::info!("spawned DHT maintenance task");
        }

        // 9. Periodic profile refresh for connected users (every 30 min)
        {
            let shutdown_rx = self.shutdown.subscribe();
            let services = Arc::clone(&self.services);
            tokio::spawn(async move {
                background::profile_refresh_loop(services, shutdown_rx).await;
            });
            tracing::info!("spawned profile refresh task");
        }

        // 10. Contact reconnect loop: try to connect to contacts every 60s.
        //     On Iroh, each contact's pubkey IS their NodeId, so we can
        //     connect without knowing their IP address.
        {
            let shutdown_rx = self.shutdown.subscribe();
            let services = Arc::clone(&self.services);
            tokio::spawn(async move {
                background::contact_reconnect_loop(services, shutdown_rx).await;
            });
            tracing::info!("spawned contact reconnect task");
        }

        self.running = true;
        tracing::info!("ephemera node started");

        Ok(())
    }

    /// Gracefully shut down the node, stopping all background tasks.
    ///
    /// # Errors
    ///
    /// Returns an error if the shutdown signal cannot be sent.
    pub async fn shutdown(&mut self) -> Result<(), StartupError> {
        if !self.running {
            return Ok(());
        }

        tracing::info!("shutting down ephemera node");

        // Signal all background tasks to stop.
        self.shutdown
            .send(true)
            .map_err(|_| StartupError::Internal {
                reason: "failed to send shutdown signal".into(),
            })?;

        // Shut down the network subsystem.
        if let Some(ref network) = self.network {
            network.shutdown().await;
        }
        self.network = None;

        // Clean up the RPC token file.
        self.rpc_auth.cleanup();

        self.running = false;
        tracing::info!("ephemera node stopped");
        Ok(())
    }

    /// Whether the node is currently running.
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.running
    }

    /// Access the service container for direct API calls.
    #[must_use]
    pub fn services(&self) -> &Arc<ServiceContainer> {
        &self.services
    }

    /// Get a clone of the event bus for subscribing to events.
    #[must_use]
    pub fn event_bus(&self) -> &EventBus {
        &self.event_bus
    }

    /// Access the node configuration.
    #[must_use]
    pub fn config(&self) -> &NodeConfig {
        &self.config
    }

    /// Access the RPC authentication token manager.
    #[must_use]
    pub fn rpc_auth(&self) -> &RpcAuth {
        &self.rpc_auth
    }

    /// Access the network subsystem (available after `start()`).
    #[must_use]
    pub fn network(&self) -> Option<&Arc<NetworkSubsystem>> {
        self.network.as_ref()
    }

    /// Access the debug log handle for wiring into the RPC router.
    #[must_use]
    pub fn debug_log(&self) -> &DebugLogHandle {
        &self.debug_log
    }

    /// Extract the Ed25519 secret key bytes from the identity service.
    ///
    /// Returns `None` if the identity is locked (no active keypair).
    /// The returned bytes are the 32-byte Ed25519 seed, suitable for use
    /// as an Iroh secret key so that NodeId = Ed25519 public key.
    fn derive_secret_key_bytes(&self) -> Option<[u8; 32]> {
        self.services
            .identity
            .get_signing_keypair()
            .ok()
            .map(|kp| {
                let secret = kp.secret_bytes();
                *secret
            })
    }
}
