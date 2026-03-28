//! TCP transport implementation with length-prefixed framing.
//!
//! Provides [`TcpTransport`] which implements the [`Transport`] trait using
//! tokio's `TcpListener` and `TcpStream`. Messages are framed as:
//! `[u32 big-endian length][payload bytes]`.
//!
//! This is a stepping-stone transport. The plan is to upgrade to QUIC/Iroh
//! once the higher layers are proven.

use crate::error::TransportError;
use crate::{PeerAddr, Transport};
use ephemera_types::NodeId;

use std::collections::HashMap;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, Mutex};
use tracing;

/// Maximum frame size: 1 MiB.
const MAX_FRAME_SIZE: u32 = 1_048_576;

/// Internal message routed from a TCP reader task.
struct InboundMessage {
    sender: NodeId,
    data: Vec<u8>,
}

/// State of a single peer connection (writer half).
struct PeerState {
    /// Channel to send outbound data to the writer task.
    outbound_tx: mpsc::Sender<Vec<u8>>,
}

/// A real TCP transport implementing the [`Transport`] trait.
///
/// Internally spawns reader/writer tasks per connection. Inbound messages
/// are funneled into a single MPSC channel consumed by [`Transport::recv`].
pub struct TcpTransport {
    /// Our own node ID.
    local_id: NodeId,
    /// Connected peers: peer_id -> writer channel.
    peers: Arc<Mutex<HashMap<NodeId, PeerState>>>,
    /// Inbound message channel (sender side, cloned into reader tasks).
    inbound_tx: mpsc::Sender<InboundMessage>,
    /// Inbound message channel (receiver side, consumed by `recv()`).
    inbound_rx: Arc<Mutex<mpsc::Receiver<InboundMessage>>>,
    /// Address this transport is listening on (set after `listen()`).
    listen_addr: Arc<Mutex<Option<std::net::SocketAddr>>>,
    /// Shutdown signal.
    shutdown_tx: tokio::sync::watch::Sender<bool>,
}

impl TcpTransport {
    /// Create a new TCP transport for the given local node ID.
    pub fn new(local_id: NodeId) -> Self {
        let (inbound_tx, inbound_rx) = mpsc::channel(1024);
        let (shutdown_tx, _) = tokio::sync::watch::channel(false);
        Self {
            local_id,
            peers: Arc::new(Mutex::new(HashMap::new())),
            inbound_tx,
            inbound_rx: Arc::new(Mutex::new(inbound_rx)),
            listen_addr: Arc::new(Mutex::new(None)),
            shutdown_tx,
        }
    }

    /// Start listening for incoming connections on the given address.
    ///
    /// Spawns a background task that accepts connections and performs a
    /// handshake to learn the remote peer's node ID.
    pub async fn listen(&self, addr: &str) -> Result<std::net::SocketAddr, TransportError> {
        let listener = TcpListener::bind(addr).await?;
        let local_addr = listener.local_addr()?;
        {
            let mut guard = self.listen_addr.lock().await;
            *guard = Some(local_addr);
        }

        let peers = Arc::clone(&self.peers);
        let inbound_tx = self.inbound_tx.clone();
        let local_id = self.local_id;
        let mut shutdown_rx = self.shutdown_tx.subscribe();

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    result = listener.accept() => {
                        match result {
                            Ok((stream, peer_addr)) => {
                                tracing::debug!(%peer_addr, "accepted TCP connection");
                                let peers = Arc::clone(&peers);
                                let inbound_tx = inbound_tx.clone();
                                tokio::spawn(async move {
                                    if let Err(e) = Self::handle_inbound(
                                        stream, local_id, peers, inbound_tx,
                                    ).await {
                                        tracing::warn!(%peer_addr, error = %e, "inbound connection failed");
                                    }
                                });
                            }
                            Err(e) => {
                                tracing::error!(error = %e, "TCP accept failed");
                            }
                        }
                    }
                    _ = shutdown_rx.changed() => {
                        tracing::debug!("TCP listener shutting down");
                        break;
                    }
                }
            }
        });

        tracing::info!(%local_addr, "TCP transport listening");
        Ok(local_addr)
    }

    /// Handle an inbound TCP connection: perform handshake, then spawn
    /// reader/writer tasks.
    async fn handle_inbound(
        mut stream: TcpStream,
        local_id: NodeId,
        peers: Arc<Mutex<HashMap<NodeId, PeerState>>>,
        inbound_tx: mpsc::Sender<InboundMessage>,
    ) -> Result<(), TransportError> {
        // Handshake: we send our node ID, they send theirs.
        // Send our ID.
        stream.write_all(local_id.as_bytes()).await?;
        stream.flush().await?;

        // Read their ID.
        let mut remote_id_bytes = [0u8; 32];
        stream.read_exact(&mut remote_id_bytes).await?;
        let remote_id = NodeId::from_bytes(remote_id_bytes);

        tracing::info!(?remote_id, "inbound peer handshake complete");

        Self::setup_connection(stream, remote_id, peers, inbound_tx).await;
        Ok(())
    }

    /// Set up reader/writer tasks for an established connection.
    async fn setup_connection(
        stream: TcpStream,
        remote_id: NodeId,
        peers: Arc<Mutex<HashMap<NodeId, PeerState>>>,
        inbound_tx: mpsc::Sender<InboundMessage>,
    ) {
        let (read_half, write_half) = stream.into_split();
        let (outbound_tx, outbound_rx) = mpsc::channel::<Vec<u8>>(256);

        // Register the peer.
        {
            let mut guard = peers.lock().await;
            guard.insert(remote_id, PeerState { outbound_tx });
        }

        // Spawn writer task.
        let peers_writer = Arc::clone(&peers);
        let writer_peer_id = remote_id;
        tokio::spawn(async move {
            Self::writer_loop(write_half, outbound_rx).await;
            // Clean up on disconnect.
            let mut guard = peers_writer.lock().await;
            guard.remove(&writer_peer_id);
            tracing::info!(?writer_peer_id, "peer writer task ended, peer removed");
        });

        // Spawn reader task.
        let peers_reader = Arc::clone(&peers);
        let reader_peer_id = remote_id;
        tokio::spawn(async move {
            Self::reader_loop(read_half, reader_peer_id, inbound_tx).await;
            // Clean up on disconnect.
            let mut guard = peers_reader.lock().await;
            guard.remove(&reader_peer_id);
            tracing::info!(?reader_peer_id, "peer reader task ended, peer removed");
        });
    }

    /// Writer task: reads from the outbound channel and writes length-prefixed
    /// frames to the TCP stream.
    async fn writer_loop(
        mut writer: tokio::net::tcp::OwnedWriteHalf,
        mut outbound_rx: mpsc::Receiver<Vec<u8>>,
    ) {
        while let Some(data) = outbound_rx.recv().await {
            let len = data.len() as u32;
            if let Err(e) = writer.write_all(&len.to_be_bytes()).await {
                tracing::warn!(error = %e, "writer: failed to send length prefix");
                break;
            }
            if let Err(e) = writer.write_all(&data).await {
                tracing::warn!(error = %e, "writer: failed to send payload");
                break;
            }
            if let Err(e) = writer.flush().await {
                tracing::warn!(error = %e, "writer: flush failed");
                break;
            }
        }
    }

    /// Reader task: reads length-prefixed frames from the TCP stream and
    /// forwards them to the inbound channel.
    async fn reader_loop(
        mut reader: tokio::net::tcp::OwnedReadHalf,
        peer_id: NodeId,
        inbound_tx: mpsc::Sender<InboundMessage>,
    ) {
        loop {
            // Read 4-byte length prefix.
            let mut len_buf = [0u8; 4];
            if let Err(e) = reader.read_exact(&mut len_buf).await {
                tracing::debug!(?peer_id, error = %e, "reader: connection closed or error");
                break;
            }
            let len = u32::from_be_bytes(len_buf);

            if len > MAX_FRAME_SIZE {
                tracing::warn!(
                    ?peer_id,
                    len,
                    "reader: frame too large, dropping connection"
                );
                break;
            }

            // Read payload.
            let mut payload = vec![0u8; len as usize];
            if let Err(e) = reader.read_exact(&mut payload).await {
                tracing::debug!(?peer_id, error = %e, "reader: failed to read payload");
                break;
            }

            let msg = InboundMessage {
                sender: peer_id,
                data: payload,
            };
            if inbound_tx.send(msg).await.is_err() {
                tracing::debug!(?peer_id, "reader: inbound channel closed");
                break;
            }
        }
    }

    /// Shut down the transport, closing all connections and the listener.
    pub fn shutdown(&self) {
        let _ = self.shutdown_tx.send(true);
    }

    /// Return the address this transport is listening on (if any).
    pub async fn listen_addr(&self) -> Option<std::net::SocketAddr> {
        *self.listen_addr.lock().await
    }
}

#[path = "tcp_transport.rs"]
mod tcp_transport;
