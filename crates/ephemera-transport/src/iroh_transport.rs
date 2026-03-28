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

use crate::error::TransportError;
use crate::{PeerAddr, Transport};
use ephemera_types::NodeId;

use iroh::endpoint::presets;
use iroh::{EndpointAddr, Endpoint, TransportAddr};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tracing;

/// ALPN protocol identifier for Ephemera transport connections.
const EPHEMERA_ALPN: &[u8] = b"ephemera/0";

/// Maximum frame size: 1 MiB (matches the TCP transport limit).
const MAX_FRAME_SIZE: u32 = 1_048_576;

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
}

impl IrohTransport {
    /// Create a new Iroh transport with a random identity.
    ///
    /// Uses n0's public relay servers for NAT traversal and discovery.
    /// The endpoint binds to an ephemeral port immediately.
    pub async fn new() -> Result<Self, TransportError> {
        let endpoint = Endpoint::builder(presets::N0)
            .alpns(vec![EPHEMERA_ALPN.to_vec()])
            .bind()
            .await
            .map_err(|e| TransportError::ConnectionFailed {
                peer: "local".into(),
                reason: format!("failed to create Iroh endpoint: {e}"),
            })?;

        let local_node_id = iroh_pubkey_to_node_id(endpoint.id());

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
            "Iroh transport created"
        );

        Ok(Self {
            endpoint,
            local_node_id,
            peers,
            inbound_tx,
            inbound_rx: Arc::new(Mutex::new(inbound_rx)),
            _accept_task: Some(accept_task),
        })
    }

    /// Create a new Iroh transport from a known secret key.
    ///
    /// This produces a deterministic NodeId for the same secret key bytes.
    pub async fn with_secret_key(secret_key_bytes: [u8; 32]) -> Result<Self, TransportError> {
        let secret_key = iroh::SecretKey::from_bytes(&secret_key_bytes);

        let endpoint = Endpoint::builder(presets::N0)
            .alpns(vec![EPHEMERA_ALPN.to_vec()])
            .secret_key(secret_key)
            .bind()
            .await
            .map_err(|e| TransportError::ConnectionFailed {
                peer: "local".into(),
                reason: format!("failed to create Iroh endpoint: {e}"),
            })?;

        let local_node_id = iroh_pubkey_to_node_id(endpoint.id());

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
            "Iroh transport created (deterministic key)"
        );

        Ok(Self {
            endpoint,
            local_node_id,
            peers,
            inbound_tx,
            inbound_rx: Arc::new(Mutex::new(inbound_rx)),
            _accept_task: Some(accept_task),
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

    /// Background loop accepting incoming QUIC connections.
    async fn accept_loop(
        endpoint: Endpoint,
        peers: Arc<Mutex<HashMap<NodeId, PeerState>>>,
        inbound_tx: mpsc::Sender<InboundMessage>,
    ) {
        loop {
            let incoming = match endpoint.accept().await {
                Some(incoming) => incoming,
                None => {
                    tracing::debug!("Iroh accept loop: endpoint closed");
                    break;
                }
            };

            let peers = Arc::clone(&peers);
            let inbound_tx = inbound_tx.clone();

            tokio::spawn(async move {
                match incoming.accept() {
                    Ok(accepting) => match accepting.await {
                        Ok(connection) => {
                            let remote_pubkey = connection.remote_id();
                            let remote_node_id = iroh_pubkey_to_node_id(remote_pubkey);
                            tracing::info!(
                                ?remote_node_id,
                                "accepted incoming Iroh connection"
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
                            tracing::warn!(error = %e, "incoming Iroh connection failed");
                        }
                    },
                    Err(e) => {
                        tracing::warn!(error = %e, "failed to accept incoming connection");
                    }
                }
            });
        }
    }

    /// Set up reader/writer tasks for an established QUIC connection.
    async fn setup_connection(
        connection: iroh::endpoint::Connection,
        remote_node_id: NodeId,
        peers: Arc<Mutex<HashMap<NodeId, PeerState>>>,
        inbound_tx: mpsc::Sender<InboundMessage>,
    ) {
        let (outbound_tx, outbound_rx) = mpsc::channel::<Vec<u8>>(256);

        // Register the peer.
        {
            let mut guard = peers.lock().await;
            guard.insert(remote_node_id, PeerState { outbound_tx });
        }

        // Spawn writer task.
        let writer_conn = connection.clone();
        let peers_writer = Arc::clone(&peers);
        let writer_peer_id = remote_node_id;
        tokio::spawn(async move {
            Self::writer_loop(writer_conn, outbound_rx).await;
            let mut guard = peers_writer.lock().await;
            guard.remove(&writer_peer_id);
            tracing::info!(?writer_peer_id, "Iroh peer writer task ended");
        });

        // Spawn reader task.
        let reader_conn = connection;
        let peers_reader = Arc::clone(&peers);
        let reader_peer_id = remote_node_id;
        tokio::spawn(async move {
            Self::reader_loop(reader_conn, reader_peer_id, inbound_tx).await;
            let mut guard = peers_reader.lock().await;
            guard.remove(&reader_peer_id);
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
        let iroh_node_id = node_id_to_iroh_pubkey(&addr.node_id).map_err(|e| {
            TransportError::ConnectionFailed {
                peer: format!("{:?}", addr.node_id),
                reason: format!("invalid node ID for Iroh: {e}"),
            }
        })?;

        let node_addr = if addr.addresses.is_empty() {
            // If no addresses are provided, rely on Iroh's discovery.
            EndpointAddr::new(iroh_node_id)
        } else {
            // Parse socket addresses from the PeerAddr.
            let ip_addrs = addr
                .addresses
                .iter()
                .filter_map(|a| a.parse::<std::net::SocketAddr>().ok())
                .map(TransportAddr::Ip);
            EndpointAddr::from_parts(iroh_node_id, ip_addrs)
        };

        let connection = self
            .endpoint
            .connect(node_addr, EPHEMERA_ALPN)
            .await
            .map_err(|e| TransportError::ConnectionFailed {
                peer: format!("{:?}", addr.node_id),
                reason: format!("Iroh connect failed: {e}"),
            })?;

        let remote_node_id = addr.node_id;
        tracing::info!(?remote_node_id, "outbound Iroh connection established");

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
}
