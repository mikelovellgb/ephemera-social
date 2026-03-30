//! Service container: holds all domain services initialized from config.
//!
//! The [`ServiceContainer`] is the dependency injection root. It owns
//! instances of every domain service and provides access for the API layer.

pub mod dht;
pub mod error;
mod feed;
pub mod handle;
pub mod identity;
pub mod media;
pub mod notifications;
mod post;
mod social;

pub use dht::DhtNodeService;
pub use feed::FeedService;
pub use handle::HandleService;
pub use identity::IdentityService;
pub use media::MediaFile;
pub use notifications::NotificationService;
pub use post::PostService;
pub use social::{MessageService, ModerationService, ProfileService, SocialService};

use crate::startup::StartupError;
use ephemera_abuse::{FingerprintStore, RateLimiter, ReputationScore};
use ephemera_config::NodeConfig;
use ephemera_crypto::EpochKeyManager;
use ephemera_dht::routing::RoutingTable;
use ephemera_dht::storage::DhtStorage;
use ephemera_dht::DhtConfig;
use ephemera_events::EventBus;
use ephemera_mod::{ContentFilter, ReportService};
use ephemera_social::store::SqliteSocialServices;
use ephemera_social::HandleRegistry;
use ephemera_store::{ContentStore, GarbageCollector, MetadataDb};
use ephemera_types::{IdentityKey, NodeId};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Instant;

/// Holds all domain services, initialized during node construction.
pub struct ServiceContainer {
    started_at: Instant,
    /// Internal event bus for broadcasting node events.
    pub event_bus: EventBus,
    /// Post service: create, get, delete, list posts.
    pub posts: PostService,
    /// Feed service: assemble feeds from various sources.
    pub feed: FeedService,
    /// Social service: connections, follows, reactions.
    pub social: SocialService,
    /// Message service: encrypted direct messages.
    pub messages: MessageService,
    /// Identity service: keystore, pseudonym management.
    pub identity: IdentityService,
    /// Profile service: get/update user profiles.
    pub profiles: ProfileService,
    /// Moderation service: report, block, mute.
    pub moderation: ModerationService,
    /// Garbage collector for expired content.
    gc: GarbageCollector,
    /// Content blob store.
    content_store: ContentStore,
    /// SQLite metadata database.
    pub metadata_db: Mutex<MetadataDb>,
    /// Per-identity rate limiter.
    pub rate_limiter: Mutex<RateLimiter>,
    /// Per-identity reputation tracker.
    pub reputation: Mutex<HashMap<IdentityKey, ReputationScore>>,
    /// SimHash near-duplicate content detector.
    pub fingerprint_store: Mutex<FingerprintStore>,
    /// Content moderation filter (blocklist + heuristics).
    pub content_filter: Mutex<ContentFilter>,
    /// Epoch key manager for cryptographic shredding.
    pub epoch_key_manager: Mutex<Option<EpochKeyManager>>,
    /// Handle registry for human-readable @usernames.
    pub handle_registry: Mutex<HandleRegistry>,
    /// DHT record storage (TTL-aware, Kademlia-style).
    pub dht_storage: Mutex<DhtStorage>,
    /// DHT routing table (256 k-buckets, XOR metric).
    pub dht_routing: Mutex<RoutingTable>,
    /// DHT configuration parameters.
    pub dht_config: DhtConfig,
    /// Network subsystem reference, set after `EphemeraNode::start()`.
    /// Used by services that need to publish to gossip (e.g. message delivery).
    pub network: Mutex<Option<std::sync::Arc<crate::network::NetworkSubsystem>>>,
    /// Path to the content blob store directory (needed to re-open stores
    /// when re-spawning gossip ingest loops after a network upgrade).
    content_path: PathBuf,
    /// Path to the SQLite metadata database (needed to open fresh connections
    /// for re-spawned ingest loops).
    metadata_db_path: PathBuf,
    /// Shutdown signal sender. Cloned from `EphemeraNode` after `start()` so
    /// that `upgrade_network_to_iroh()` can hand shutdown receivers to newly
    /// spawned ingest loops.
    shutdown_tx: Mutex<Option<tokio::sync::watch::Sender<bool>>>,
    /// JoinHandles for the gossip ingest loops (public_feed, dm_delivery,
    /// moderation). Stored so they can be aborted when the network is
    /// upgraded from TCP to Iroh and new loops must be spawned.
    ingest_handles: Mutex<Vec<tokio::task::JoinHandle<()>>>,
}

impl ServiceContainer {
    /// Create a new service container from configuration.
    pub fn new(config: &NodeConfig, event_bus: EventBus) -> Result<Self, StartupError> {
        tracing::debug!(data_dir = %config.data_dir.display(), "initializing services");

        let content_path = config.content_path();
        std::fs::create_dir_all(&content_path).map_err(|e| StartupError::Storage {
            reason: format!("failed to create content dir: {e}"),
        })?;

        let metadata_dir = config.data_dir.join("metadata");
        std::fs::create_dir_all(&metadata_dir).map_err(|e| StartupError::Storage {
            reason: format!("failed to create metadata dir: {e}"),
        })?;

        let content_store =
            ContentStore::open(&content_path).map_err(|e| StartupError::Storage {
                reason: format!("failed to open content store: {e}"),
            })?;

        let metadata_db_path = config.metadata_db_path();
        let metadata_db =
            MetadataDb::open(&metadata_db_path).map_err(|e| StartupError::Storage {
                reason: format!("failed to open metadata db: {e}"),
            })?;

        // Open a second connection to the same SQLite database for the social services.
        // SQLite WAL mode supports concurrent readers, so this is safe.
        let social_db = MetadataDb::open(&metadata_db_path).map_err(|e| StartupError::Storage {
            reason: format!("failed to open social metadata db: {e}"),
        })?;

        // Open a third connection for the message service.
        let message_db =
            MetadataDb::open(&metadata_db_path).map_err(|e| StartupError::Storage {
                reason: format!("failed to open message metadata db: {e}"),
            })?;

        let keystore_path = config.keystore_path();
        let keystore_dir = config.data_dir.join("keystore");
        let _ = std::fs::create_dir_all(&keystore_dir);

        let gc = GarbageCollector::with_defaults();

        let identity = IdentityService {
            keystore_path,
            active_keypair: Mutex::new(None),
            active_index: Mutex::new(0),
            master_secret: Mutex::new(None),
            pseudonym_count: Mutex::new(0),
            device_manager: Mutex::new(ephemera_crypto::DeviceManager::new()),
        };

        let social = SocialService {
            social_services: SqliteSocialServices::new(social_db),
        };

        let messages = MessageService {
            metadata_db: Mutex::new(message_db),
        };

        let moderation = ModerationService {
            report_service: Mutex::new(ReportService::new()),
        };

        // Initialize DHT subsystem with a placeholder node ID.
        // The real node ID (derived from identity) will be set when the
        // identity is created or unlocked, via `update_dht_node_id`.
        let dht_config = DhtConfig::default();
        let placeholder_node_id = NodeId::from_bytes([0u8; 32]);
        let dht_routing = RoutingTable::new(placeholder_node_id, dht_config.k);
        let dht_storage = DhtStorage::new(&dht_config);

        Ok(Self {
            started_at: Instant::now(),
            event_bus,
            posts: PostService,
            feed: FeedService,
            social,
            messages,
            identity,
            profiles: ProfileService,
            moderation,
            gc,
            content_store,
            metadata_db: Mutex::new(metadata_db),
            rate_limiter: Mutex::new(RateLimiter::new()),
            reputation: Mutex::new(HashMap::new()),
            fingerprint_store: Mutex::new(FingerprintStore::new()),
            content_filter: Mutex::new(ContentFilter::empty()),
            epoch_key_manager: Mutex::new(None),
            handle_registry: Mutex::new(HandleRegistry::new()),
            dht_storage: Mutex::new(dht_storage),
            dht_routing: Mutex::new(dht_routing),
            dht_config,
            network: Mutex::new(None),
            content_path,
            metadata_db_path,
            shutdown_tx: Mutex::new(None),
            ingest_handles: Mutex::new(Vec::new()),
        })
    }

    /// Set the network subsystem reference. Called by `EphemeraNode::start()`
    /// after the network is created so services can publish to gossip.
    pub fn set_network(&self, network: std::sync::Arc<crate::network::NetworkSubsystem>) {
        if let Ok(mut guard) = self.network.lock() {
            *guard = Some(network);
        }
    }

    /// Store the shutdown signal sender so that `upgrade_network_to_iroh()`
    /// can subscribe new ingest loops to the shutdown signal.
    pub fn set_shutdown_tx(&self, tx: tokio::sync::watch::Sender<bool>) {
        if let Ok(mut guard) = self.shutdown_tx.lock() {
            *guard = Some(tx);
        }
    }

    /// Store the JoinHandles for the initial gossip ingest loops so they
    /// can be aborted when the network is upgraded from TCP to Iroh.
    pub fn store_ingest_handles(&self, handles: Vec<tokio::task::JoinHandle<()>>) {
        if let Ok(mut guard) = self.ingest_handles.lock() {
            *guard = handles;
        }
    }

    /// Attempt to upgrade the network transport from TCP to Iroh after the
    /// identity is unlocked. This is critical because at boot time the
    /// identity is typically locked, so the node starts with TCP fallback.
    /// Once the user unlocks their identity, we can derive the Iroh secret
    /// key and create a proper Iroh transport with NAT traversal.
    ///
    /// After creating the new Iroh network, this method:
    /// 1. Aborts the old ingest loop tasks (they hold subscriptions to the
    ///    dead TCP gossip service and would never receive messages).
    /// 2. Subscribes to all three gossip topics on the NEW network.
    /// 3. Spawns fresh ingest loops for each subscription.
    /// 4. Stores the new network in the Mutex (dropping the old TCP one).
    ///
    /// Returns `Ok(true)` if the network was upgraded, `Ok(false)` if
    /// already using Iroh or no upgrade was needed.
    #[cfg(feature = "iroh-transport")]
    pub async fn upgrade_network_to_iroh(&self) -> Result<bool, String> {
        use crate::network::{NetworkSubsystem, TransportKind};

        // Check if already using Iroh.
        if let Ok(guard) = self.network.lock() {
            if let Some(ref net) = *guard {
                if net.transport_kind() == TransportKind::Iroh {
                    tracing::debug!("network already using Iroh transport, no upgrade needed");
                    return Ok(false);
                }
            }
        }

        // Derive secret key from unlocked identity.
        let secret_key = self
            .identity
            .get_signing_keypair()
            .map_err(|e| format!("identity not unlocked: {e}"))?
            .secret_bytes();

        // Create new Iroh transport with deterministic key.
        let new_net = NetworkSubsystem::new_iroh_with_key(*secret_key)
            .await
            .map_err(|e| format!("Iroh transport init failed: {e}"))?;

        let new_net = std::sync::Arc::new(new_net);
        tracing::info!(
            node_id = %new_net.local_id(),
            "upgraded network transport from TCP to Iroh"
        );

        // --- Abort old ingest loops ---
        // The old loops hold subscriptions to the dead TCP gossip service.
        // They must be aborted before we spawn replacements.
        if let Ok(mut handles) = self.ingest_handles.lock() {
            for h in handles.drain(..) {
                h.abort();
            }
            tracing::debug!("aborted old gossip ingest loops");
        }

        // --- Subscribe to gossip topics on the NEW network ---
        let feed_sub = new_net
            .subscribe_public_feed()
            .await
            .map_err(|e| format!("subscribe public_feed on new network: {e}"))?;

        let dm_topic = ephemera_gossip::GossipTopic::direct_messages();
        let dm_sub = new_net
            .subscribe(&dm_topic)
            .await
            .map_err(|e| format!("subscribe dm_delivery on new network: {e}"))?;

        let mod_topic = ephemera_gossip::GossipTopic::moderation();
        let mod_sub = new_net
            .subscribe(&mod_topic)
            .await
            .map_err(|e| format!("subscribe moderation on new network: {e}"))?;

        // --- Get shutdown receivers ---
        let shutdown_tx = self
            .shutdown_tx
            .lock()
            .map_err(|e| format!("lock shutdown_tx: {e}"))?
            .clone()
            .ok_or("shutdown_tx not set — node not started")?;

        // --- Spawn fresh ingest loops ---
        let mut new_handles = Vec::with_capacity(3);

        // 1. Public feed ingest
        {
            let content_store = ContentStore::open(&self.content_path)
                .map_err(|e| format!("reopen content store: {e}"))?;
            let metadata_db = MetadataDb::open(&self.metadata_db_path)
                .map_err(|e| format!("reopen metadata db for feed ingest: {e}"))?;
            let metadata_db = std::sync::Mutex::new(metadata_db);
            let event_bus = self.event_bus.clone();
            let rate_limiter = std::sync::Mutex::new(RateLimiter::new());
            let fingerprint_store = std::sync::Mutex::new(FingerprintStore::new());
            let content_filter = std::sync::Mutex::new(ContentFilter::empty());
            let shutdown_rx = shutdown_tx.subscribe();

            let h = tokio::spawn(async move {
                crate::gossip_ingest::gossip_ingest_loop(
                    feed_sub,
                    content_store,
                    metadata_db,
                    event_bus,
                    rate_limiter,
                    fingerprint_store,
                    content_filter,
                    shutdown_rx,
                )
                .await;
            });
            new_handles.push(h);
            tracing::info!("re-spawned gossip ingest loop on new Iroh network");
        }

        // 2. DM (message) ingest
        {
            let dm_metadata_db = MetadataDb::open(&self.metadata_db_path)
                .map_err(|e| format!("reopen metadata db for dm ingest: {e}"))?;
            let dm_metadata_db = std::sync::Mutex::new(dm_metadata_db);
            let dm_event_bus = self.event_bus.clone();
            let dm_shutdown_rx = shutdown_tx.subscribe();
            let our_pubkey = self
                .identity
                .get_signing_keypair()
                .ok()
                .map(|kp| kp.public_key());

            let h = tokio::spawn(async move {
                crate::message_ingest::message_ingest_loop(
                    dm_sub,
                    dm_metadata_db,
                    dm_event_bus,
                    our_pubkey,
                    dm_shutdown_rx,
                )
                .await;
            });
            new_handles.push(h);
            tracing::info!("re-spawned message ingest loop on new Iroh network");
        }

        // 3. Moderation ingest
        {
            let mod_metadata_db = MetadataDb::open(&self.metadata_db_path)
                .map_err(|e| format!("reopen metadata db for moderation ingest: {e}"))?;
            let mod_metadata_db = std::sync::Mutex::new(mod_metadata_db);
            let mod_content_store = ContentStore::open(&self.content_path)
                .map_err(|e| format!("reopen content store for moderation ingest: {e}"))?;
            let mod_shutdown_rx = shutdown_tx.subscribe();

            let h = tokio::spawn(async move {
                crate::moderation_ingest_loop(
                    mod_sub,
                    mod_metadata_db,
                    mod_content_store,
                    mod_shutdown_rx,
                )
                .await;
            });
            new_handles.push(h);
            tracing::info!("re-spawned moderation ingest loop on new Iroh network");
        }

        tracing::info!("Iroh upgrade: gossip re-subscribed, ingest loops re-spawned");
        tracing::info!(
            peer_count = new_net.peer_count(),
            "Iroh upgrade: current peer count after upgrade"
        );

        // Store the new handles.
        if let Ok(mut guard) = self.ingest_handles.lock() {
            *guard = new_handles;
        }

        // Replace the network subsystem. The old TCP transport and its
        // dead gossip subscriptions get dropped when the Arc refcount hits 0.
        if let Ok(mut guard) = self.network.lock() {
            if guard.is_some() {
                tracing::info!("Iroh upgrade: replacing old TCP network subsystem with Iroh");
            }
            *guard = Some(new_net);
        }

        tracing::info!("Iroh upgrade: network upgrade complete");
        Ok(true)
    }

    /// How long the node has been running, in seconds.
    pub fn uptime_secs(&self) -> u64 {
        self.started_at.elapsed().as_secs()
    }

    /// Access the content store.
    pub fn content_store(&self) -> &ContentStore {
        &self.content_store
    }

    /// Initialize the epoch key manager from the current master secret.
    ///
    /// This must be called after the identity is created or unlocked. The
    /// epoch key manager enables cryptographic shredding of expired content.
    pub fn init_epoch_key_manager(&self) -> Result<(), String> {
        let ms_guard = self
            .identity
            .master_secret
            .lock()
            .map_err(|e| format!("lock master_secret: {e}"))?;
        let master = ms_guard
            .as_ref()
            .ok_or("identity is locked; create or unlock first")?;
        let ekm = EpochKeyManager::new(master.clone());
        let mut ekm_guard = self
            .epoch_key_manager
            .lock()
            .map_err(|e| format!("lock epoch_key_manager: {e}"))?;
        *ekm_guard = Some(ekm);
        tracing::info!("epoch key manager initialized");
        Ok(())
    }

    /// Run one GC sweep cycle. Returns (posts_deleted, tombstones_purged).
    pub fn run_gc(&self) -> Result<(u64, u64), String> {
        let db = self
            .metadata_db
            .lock()
            .map_err(|e| format!("lock error: {e}"))?;
        let report = self
            .gc
            .sweep(&db, &self.content_store)
            .map_err(|e| format!("gc sweep error: {e}"))?;
        Ok((report.posts_deleted, report.tombstones_purged))
    }
}
