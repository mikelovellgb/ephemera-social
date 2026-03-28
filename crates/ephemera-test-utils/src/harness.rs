//! [`TestHarness`]: manage a network of N test nodes for integration testing.
//!
//! The harness creates multiple [`TestNode`] instances, each with its own
//! temp directory and identity. Nodes can be connected in various topologies
//! and the harness provides methods to wait for gossip convergence.

use crate::node::TestNode;
use std::time::Duration;

/// Connectivity topology for the test harness.
#[derive(Debug, Clone, Copy)]
pub enum Topology {
    /// No automatic connections (connect manually with `harness.connect(i, j)`).
    Disconnected,
    /// Full mesh: every node is connected to every other node.
    FullMesh,
    /// Linear chain: node 0 <-> 1 <-> 2 <-> ... <-> N-1.
    Chain,
    /// Star: node 0 is the hub, all other nodes connect only to node 0.
    Star,
}

/// Configuration for the test harness.
#[derive(Debug, Clone)]
pub struct HarnessConfig {
    /// Number of nodes to create.
    pub node_count: usize,
    /// Initial connectivity topology.
    pub topology: Topology,
    /// Simulated network latency (applied as a tokio::time::sleep before
    /// forwarding messages between nodes). Zero means no simulated latency.
    pub simulated_latency: Duration,
}

impl Default for HarnessConfig {
    fn default() -> Self {
        Self {
            node_count: 2,
            topology: Topology::FullMesh,
            simulated_latency: Duration::ZERO,
        }
    }
}

impl HarnessConfig {
    /// Create a config for N fully-connected nodes with no latency.
    #[must_use]
    pub fn full_mesh(node_count: usize) -> Self {
        Self {
            node_count,
            topology: Topology::FullMesh,
            simulated_latency: Duration::ZERO,
        }
    }

    /// Create a config for N disconnected nodes.
    #[must_use]
    pub fn disconnected(node_count: usize) -> Self {
        Self {
            node_count,
            topology: Topology::Disconnected,
            simulated_latency: Duration::ZERO,
        }
    }
}

/// A test harness that manages a network of N test nodes.
///
/// All nodes are cleaned up when the harness is dropped (temp dirs removed).
pub struct TestHarness {
    nodes: Vec<TestNode>,
    config: HarnessConfig,
}

impl TestHarness {
    /// Create a new test harness with the given configuration.
    ///
    /// All nodes are created and started. The initial topology is
    /// applied (connections are registered at the service layer).
    ///
    /// # Errors
    ///
    /// Returns an error if any node fails to start.
    pub async fn new(config: HarnessConfig) -> Result<Self, Box<dyn std::error::Error>> {
        let mut nodes = Vec::with_capacity(config.node_count);

        for i in 0..config.node_count {
            let node = TestNode::started().await?;
            tracing::info!(
                index = i,
                peer_id = %node.identity().node_id(),
                "harness: created test node"
            );
            nodes.push(node);
        }

        let mut harness = Self { nodes, config };
        harness.apply_topology().await;

        Ok(harness)
    }

    /// Create a default harness with 2 fully-connected nodes.
    ///
    /// # Errors
    ///
    /// Returns an error if node creation fails.
    pub async fn default_pair() -> Result<Self, Box<dyn std::error::Error>> {
        Self::new(HarnessConfig::default()).await
    }

    /// Access the node at the given index.
    ///
    /// # Panics
    ///
    /// Panics if `index >= node_count`.
    #[must_use]
    pub fn node(&self, index: usize) -> &TestNode {
        &self.nodes[index]
    }

    /// Access the node at the given index mutably.
    ///
    /// # Panics
    ///
    /// Panics if `index >= node_count`.
    pub fn node_mut(&mut self, index: usize) -> &mut TestNode {
        &mut self.nodes[index]
    }

    /// The number of nodes in the harness.
    #[must_use]
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Connect two nodes at the service layer.
    ///
    /// In the current stub implementation, this sends `social.connect`
    /// RPCs in both directions. When real transport is implemented, this
    /// will establish actual network connections.
    pub async fn connect(&self, i: usize, j: usize) {
        let peer_j = self.nodes[j].identity().node_id().to_string();
        let peer_i = self.nodes[i].identity().node_id().to_string();

        if !self.config.simulated_latency.is_zero() {
            tokio::time::sleep(self.config.simulated_latency).await;
        }

        let _ = self.nodes[i]
            .node_rpc(
                "social.connect",
                serde_json::json!({
                    "target": peer_j,
                    "message": "test connection",
                }),
            )
            .await;

        let _ = self.nodes[j]
            .node_rpc("social.accept", serde_json::json!({ "from": peer_i }))
            .await;

        tracing::debug!(from = i, to = j, "harness: connected nodes");
    }

    /// Wait for gossip to converge across all nodes.
    ///
    /// This is a best-effort operation. With the current stub services,
    /// gossip convergence is a no-op. Once real gossip is implemented,
    /// this will poll node state until all nodes have the same content
    /// or the timeout expires.
    ///
    /// # Arguments
    ///
    /// * `timeout` — maximum time to wait for convergence.
    ///
    /// # Errors
    ///
    /// Returns an error if convergence is not reached within the timeout.
    pub async fn wait_for_gossip_convergence(
        &self,
        timeout: Duration,
    ) -> Result<(), Box<dyn std::error::Error>> {
        tracing::debug!(
            timeout_ms = timeout.as_millis(),
            node_count = self.nodes.len(),
            "waiting for gossip convergence"
        );

        // With stub services, gossip doesn't actually propagate anything.
        // We sleep for a brief moment to let any async tasks settle, then
        // return. Real implementation will poll for actual convergence.
        let settle_time = Duration::from_millis(100).min(timeout);
        tokio::time::sleep(settle_time).await;

        tracing::debug!("gossip convergence: done (stub)");
        Ok(())
    }

    /// Shut down all nodes in the harness.
    ///
    /// # Errors
    ///
    /// Returns an error if any node fails to shut down.
    pub async fn shutdown_all(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        for (i, node) in self.nodes.iter_mut().enumerate() {
            tracing::debug!(index = i, "harness: shutting down node");
            node.shutdown().await?;
        }
        Ok(())
    }

    // -- Private helpers -----------------------------------------------------

    /// Apply the configured topology by connecting node pairs.
    async fn apply_topology(&mut self) {
        let n = self.nodes.len();
        if n < 2 {
            return;
        }

        match self.config.topology {
            Topology::Disconnected => {
                // No connections.
            }
            Topology::FullMesh => {
                for i in 0..n {
                    for j in (i + 1)..n {
                        self.connect(i, j).await;
                    }
                }
            }
            Topology::Chain => {
                for i in 0..(n - 1) {
                    self.connect(i, i + 1).await;
                }
            }
            Topology::Star => {
                for i in 1..n {
                    self.connect(0, i).await;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn harness_creates_nodes() {
        crate::init_test_tracing();
        let harness = TestHarness::new(HarnessConfig::disconnected(3))
            .await
            .unwrap();
        assert_eq!(harness.node_count(), 3);
        for i in 0..3 {
            assert!(harness.node(i).is_running());
        }
    }

    #[tokio::test]
    async fn harness_default_pair() {
        crate::init_test_tracing();
        let harness = TestHarness::default_pair().await.unwrap();
        assert_eq!(harness.node_count(), 2);
    }

    #[tokio::test]
    async fn harness_convergence_returns_ok() {
        crate::init_test_tracing();
        let harness = TestHarness::new(HarnessConfig::full_mesh(2)).await.unwrap();
        harness
            .wait_for_gossip_convergence(Duration::from_secs(5))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn harness_shutdown_all() {
        crate::init_test_tracing();
        let mut harness = TestHarness::new(HarnessConfig::disconnected(2))
            .await
            .unwrap();
        harness.shutdown_all().await.unwrap();
        for i in 0..harness.node_count() {
            assert!(!harness.node(i).is_running());
        }
    }
}
