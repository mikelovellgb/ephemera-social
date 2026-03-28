//! [`Transport`] trait implementation for [`TcpTransport`].

use super::*;

#[async_trait::async_trait]
impl Transport for TcpTransport {
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
        let address = addr
            .addresses
            .first()
            .ok_or_else(|| TransportError::ConnectionFailed {
                peer: format!("{:?}", addr.node_id),
                reason: "no addresses provided".into(),
            })?;

        let mut stream =
            TcpStream::connect(address)
                .await
                .map_err(|e| TransportError::ConnectionFailed {
                    peer: format!("{:?}", addr.node_id),
                    reason: e.to_string(),
                })?;

        // Handshake: send our ID, read theirs.
        stream.write_all(self.local_id.as_bytes()).await?;
        stream.flush().await?;

        let mut remote_id_bytes = [0u8; 32];
        stream.read_exact(&mut remote_id_bytes).await?;
        let remote_id = NodeId::from_bytes(remote_id_bytes);

        tracing::info!(?remote_id, %address, "outbound peer handshake complete");

        Self::setup_connection(
            stream,
            remote_id,
            Arc::clone(&self.peers),
            self.inbound_tx.clone(),
        )
        .await;

        Ok(())
    }

    async fn disconnect(&self, peer: &NodeId) -> Result<(), TransportError> {
        let mut guard = self.peers.lock().await;
        if guard.remove(peer).is_some() {
            tracing::info!(?peer, "disconnected from peer");
        }
        Ok(())
    }

    fn connected_peers(&self) -> Vec<NodeId> {
        // Note: this blocks briefly on the lock. For a non-async context
        // we use try_lock with a fallback.
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

#[cfg(test)]
#[path = "tcp_tests.rs"]
mod tests;
