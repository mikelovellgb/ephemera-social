//! Iroh-based QUIC transport implementation.
//!
//! [`IrohTransport`] implements the [`Transport`] trait using Iroh's QUIC
//! endpoint with built-in NAT traversal via relay servers and hole-punching.
//!
//! Peers are addressed by their Ed25519 public key (EndpointId). Iroh's discovery
//! layer resolves EndpointIds to network addresses automatically, so callers do
//! not need to know IP:port pairs.
//!
//! # ALPN Protocol
//!
//! This transport uses the ALPN string `b"ephemera/0"` for all connections.
//! This ensures that only Ephemera nodes accept connections from each other.
//!
//! # Relay & Discovery
//!
//! After binding, the endpoint must go "online" (connect to at least one relay
//! server) before it can be discovered by other peers. The constructors call
//! `endpoint.online().await` with a timeout to ensure the relay is connected
//! before returning. Without this, outbound `connect()` calls targeting a
//! NodeId-only address may fail because the remote hasn't published its relay
//! URL yet, and *this* endpoint hasn't published either.

use crate::error::TransportError;
use crate::{PeerAddr, Transport};
use ephemera_types::NodeId;

use iroh::endpoint::presets;
use iroh::{EndpointAddr, TransportAddr};

/// Re-export Iroh's Endpoint type for use by consumers that need
/// direct access (e.g., for re-announcing after mobile resume).
pub use iroh::Endpoint;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, Mutex};
use tracing;

/// ALPN protocol identifier for Ephemera transport connections.
const EPHEMERA_ALPN: &[u8] = b"ephemera/0";

/// Maximum frame size: 1 MiB (matches the TCP transport limit).
const MAX_FRAME_SIZE: u32 = 1_048_576;

/// How long to wait for the endpoint to go "online" (connect to a relay
/// server and obtain a local address). Without this, discovery publishing
/// won't have happened yet and remote peers can't find us.
const ONLINE_TIMEOUT: Duration = Duration::from_secs(15);

/// Whether the Iroh relay server was successfully connected.
///
/// This is tracked separately from the transport itself because some
/// networks (e.g. mobile with no IPv6) cannot reach the relay servers.
/// In that case the transport is still usable for direct IP connections,
/// but NAT traversal and discovery-based connections will not work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelayStatus {
    /// Relay connected and working -- NAT traversal is available.
    Connected,
    /// Relay connection timed out -- only direct IP connections work.
    TimedOut,
}

/// Internal message routed from an Iroh connection reader task.
struct InboundMessage {
    sender: NodeId,
    data: Vec<u8>,
}

/// State of a single peer connection (writer channel).
struct PeerState {
    /// Channel to send outbound data to the writer task.
    outbound_tx: mpsc::Sender<Vec<u8>>,
}

/// An Iroh-based QUIC transport implementing the [`Transport`] trait.
///
/// Internally maintains an [`Endpoint`] that handles QUIC connection
/// management, NAT traversal, and relay fallback. Each peer connection
/// spawns reader/writer tasks communicating through MPSC channels.
pub struct IrohTransport {
    /// The Iroh endpoint.
    endpoint: Endpoint,
    /// Our derived NodeId (from the endpoint's public key).
    local_node_id: NodeId,
    /// Connected peers: NodeId -> writer channel.
    peers: Arc<Mutex<HashMap<NodeId, PeerState>>>,
    /// Inbound message channel (sender side, cloned into reader tasks).
    inbound_tx: mpsc::Sender<InboundMessage>,
    /// Inbound message channel (receiver side, consumed by `recv()`).
    inbound_rx: Arc<Mutex<mpsc::Receiver<InboundMessage>>>,
    /// Handle to the background accept loop.
    _accept_task: Option<tokio::task::JoinHandle<()>>,
    /// Whether the relay server was successfully connected.
    relay_status: RelayStatus,
}

impl IrohTransport {
    /// Create a new Iroh transport with a random identity.
    ///
    /// Uses n0's public relay servers for NAT traversal and discovery.
    /// The endpoint binds to an ephemeral port immediately, then waits
    /// (up to [`ONLINE_TIMEOUT`]) for relay connectivity so that the
    /// node's address is published to discovery before any `connect()`
    /// calls are attempted.
    pub async fn new() -> Result<Self, TransportError> {
        let endpoint = Endpoint::builder(presets::N0)
            .alpns(vec![EPHEMERA_ALPN.to_vec()])
            .clear_ip_transports()
            .bind_addr("0.0.0.0:0")
            .map_err(|e| TransportError::ConnectionFailed {
                peer: "local".into(),
                reason: format!("invalid bind address: {e}"),
            })?
            .bind()
            .await
            .map_err(|e| TransportError::ConnectionFailed {
                peer: "local".into(),
                reason: format!("failed to create Iroh endpoint: {e}"),
            })?;

        let local_node_id = iroh_pubkey_to_node_id(endpoint.id());
        tracing::info!(
            node_id = %local_node_id,
            "Iroh endpoint bound, waiting for relay connection..."
        );

        // Wait for the endpoint to connect to at least one relay server.
        // This is CRITICAL: without this, the Pkarr publisher hasn't
        // announced our relay URL yet, so no one can discover us.
        let relay_status = Self::wait_online(&endpoint, &local_node_id).await;

        let (inbound_tx, inbound_rx) = mpsc::channel(1024);
        let peers: Arc<Mutex<HashMap<NodeId, PeerState>>> =
            Arc::new(Mutex::new(HashMap::new()));

        // Spawn the accept loop for incoming connections.
        let accept_endpoint = endpoint.clone();
        let accept_peers = Arc::clone(&peers);
        let accept_inbound_tx = inbound_tx.clone();

        let accept_task = tokio::spawn(async move {
            Self::accept_loop(accept_endpoint, accept_peers, accept_inbound_tx).await;
        });

        tracing::info!(
            node_id = %local_node_id,
            ?relay_status,
            "Iroh transport created"
        );

        Ok(Self {
            endpoint,
            local_node_id,
            peers,
            inbound_tx,
            inbound_rx: Arc::new(Mutex::new(inbound_rx)),
            _accept_task: Some(accept_task),
            relay_status,
        })
    }

    /// Create a new Iroh transport from a known secret key.
    ///
    /// This produces a deterministic NodeId for the same secret key bytes.
    /// Waits for relay connectivity before returning (see [`Self::new`]).
    pub async fn with_secret_key(secret_key_bytes: [u8; 32]) -> Result<Self, TransportError> {
        let secret_key = iroh::SecretKey::from_bytes(&secret_key_bytes);

        let endpoint = Endpoint::builder(presets::N0)
            .alpns(vec![EPHEMERA_ALPN.to_vec()])
            .secret_key(secret_key)
            .clear_ip_transports()
            .bind_addr("0.0.0.0:0")
            .map_err(|e| TransportError::ConnectionFailed {
                peer: "local".into(),
                reason: format!("invalid bind address: {e}"),
            })?
            .bind()
            .await
            .map_err(|e| TransportError::ConnectionFailed {
                peer: "local".into(),
                reason: format!("failed to create Iroh endpoint: {e}"),
            })?;

        let local_node_id = iroh_pubkey_to_node_id(endpoint.id());
        tracing::info!(
            node_id = %local_node_id,
            "Iroh endpoint bound (deterministic key), waiting for relay connection..."
        );

        // Wait for relay connectivity -- same reason as in `new()`.
        let relay_status = Self::wait_online(&endpoint, &local_node_id).await;

        let (inbound_tx, inbound_rx) = mpsc::channel(1024);
        let peers: Arc<Mutex<HashMap<NodeId, PeerState>>> =
            Arc::new(Mutex::new(HashMap::new()));

        let accept_endpoint = endpoint.clone();
        let accept_peers = Arc::clone(&peers);
        let accept_inbound_tx = inbound_tx.clone();

        let accept_task = tokio::spawn(async move {
            Self::accept_loop(accept_endpoint, accept_peers, accept_inbound_tx).await;
        });

        tracing::info!(
            node_id = %local_node_id,
            ?relay_status,
            "Iroh transport created (deterministic key)"
        );

        Ok(Self {
            endpoint,
            local_node_id,
            peers,
            inbound_tx,
            inbound_rx: Arc::new(Mutex::new(inbound_rx)),
            _accept_task: Some(accept_task),
            relay_status,
        })
    }

    /// Return the Iroh endpoint's NodeId as a hex string for display.
    pub fn node_id_hex(&self) -> String {
        format!("{}", self.local_node_id)
    }

    /// Return a reference to the underlying Iroh endpoint.
    ///
    /// Useful for advanced operations like accessing relay info or
    /// connection statistics.
    pub fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }

    /// Whether the relay server was successfully connected.
    ///
    /// When [`RelayStatus::TimedOut`], NAT traversal and discovery-based
    /// connections are unavailable. The user should connect to peers by
    /// entering their IP:port directly.
    pub fn relay_status(&self) -> RelayStatus {
        self.relay_status
    }

    /// Wait for the Iroh endpoint to go "online" (connected to a relay
    /// server) with a timeout. Returns [`RelayStatus::Connected`] if the
    /// relay was reached, or [`RelayStatus::TimedOut`] if not.
    ///
    /// This never fails hard -- the transport is still usable for direct
    /// IP connections even when the relay is unreachable (e.g. on mobile
    /// networks that lack IPv6 connectivity to the relay servers).
    async fn wait_online(
        endpoint: &Endpoint,
        local_node_id: &NodeId,
    ) -> RelayStatus {
        match tokio::time::timeout(ONLINE_TIMEOUT, endpoint.online()).await {
            Ok(()) => {
                tracing::info!(
                    node_id = %local_node_id,
                    "Iroh endpoint is ONLINE (relay connected)"
                );

                // Log the endpoint address for diagnostics.
                let addr = endpoint.addr();
                tracing::info!(
                    node_id = %local_node_id,
                    endpoint_addr = ?addr,
                    "Iroh endpoint address after going online"
                );

                RelayStatus::Connected
            }
            Err(_) => {
                tracing::warn!(
                    node_id = %local_node_id,
                    timeout_secs = ONLINE_TIMEOUT.as_secs(),
                    "Iroh relay connection TIMED OUT. This usually means the \
                     network cannot reach the relay server (e.g. IPv6 not \
                     available on this network). Direct IP connections still \
                     work, but NAT traversal and discovery are unavailable. \
                     Users should connect by entering peer IP:port directly."
                );

                RelayStatus::TimedOut
            }
        }
    }

    /// Background loop accepting incoming QUIC connections.
    async fn accept_loop(
        endpoint: Endpoint,
        peers: Arc<Mutex<HashMap<NodeId, PeerState>>>,
        inbound_tx: mpsc::Sender<InboundMessage>,
    ) {
        tracing::info!("Iroh accept loop: waiting for incoming connections...");
        loop {
            let incoming = match endpoint.accept().await {
                Some(incoming) => {
                    tracing::info!("Iroh accept loop: received incoming connection attempt");
                    incoming
                }
                None => {
                    tracing::info!("Iroh accept loop: endpoint closed, exiting accept loop");
                    break;
                }
            };

            let peers = Arc::clone(&peers);
            let inbound_tx = inbound_tx.clone();

            tokio::spawn(async move {
                match incoming.accept() {
                    Ok(accepting) => {
                        tracing::info!("Iroh accept loop: QUIC handshake in progress...");
                        match accepting.await {
                            Ok(connection) => {
                                let remote_pubkey = connection.remote_id();
                                let remote_node_id = iroh_pubkey_to_node_id(remote_pubkey);
                                tracing::info!(
                                    remote = ?remote_node_id,
                                    "Iroh accept loop: incoming connection accepted!"
                                );
                                Self::setup_connection(
                                    connection,
                                    remote_node_id,
                                    peers,
                                    inbound_tx,
                                )
                                .await;
                            }
                            Err(e) => {
                                tracing::error!(
                                    error = %e,
                                    "Iroh accept loop: QUIC handshake FAILED"
                                );
                            }
                        }
                    }
                    Err(e) => {
                        tracing::error!(
                            error = %e,
                            "Iroh accept loop: failed to accept incoming connection"
                        );
                    }
                }
            });
        }
    }

    /// Set up reader/writer tasks for an established QUIC connection.
    ///
    /// If a connection to this peer already exists (e.g. they connected to
    /// us while we were also connecting to them), the new connection is
    /// silently dropped and the existing one is kept. This prevents a
    /// failed reverse-connect from tearing down a working inbound
    /// connection.
    async fn setup_connection(
        connection: iroh::endpoint::Connection,
        remote_node_id: NodeId,
        peers: Arc<Mutex<HashMap<NodeId, PeerState>>>,
        inbound_tx: mpsc::Sender<InboundMessage>,
    ) {
        let (outbound_tx, outbound_rx) = mpsc::channel::<Vec<u8>>(256);

        // Register the peer — but only if not already connected.
        {
            let mut guard = peers.lock().await;
            if guard.contains_key(&remote_node_id) {
                tracing::info!(
                    remote = ?remote_node_id,
                    peer_count = guard.len(),
                    "Iroh: peer already connected, keeping existing connection"
                );
                // Drop the new QUIC connection — the existing one is live.
                return;
            }
            guard.insert(remote_node_id, PeerState { outbound_tx });
            tracing::info!(
                remote = ?remote_node_id,
                peer_count = guard.len(),
                "Iroh: peer registered, total connected peers"
            );
        }

        // Spawn writer task.
        let writer_conn = connection.clone();
        let peers_writer = Arc::clone(&peers);
        let writer_peer_id = remote_node_id;
        tokio::spawn(async move {
            Self::writer_loop(writer_conn, outbound_rx).await;
            // Only remove the peer if no newer connection has replaced us.
            // We check by verifying the outbound_tx is closed (our rx was
            // consumed by writer_loop and is now dropped).
            let mut guard = peers_writer.lock().await;
            if let Some(state) = guard.get(&writer_peer_id) {
                // If the stored sender is closed, it means our reader also
                // died and no replacement was inserted. Safe to remove.
                if state.outbound_tx.is_closed() {
                    guard.remove(&writer_peer_id);
                    tracing::info!(?writer_peer_id, "Iroh peer writer task ended, peer removed");
                } else {
                    tracing::debug!(
                        ?writer_peer_id,
                        "Iroh peer writer task ended, but peer has active sender (replacement connection?) — not removing"
                    );
                }
            }
        });

        // Spawn reader task.
        let reader_conn = connection;
        let reader_peer_id = remote_node_id;
        tokio::spawn(async move {
            Self::reader_loop(reader_conn, reader_peer_id, inbound_tx).await;
            // Reader ended — but don't blindly remove the peer. The writer
            // cleanup handles removal. We just log.
            tracing::info!(?reader_peer_id, "Iroh peer reader task ended");
        });
    }

    /// Writer task: opens a unidirectional send stream for each message.
    ///
    /// Each message is sent as a length-prefixed frame on a new uni stream.
    /// Using uni streams avoids head-of-line blocking between messages.
    async fn writer_loop(
        connection: iroh::endpoint::Connection,
        mut outbound_rx: mpsc::Receiver<Vec<u8>>,
    ) {
        while let Some(data) = outbound_rx.recv().await {
            match connection.open_uni().await {
                Ok(mut send_stream) => {
                    let len = data.len() as u32;
                    if send_stream.write_all(&len.to_be_bytes()).await.is_err() {
                        tracing::warn!("Iroh writer: failed to send length prefix");
                        break;
                    }
                    if send_stream.write_all(&data).await.is_err() {
                        tracing::warn!("Iroh writer: failed to send payload");
                        break;
                    }
                    if send_stream.finish().is_err() {
                        tracing::warn!("Iroh writer: failed to finish stream");
                        break;
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, "Iroh writer: failed to open uni stream");
                    break;
                }
            }
        }
    }

    /// Reader task: accepts incoming unidirectional streams and reads
    /// length-prefixed frames.
    async fn reader_loop(
        connection: iroh::endpoint::Connection,
        peer_id: NodeId,
        inbound_tx: mpsc::Sender<InboundMessage>,
    ) {
        loop {
            match connection.accept_uni().await {
                Ok(mut recv_stream) => {
                    // Read 4-byte length prefix.
                    let mut len_buf = [0u8; 4];
                    if recv_stream.read_exact(&mut len_buf).await.is_err() {
                        tracing::debug!(?peer_id, "Iroh reader: stream closed");
                        continue;
                    }
                    let len = u32::from_be_bytes(len_buf);

                    if len > MAX_FRAME_SIZE {
                        tracing::warn!(
                            ?peer_id,
                            len,
                            "Iroh reader: frame too large, skipping"
                        );
                        continue;
                    }

                    // Read payload.
                    let mut payload = vec![0u8; len as usize];
                    if recv_stream.read_exact(&mut payload).await.is_err() {
                        tracing::debug!(?peer_id, "Iroh reader: failed to read payload");
                        continue;
                    }

                    let msg = InboundMessage {
                        sender: peer_id,
                        data: payload,
                    };
                    if inbound_tx.send(msg).await.is_err() {
                        tracing::debug!(?peer_id, "Iroh reader: inbound channel closed");
                        break;
                    }
                }
                Err(e) => {
                    tracing::debug!(
                        ?peer_id,
                        error = %e,
                        "Iroh reader: connection closed"
                    );
                    break;
                }
            }
        }
    }

    /// Shut down the Iroh endpoint and close all connections.
    pub async fn shutdown(&self) {
        self.endpoint.close().await;
    }
}

#[async_trait::async_trait]
impl Transport for IrohTransport {
    async fn send(&self, peer: &NodeId, data: &[u8]) -> Result<(), TransportError> {
        let guard = self.peers.lock().await;
        let state = guard
            .get(peer)
            .ok_or_else(|| TransportError::PeerNotConnected {
                peer: format!("{peer:?}"),
            })?;

        state.outbound_tx.send(data.to_vec()).await.map_err(|_| {
            TransportError::ConnectionClosed {
                reason: "outbound channel closed".into(),
            }
        })?;

        Ok(())
    }

    async fn recv(&self) -> Result<(NodeId, Vec<u8>), TransportError> {
        let mut rx = self.inbound_rx.lock().await;
        let msg = rx.recv().await.ok_or(TransportError::Shutdown)?;
        Ok((msg.sender, msg.data))
    }

    async fn connect(&self, addr: &PeerAddr) -> Result<(), TransportError> {
        // Fast-path: if we already have a live connection to this peer
        // (e.g. they connected inbound), skip the outbound attempt.
        // This avoids a long timeout and prevents tearing down the
        // existing connection.
        {
            let guard = self.peers.lock().await;
            if guard.contains_key(&addr.node_id) {
                tracing::debug!(
                    node_id = ?addr.node_id,
                    "Iroh: already connected to peer, skipping outbound connect"
                );
                return Ok(());
            }
        }

        tracing::info!(
            node_id = ?addr.node_id,
            addresses = ?addr.addresses,
            "Iroh: starting connect attempt"
        );

        let iroh_node_id = node_id_to_iroh_pubkey(&addr.node_id).map_err(|e| {
            tracing::error!(
                node_id = ?addr.node_id,
                error = %e,
                "Iroh: node_id_to_iroh_pubkey conversion FAILED"
            );
            TransportError::ConnectionFailed {
                peer: format!("{:?}", addr.node_id),
                reason: format!("invalid node ID for Iroh: {e}"),
            }
        })?;

        let node_addr = if addr.addresses.is_empty() {
            tracing::info!(
                "Iroh: no direct addresses provided, relying on discovery/relay"
            );
            // If no addresses are provided, rely on Iroh's discovery.
            EndpointAddr::new(iroh_node_id)
        } else {
            tracing::info!(
                address_count = addr.addresses.len(),
                addresses = ?addr.addresses,
                "Iroh: using provided direct addresses"
            );
            // Parse socket addresses from the PeerAddr.
            let ip_addrs = addr
                .addresses
                .iter()
                .filter_map(|a| a.parse::<std::net::SocketAddr>().ok())
                .map(TransportAddr::Ip);
            EndpointAddr::from_parts(iroh_node_id, ip_addrs)
        };

        tracing::info!(
            alpn = ?EPHEMERA_ALPN,
            "Iroh: calling endpoint.connect()"
        );

        let connection = self
            .endpoint
            .connect(node_addr, EPHEMERA_ALPN)
            .await
            .map_err(|e| {
                tracing::error!(
                    error = %e,
                    node_id = ?addr.node_id,
                    "Iroh: endpoint.connect() FAILED"
                );
                TransportError::ConnectionFailed {
                    peer: format!("{:?}", addr.node_id),
                    reason: format!("Iroh connect failed: {e}"),
                }
            })?;

        let remote_node_id = addr.node_id;
        tracing::info!(
            remote = ?remote_node_id,
            "Iroh: connection established!"
        );

        Self::setup_connection(
            connection,
            remote_node_id,
            Arc::clone(&self.peers),
            self.inbound_tx.clone(),
        )
        .await;

        Ok(())
    }

    async fn disconnect(&self, peer: &NodeId) -> Result<(), TransportError> {
        let mut guard = self.peers.lock().await;
        if guard.remove(peer).is_some() {
            tracing::info!(?peer, "disconnected from Iroh peer");
        }
        Ok(())
    }

    fn connected_peers(&self) -> Vec<NodeId> {
        match self.peers.try_lock() {
            Ok(guard) => guard.keys().copied().collect(),
            Err(_) => Vec::new(),
        }
    }

    fn is_connected(&self, peer: &NodeId) -> bool {
        match self.peers.try_lock() {
            Ok(guard) => guard.contains_key(peer),
            Err(_) => false,
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Convert an Iroh `PublicKey` to our `NodeId`.
///
/// Both are 32-byte Ed25519 public keys, so this is a direct byte copy.
fn iroh_pubkey_to_node_id(pubkey: iroh::PublicKey) -> NodeId {
    NodeId::from_bytes(*pubkey.as_bytes())
}

/// Convert our `NodeId` to an Iroh `PublicKey`.
///
/// Returns an error if the bytes are not a valid Ed25519 curve point
/// (which should never happen for real node IDs derived from keypairs).
fn node_id_to_iroh_pubkey(node_id: &NodeId) -> Result<iroh::PublicKey, String> {
    iroh::PublicKey::from_bytes(node_id.as_bytes())
        .map_err(|e| format!("invalid Ed25519 public key bytes: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_iroh_transport_creates_endpoint() {
        let transport = IrohTransport::new().await;
        match transport {
            Ok(t) => {
                assert!(!t.node_id_hex().is_empty());
                assert_eq!(t.connected_peers().len(), 0);
                t.shutdown().await;
            }
            Err(e) => {
                // Iroh may fail on CI or restricted environments. Log and pass.
                eprintln!("Iroh transport creation failed (may be expected in CI): {e}");
            }
        }
    }

    #[tokio::test]
    async fn test_iroh_node_id_is_deterministic() {
        let secret = [42u8; 32];
        let t1 = IrohTransport::with_secret_key(secret).await;
        let t2 = IrohTransport::with_secret_key(secret).await;

        match (t1, t2) {
            (Ok(t1), Ok(t2)) => {
                assert_eq!(
                    t1.local_node_id, t2.local_node_id,
                    "Same secret key should produce the same NodeId"
                );
                t1.shutdown().await;
                t2.shutdown().await;
            }
            _ => {
                eprintln!("Iroh transport creation failed (may be expected in CI)");
            }
        }
    }

    /// End-to-end connectivity test: two IrohTransport instances connect
    /// using only NodeId (no address hints), send a message, and verify
    /// receipt. This tests the full Iroh discovery + relay pipeline.
    ///
    /// If this test fails, Iroh relay discovery is broken and cross-device
    /// connections will never work.
    #[tokio::test]
    async fn test_iroh_two_transports_connect_and_message() {
        // Use different deterministic keys for the two endpoints.
        let secret_a = [1u8; 32];
        let secret_b = [2u8; 32];

        let t_a = match IrohTransport::with_secret_key(secret_a).await {
            Ok(t) => t,
            Err(e) => {
                eprintln!("Skipping: Iroh transport A creation failed: {e}");
                return;
            }
        };
        let t_b = match IrohTransport::with_secret_key(secret_b).await {
            Ok(t) => t,
            Err(e) => {
                eprintln!("Skipping: Iroh transport B creation failed: {e}");
                t_a.shutdown().await;
                return;
            }
        };

        let node_id_a = t_a.local_node_id;
        let node_id_b = t_b.local_node_id;
        eprintln!("Transport A NodeId: {node_id_a}");
        eprintln!("Transport B NodeId: {node_id_b}");
        assert_ne!(node_id_a, node_id_b, "two different keys must produce different NodeIds");

        // Give Pkarr publisher a moment to propagate the relay URLs.
        // The online() wait ensures relay is connected, but the DNS
        // record may take a beat to be queryable.
        tokio::time::sleep(Duration::from_secs(3)).await;

        // B connects to A using only A's NodeId (no address hints).
        // This exercises the full discovery pipeline: B's endpoint asks
        // DNS for A's relay URL, then connects through the relay.
        let connect_result = tokio::time::timeout(
            Duration::from_secs(30),
            t_b.connect(&PeerAddr {
                node_id: node_id_a,
                addresses: vec![],
            }),
        )
        .await;

        match connect_result {
            Ok(Ok(())) => {
                eprintln!("B connected to A via NodeId-only discovery!");
            }
            Ok(Err(e)) => {
                eprintln!("Connect FAILED: {e}");
                t_a.shutdown().await;
                t_b.shutdown().await;
                panic!("Iroh connect by NodeId failed: {e}");
            }
            Err(_) => {
                t_a.shutdown().await;
                t_b.shutdown().await;
                panic!(
                    "Iroh connect TIMED OUT after 30s. \
                     Discovery/relay pipeline is not working."
                );
            }
        }

        // Verify peer counts.
        // B should see A as connected.
        assert!(
            t_b.is_connected(&node_id_a),
            "B should show A as connected after outbound connect"
        );

        // A's accept loop should have picked up the inbound connection.
        // Give it a moment to register.
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert!(
            t_a.is_connected(&node_id_b),
            "A should show B as connected after accepting inbound"
        );

        // Send a message from A to B.
        let test_payload = b"hello from A to B";
        t_a.send(&node_id_b, test_payload)
            .await
            .expect("send from A to B should succeed");

        // B receives the message.
        let recv_result = tokio::time::timeout(Duration::from_secs(10), t_b.recv()).await;

        match recv_result {
            Ok(Ok((sender, data))) => {
                assert_eq!(sender, node_id_a, "sender should be A");
                assert_eq!(data, test_payload, "payload should match");
                eprintln!("B received message from A: {:?}", String::from_utf8_lossy(&data));
            }
            Ok(Err(e)) => {
                panic!("recv failed: {e}");
            }
            Err(_) => {
                panic!("recv timed out after 10s");
            }
        }

        // Send a message from B to A.
        let reply_payload = b"reply from B to A";
        t_b.send(&node_id_a, reply_payload)
            .await
            .expect("send from B to A should succeed");

        let recv_result = tokio::time::timeout(Duration::from_secs(10), t_a.recv()).await;

        match recv_result {
            Ok(Ok((sender, data))) => {
                assert_eq!(sender, node_id_b, "sender should be B");
                assert_eq!(data, reply_payload, "payload should match");
                eprintln!("A received reply from B: {:?}", String::from_utf8_lossy(&data));
            }
            Ok(Err(e)) => {
                panic!("recv failed: {e}");
            }
            Err(_) => {
                panic!("recv timed out after 10s");
            }
        }

        eprintln!("Full bidirectional communication verified!");

        t_a.shutdown().await;
        t_b.shutdown().await;
    }
}
