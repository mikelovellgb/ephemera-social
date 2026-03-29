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
mod post;
mod social;

pub use dht::DhtNodeService;
pub use feed::FeedService;
pub use handle::HandleService;
pub use identity::IdentityService;
pub use media::MediaFile;
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
        })
    }

    /// Set the network subsystem reference. Called by `EphemeraNode::start()`
    /// after the network is created so services can publish to gossip.
    pub fn set_network(&self, network: std::sync::Arc<crate::network::NetworkSubsystem>) {
        if let Ok(mut guard) = self.network.lock() {
            *guard = Some(network);
        }
    }

    /// Attempt to upgrade the network transport from TCP to Iroh after the
    /// identity is unlocked. This is critical because at boot time the
    /// identity is typically locked, so the node starts with TCP fallback.
    /// Once the user unlocks their identity, we can derive the Iroh secret
    /// key and create a proper Iroh transport with NAT traversal.
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

        // Replace the network subsystem.
        if let Ok(mut guard) = self.network.lock() {
            // Shut down the old TCP transport.
            if guard.is_some() {
                // The old network subsystem will be dropped when the Arc refcount hits zero.
                tracing::debug!("replacing old TCP network subsystem");
            }
            *guard = Some(new_net);
        }

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
