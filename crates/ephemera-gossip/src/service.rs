//! Working gossip service with PlumTree lazy-push optimization.
//!
//! [`EagerGossipService`] implements a hybrid eager/lazy push gossip
//! protocol based on PlumTree. When you publish a message:
//! - Full message is sent to `eager_peers` (sqrt(n) peers)
//! - Only the content hash (IHAVE) is sent to `lazy_peers` (remaining)
//! - Lazy peers that don't receive the full message within a timeout
//!   request it via IWANT.
//!
//! Deduplication prevents infinite loops. The eager/lazy peer split is
//! recomputed on every publish based on the current connected peer set.

use crate::fanout::{select_fanout, FanoutConfig};
use crate::topic::{GossipMessage, GossipTopic, TopicSubscription};
use crate::GossipError;

use ephemera_transport::Transport;
use ephemera_types::NodeId;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tracing;

/// A gossip wire message serialized between peers.
///
/// This is the on-the-wire format for gossip messages. It wraps a topic
/// identifier, the payload, and the content hash for dedup.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GossipWireMessage {
    /// The gossip topic (32 bytes).
    pub topic: [u8; 32],
    /// The message payload.
    pub payload: Vec<u8>,
    /// BLAKE3 hash of the payload for deduplication.
    pub content_hash: [u8; 32],
    /// Node ID of the original publisher.
    pub origin: [u8; 32],
}

/// An IHAVE announcement: tells a peer we have content they may want.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct IHaveMessage {
    /// The gossip topic.
    pub topic: [u8; 32],
    /// BLAKE3 hash of the content we have.
    pub content_hash: [u8; 32],
    /// Node ID of the original publisher.
    pub origin: [u8; 32],
}

/// An IWANT request: asks a peer to send us the full content.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct IWantMessage {
    /// BLAKE3 hash of the content we want.
    pub content_hash: [u8; 32],
}

/// Envelope for all wire-level gossip messages.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type")]
pub enum GossipEnvelope {
    /// Full message (eager push).
    #[serde(rename = "msg")]
    FullMessage(GossipWireMessage),
    /// IHAVE announcement (lazy push).
    #[serde(rename = "ihave")]
    IHave(IHaveMessage),
    /// IWANT request (lazy pull).
    #[serde(rename = "iwant")]
    IWant(IWantMessage),
}

/// Internal state for a single topic subscription.
struct TopicState {
    /// Sender to deliver messages to the local subscriber.
    subscriber_tx: mpsc::Sender<GossipMessage>,
}

/// A pending IHAVE announcement that we need to track in case we need to
/// send an IWANT request.
struct PendingIHave {
    /// The topic this IHAVE was for (reserved for future routing).
    _topic: [u8; 32],
    /// The node that announced it.
    from: NodeId,
    /// When we received the IHAVE.
    received_at: std::time::Instant,
    /// Origin of the message (reserved for future routing).
    _origin: [u8; 32],
}

/// A working gossip service with PlumTree-style eager/lazy split.
///
/// On publish, the full message is sent to a subset of peers (eager push)
/// and only the content hash (IHAVE) is sent to the remaining peers
/// (lazy push). This dramatically reduces bandwidth for large networks.
///
/// The gossip service is transport-agnostic: it works with any type that
/// implements the [`Transport`] trait (TCP, Iroh QUIC, or test mocks).
pub struct EagerGossipService {
    /// Our own node ID.
    local_id: NodeId,
    /// The underlying transport (type-erased via `dyn Transport`).
    transport: Arc<dyn Transport>,
    /// Active topic subscriptions.
    topics: Arc<Mutex<HashMap<GossipTopic, TopicState>>>,
    /// Global dedup set (content hashes we have already seen).
    seen: Arc<Mutex<HashSet<[u8; 32]>>>,
    /// Cache of recently published full messages, keyed by content hash.
    /// Used to respond to IWANT requests.
    message_cache: Arc<Mutex<HashMap<[u8; 32], GossipWireMessage>>>,
    /// Pending IHAVE announcements we haven't received the full message for.
    /// Accessed via Arc clones in background tasks; field is read during construction.
    #[allow(dead_code)]
    pending_ihaves: Arc<Mutex<HashMap<[u8; 32], PendingIHave>>>,
    /// Fanout configuration.
    fanout_config: Arc<FanoutConfig>,
    /// Handle to the background receive task (for shutdown).
    _recv_task: Option<tokio::task::JoinHandle<()>>,
    /// Handle to the lazy-timeout task.
    _lazy_timeout_task: Option<tokio::task::JoinHandle<()>>,
}

impl EagerGossipService {
    /// Create a new gossip service and start the background receive loop.
    ///
    /// Accepts any `Arc<T>` where `T: Transport`. This works with
    /// `Arc<IrohTransport>` or any other transport implementation.
    pub fn new<T: Transport>(local_id: NodeId, transport: Arc<T>) -> Self {
        Self::with_fanout(local_id, transport, FanoutConfig::default())
    }

    /// Create a new gossip service with a custom fanout configuration.
    pub fn with_fanout<T: Transport>(
        local_id: NodeId,
        transport: Arc<T>,
        fanout_config: FanoutConfig,
    ) -> Self {
        // Erase the concrete type to `Arc<dyn Transport>`.
        let transport: Arc<dyn Transport> = transport;
        Self::with_fanout_dyn(local_id, transport, fanout_config)
    }

    /// Internal constructor that works directly with `Arc<dyn Transport>`.
    fn with_fanout_dyn(
        local_id: NodeId,
        transport: Arc<dyn Transport>,
        fanout_config: FanoutConfig,
    ) -> Self {
        let topics: Arc<Mutex<HashMap<GossipTopic, TopicState>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let seen: Arc<Mutex<HashSet<[u8; 32]>>> = Arc::new(Mutex::new(HashSet::new()));
        let message_cache: Arc<Mutex<HashMap<[u8; 32], GossipWireMessage>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let pending_ihaves: Arc<Mutex<HashMap<[u8; 32], PendingIHave>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let fanout_config = Arc::new(fanout_config);

        let recv_transport = Arc::clone(&transport);
        let recv_topics = Arc::clone(&topics);
        let recv_seen = Arc::clone(&seen);
        let recv_local_id = local_id;
        let recv_cache = Arc::clone(&message_cache);
        let recv_pending = Arc::clone(&pending_ihaves);

        let recv_task = tokio::spawn(async move {
            Self::receive_loop(
                recv_transport,
                recv_topics,
                recv_seen,
                recv_local_id,
                recv_cache,
                recv_pending,
            )
            .await;
        });

        // Background task that checks for timed-out IHAVE announcements
        // and sends IWANT requests.
        let timeout_transport = Arc::clone(&transport);
        let timeout_pending = Arc::clone(&pending_ihaves);
        let timeout_seen = Arc::clone(&seen);
        let lazy_timeout = fanout_config.lazy_timeout;

        let lazy_timeout_task = tokio::spawn(async move {
            Self::lazy_timeout_loop(
                timeout_transport,
                timeout_pending,
                timeout_seen,
                lazy_timeout,
            )
            .await;
        });

        Self {
            local_id,
            transport,
            topics,
            seen,
            message_cache,
            pending_ihaves,
            fanout_config,
            _recv_task: Some(recv_task),
            _lazy_timeout_task: Some(lazy_timeout_task),
        }
    }

    /// Subscribe to a gossip topic.
    ///
    /// Returns a [`TopicSubscription`] handle. Messages received from
    /// peers on this topic will be delivered through the subscription.
    pub async fn subscribe(&self, topic: &GossipTopic) -> Result<TopicSubscription, GossipError> {
        let (sub, tx) = TopicSubscription::new(*topic, 256);

        let mut topics = self.topics.lock().await;
        topics.insert(*topic, TopicState { subscriber_tx: tx });

        tracing::info!(%topic, "subscribed to gossip topic");
        Ok(sub)
    }

    /// Unsubscribe from a gossip topic.
    pub async fn unsubscribe(&self, topic: &GossipTopic) -> Result<(), GossipError> {
        let mut topics = self.topics.lock().await;
        if topics.remove(topic).is_none() {
            return Err(GossipError::NotSubscribed {
                topic: format!("{topic}"),
            });
        }
        tracing::info!(%topic, "unsubscribed from gossip topic");
        Ok(())
    }

    /// Publish a message to a gossip topic.
    ///
    /// The message is:
    /// 1. Delivered to the local subscriber (if any)
    /// 2. Full message sent to eager peers (selected via fanout)
    /// 3. IHAVE announcement sent to lazy peers
    pub async fn publish(&self, topic: &GossipTopic, payload: Vec<u8>) -> Result<(), GossipError> {
        let content_hash = *blake3::hash(&payload).as_bytes();

        // Mark as seen to prevent echo.
        {
            let mut seen = self.seen.lock().await;
            seen.insert(content_hash);
        }

        let wire_msg = GossipWireMessage {
            topic: *topic.as_bytes(),
            payload: payload.clone(),
            content_hash,
            origin: *self.local_id.as_bytes(),
        };

        // Cache for IWANT responses.
        {
            let mut cache = self.message_cache.lock().await;
            cache.insert(content_hash, wire_msg.clone());
        }

        let full_envelope = GossipEnvelope::FullMessage(wire_msg);
        let full_bytes = serialize_envelope(&full_envelope)?;

        let ihave_envelope = GossipEnvelope::IHave(IHaveMessage {
            topic: *topic.as_bytes(),
            content_hash,
            origin: *self.local_id.as_bytes(),
        });
        let ihave_bytes = serialize_envelope(&ihave_envelope)?;

        // Deliver locally.
        {
            let topics = self.topics.lock().await;
            if let Some(state) = topics.get(topic) {
                let msg = GossipMessage {
                    topic: *topic,
                    payload: payload.clone(),
                    content_hash,
                    source_node: *self.local_id.as_bytes(),
                };
                let _ = state.subscriber_tx.try_send(msg);
            }
        }

        // Partition peers into eager and lazy sets.
        let peers = self.transport.connected_peers();
        let (eager_peers, lazy_peers) = select_fanout(&peers, &self.fanout_config);

        // Send full message to eager peers.
        for peer in &eager_peers {
            if let Err(e) = self.transport.send(peer, &full_bytes).await {
                tracing::warn!(?peer, error = %e, "gossip publish: failed eager push");
            }
        }

        // Send IHAVE to lazy peers.
        for peer in &lazy_peers {
            if let Err(e) = self.transport.send(peer, &ihave_bytes).await {
                tracing::warn!(?peer, error = %e, "gossip publish: failed lazy push");
            }
        }

        // If there are peers beyond the fanout limit, they also get IHAVE.
        let fanout_total = eager_peers.len() + lazy_peers.len();
        if fanout_total < peers.len() {
            let covered: HashSet<NodeId> = eager_peers
                .iter()
                .chain(lazy_peers.iter())
                .copied()
                .collect();
            for peer in &peers {
                if !covered.contains(peer) {
                    if let Err(e) = self.transport.send(peer, &ihave_bytes).await {
                        tracing::warn!(?peer, error = %e, "gossip publish: failed overflow ihave");
                    }
                }
            }
        }

        Ok(())
    }

    /// Background loop: receive messages from the transport and process them.
    async fn receive_loop(
        transport: Arc<dyn Transport>,
        topics: Arc<Mutex<HashMap<GossipTopic, TopicState>>>,
        seen: Arc<Mutex<HashSet<[u8; 32]>>>,
        local_id: NodeId,
        message_cache: Arc<Mutex<HashMap<[u8; 32], GossipWireMessage>>>,
        pending_ihaves: Arc<Mutex<HashMap<[u8; 32], PendingIHave>>>,
    ) {
        loop {
            let result = transport.recv().await;
            match result {
                Ok((sender_id, data)) => {
                    // Try to deserialize as the new envelope format.
                    let envelope: GossipEnvelope = match serde_json::from_slice(&data) {
                        Ok(env) => env,
                        Err(_) => {
                            // Fall back to legacy GossipWireMessage format for
                            // backward compatibility.
                            match serde_json::from_slice::<GossipWireMessage>(&data) {
                                Ok(msg) => GossipEnvelope::FullMessage(msg),
                                Err(e) => {
                                    tracing::warn!(
                                        error = %e,
                                        "gossip: failed to deserialize wire message"
                                    );
                                    continue;
                                }
                            }
                        }
                    };

                    match envelope {
                        GossipEnvelope::FullMessage(wire_msg) => {
                            Self::handle_full_message(
                                &transport,
                                &topics,
                                &seen,
                                local_id,
                                &message_cache,
                                &pending_ihaves,
                                wire_msg,
                                sender_id,
                                &data,
                            )
                            .await;
                        }
                        GossipEnvelope::IHave(ihave) => {
                            Self::handle_ihave(&seen, &pending_ihaves, ihave, sender_id).await;
                        }
                        GossipEnvelope::IWant(iwant) => {
                            Self::handle_iwant(&transport, &message_cache, iwant, sender_id).await;
                        }
                    }
                }
                Err(ephemera_transport::TransportError::Shutdown) => {
                    tracing::debug!("gossip receive loop: transport shut down");
                    break;
                }
                Err(e) => {
                    tracing::warn!(error = %e, "gossip receive loop: transport error");
                    break;
                }
            }
        }
    }

    /// Handle a full gossip message received from a peer.
    #[allow(clippy::too_many_arguments)]
    async fn handle_full_message(
        transport: &Arc<dyn Transport>,
        topics: &Arc<Mutex<HashMap<GossipTopic, TopicState>>>,
        seen: &Arc<Mutex<HashSet<[u8; 32]>>>,
        local_id: NodeId,
        message_cache: &Arc<Mutex<HashMap<[u8; 32], GossipWireMessage>>>,
        pending_ihaves: &Arc<Mutex<HashMap<[u8; 32], PendingIHave>>>,
        wire_msg: GossipWireMessage,
        sender_id: NodeId,
        raw_data: &[u8],
    ) {
        // Dedup check.
        {
            let mut seen_guard = seen.lock().await;
            if seen_guard.contains(&wire_msg.content_hash) {
                return;
            }
            seen_guard.insert(wire_msg.content_hash);
        }

        // Remove from pending IHAVE if present (we got the full message).
        {
            let mut pending = pending_ihaves.lock().await;
            pending.remove(&wire_msg.content_hash);
        }

        // Cache the message for IWANT responses.
        {
            let mut cache = message_cache.lock().await;
            cache.insert(wire_msg.content_hash, wire_msg.clone());
        }

        let topic = GossipTopic::from_bytes(wire_msg.topic);

        // Deliver to local subscriber.
        {
            let topics_guard = topics.lock().await;
            if let Some(state) = topics_guard.get(&topic) {
                let msg = GossipMessage {
                    topic,
                    payload: wire_msg.payload.clone(),
                    content_hash: wire_msg.content_hash,
                    source_node: wire_msg.origin,
                };
                let _ = state.subscriber_tx.try_send(msg);
            }
        }

        // Forward to all other peers (eager push / flooding).
        let peers = transport.connected_peers();
        for peer in &peers {
            if *peer == sender_id || *peer.as_bytes() == wire_msg.origin || *peer == local_id {
                continue;
            }
            if let Err(e) = transport.send(peer, raw_data).await {
                tracing::warn!(?peer, error = %e, "gossip forward: failed");
            }
        }
    }

    /// Handle an IHAVE announcement from a peer.
    async fn handle_ihave(
        seen: &Arc<Mutex<HashSet<[u8; 32]>>>,
        pending_ihaves: &Arc<Mutex<HashMap<[u8; 32], PendingIHave>>>,
        ihave: IHaveMessage,
        sender_id: NodeId,
    ) {
        // If we already have this content, ignore.
        {
            let seen_guard = seen.lock().await;
            if seen_guard.contains(&ihave.content_hash) {
                return;
            }
        }

        // Track as pending -- the lazy timeout loop will send IWANT if
        // we don't receive the full message in time.
        let mut pending = pending_ihaves.lock().await;
        pending.entry(ihave.content_hash).or_insert(PendingIHave {
            _topic: ihave.topic,
            from: sender_id,
            received_at: std::time::Instant::now(),
            _origin: ihave.origin,
        });

        tracing::trace!(
            hash = hex::encode(ihave.content_hash),
            "received IHAVE, tracking for lazy pull"
        );
    }

    /// Handle an IWANT request from a peer.
    async fn handle_iwant(
        transport: &Arc<dyn Transport>,
        message_cache: &Arc<Mutex<HashMap<[u8; 32], GossipWireMessage>>>,
        iwant: IWantMessage,
        sender_id: NodeId,
    ) {
        let cache = message_cache.lock().await;
        if let Some(wire_msg) = cache.get(&iwant.content_hash) {
            let envelope = GossipEnvelope::FullMessage(wire_msg.clone());
            match serde_json::to_vec(&envelope) {
                Ok(bytes) => {
                    if let Err(e) = transport.send(&sender_id, &bytes).await {
                        tracing::warn!(
                            ?sender_id,
                            error = %e,
                            "gossip: failed to send IWANT response"
                        );
                    } else {
                        tracing::trace!(
                            hash = hex::encode(iwant.content_hash),
                            ?sender_id,
                            "sent full message in response to IWANT"
                        );
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        "gossip: failed to serialize IWANT response"
                    );
                }
            }
        } else {
            tracing::trace!(
                hash = hex::encode(iwant.content_hash),
                "received IWANT but message not in cache"
            );
        }
    }

    /// Background loop that checks for timed-out IHAVE announcements and
    /// sends IWANT requests to retrieve the full content.
    async fn lazy_timeout_loop(
        transport: Arc<dyn Transport>,
        pending_ihaves: Arc<Mutex<HashMap<[u8; 32], PendingIHave>>>,
        seen: Arc<Mutex<HashSet<[u8; 32]>>>,
        lazy_timeout: std::time::Duration,
    ) {
        let check_interval = lazy_timeout / 2;
        let mut interval =
            tokio::time::interval(check_interval.max(std::time::Duration::from_millis(100)));

        loop {
            interval.tick().await;

            let mut to_request: Vec<([u8; 32], NodeId)> = Vec::new();

            {
                let seen_guard = seen.lock().await;
                let mut pending = pending_ihaves.lock().await;
                let mut expired_keys = Vec::new();

                for (hash, entry) in pending.iter() {
                    // If we already have it, just remove.
                    if seen_guard.contains(hash) {
                        expired_keys.push(*hash);
                        continue;
                    }

                    // If the IHAVE is older than the timeout, send IWANT.
                    if entry.received_at.elapsed() >= lazy_timeout {
                        to_request.push((*hash, entry.from));
                        expired_keys.push(*hash);
                    }
                }

                for key in expired_keys {
                    pending.remove(&key);
                }
            }

            for (content_hash, peer) in to_request {
                let envelope = GossipEnvelope::IWant(IWantMessage { content_hash });
                match serde_json::to_vec(&envelope) {
                    Ok(bytes) => {
                        if let Err(e) = transport.send(&peer, &bytes).await {
                            tracing::warn!(
                                ?peer,
                                error = %e,
                                "gossip: failed to send IWANT"
                            );
                        } else {
                            tracing::trace!(
                                hash = hex::encode(content_hash),
                                ?peer,
                                "sent IWANT request"
                            );
                        }
                    }
                    Err(e) => {
                        tracing::warn!(
                            error = %e,
                            "gossip: failed to serialize IWANT"
                        );
                    }
                }
            }
        }
    }
}

/// Serialize a gossip envelope to bytes.
fn serialize_envelope(envelope: &GossipEnvelope) -> Result<Vec<u8>, GossipError> {
    serde_json::to_vec(envelope).map_err(|e| {
        GossipError::Transport(ephemera_transport::TransportError::ConnectionClosed {
            reason: format!("serialization error: {e}"),
        })
    })
}

#[cfg(test)]
#[path = "service_tests.rs"]
mod tests;
