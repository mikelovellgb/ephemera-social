//! Transport configuration.
//!
//! Mirrors the configuration specified in the architecture document
//! (02_network_protocol.md Section 2.2).

use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Configuration for the QUIC transport layer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransportConfig {
    /// Maximum concurrent QUIC connections.
    pub max_connections: usize,
    /// QUIC idle timeout before a connection is closed.
    pub idle_timeout: Duration,
    /// Maximum QUIC streams per connection.
    pub max_streams: u64,
    /// UDP socket receive buffer size in bytes.
    pub recv_buffer_size: usize,
    /// UDP socket send buffer size in bytes.
    pub send_buffer_size: usize,
    /// Keep-alive interval for QUIC connections.
    pub keep_alive_interval: Duration,
    /// Bootstrap node addresses to connect to on startup.
    pub bootstrap_nodes: Vec<String>,
    /// Community relay node configurations.
    pub relay_nodes: Vec<RelayNodeConfig>,
    /// Send timeout for individual messages.
    pub send_timeout: Duration,
    /// Maximum message size in bytes.
    pub max_message_size: usize,
}

impl Default for TransportConfig {
    fn default() -> Self {
        Self {
            max_connections: 256,
            idle_timeout: Duration::from_secs(60),
            max_streams: 100,
            recv_buffer_size: 2 * 1024 * 1024, // 2 MiB
            send_buffer_size: 2 * 1024 * 1024, // 2 MiB
            keep_alive_interval: Duration::from_secs(15),
            bootstrap_nodes: Vec::new(),
            relay_nodes: Vec::new(),
            send_timeout: Duration::from_secs(10),
            max_message_size: 1_048_576, // 1 MiB
        }
    }
}

/// Configuration for a community relay node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelayNodeConfig {
    /// 32-byte node ID of the relay.
    pub node_id: [u8; 32],
    /// URL for connecting to the relay (e.g. "https://relay1.ephemera.social").
    pub url: String,
    /// Geographic region hint for latency optimization.
    pub region: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_reasonable() {
        let cfg = TransportConfig::default();
        assert_eq!(cfg.max_connections, 256);
        assert_eq!(cfg.idle_timeout, Duration::from_secs(60));
        assert_eq!(cfg.recv_buffer_size, 2 * 1024 * 1024);
        assert_eq!(cfg.max_message_size, 1_048_576);
    }
}
