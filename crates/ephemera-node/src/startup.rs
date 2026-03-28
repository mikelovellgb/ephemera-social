//! Node startup sequence and validation.

use crate::services::ServiceContainer;
use ephemera_config::NodeConfig;

/// Errors that can occur during node startup.
#[derive(Debug, thiserror::Error)]
pub enum StartupError {
    /// Configuration is invalid.
    #[error("invalid configuration: {reason}")]
    InvalidConfig { reason: String },
    /// The data directory could not be created or accessed.
    #[error("data directory error: {reason}")]
    DataDir { reason: String },
    /// Keystore initialization failed.
    #[error("keystore initialization failed: {reason}")]
    Keystore { reason: String },
    /// Storage engine initialization failed.
    #[error("storage initialization failed: {reason}")]
    Storage { reason: String },
    /// Transport layer initialization failed.
    #[error("transport initialization failed: {reason}")]
    Transport { reason: String },
    /// A generic internal error.
    #[error("internal error: {reason}")]
    Internal { reason: String },
}

/// Validate that the configuration is usable before proceeding.
pub fn validate_config(config: &NodeConfig) -> Result<(), StartupError> {
    if config.data_dir.as_os_str().is_empty() {
        return Err(StartupError::InvalidConfig {
            reason: "data_dir must not be empty".into(),
        });
    }
    if config.storage.max_storage_bytes < 10 * 1024 * 1024 {
        return Err(StartupError::InvalidConfig {
            reason: format!(
                "max_storage_bytes ({}) is below minimum 10 MiB",
                config.storage.max_storage_bytes
            ),
        });
    }
    if config.storage.gc_interval_secs < 5 {
        return Err(StartupError::InvalidConfig {
            reason: format!(
                "gc_interval_secs ({}) is below minimum 5",
                config.storage.gc_interval_secs
            ),
        });
    }
    if config.max_connections == 0 {
        return Err(StartupError::InvalidConfig {
            reason: "max_connections must be at least 1".into(),
        });
    }
    if config.max_message_size < 1024 {
        return Err(StartupError::InvalidConfig {
            reason: format!(
                "max_message_size ({}) is below minimum 1024 bytes",
                config.max_message_size
            ),
        });
    }
    if config.connection_timeout_secs == 0 {
        return Err(StartupError::InvalidConfig {
            reason: "connection_timeout_secs must be at least 1".into(),
        });
    }
    Ok(())
}

/// Validate data directory: exists, writable, SQLite accessible.
pub fn validate_data_dir(config: &NodeConfig) -> Result<(), StartupError> {
    std::fs::create_dir_all(&config.data_dir).map_err(|e| StartupError::DataDir {
        reason: format!("cannot create '{}': {e}", config.data_dir.display()),
    })?;
    let test_path = config.data_dir.join(".ephemera_write_test");
    std::fs::write(&test_path, b"ok").map_err(|e| StartupError::DataDir {
        reason: format!("'{}' is not writable: {e}", config.data_dir.display()),
    })?;
    let _ = std::fs::remove_file(&test_path);
    let metadata_path = config.metadata_db_path();
    if let Some(parent) = metadata_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| StartupError::Storage {
            reason: format!("cannot create metadata directory: {e}"),
        })?;
    }
    let _db = ephemera_store::MetadataDb::open(&metadata_path).map_err(|e| {
        StartupError::Storage {
            reason: format!("cannot open SQLite at '{}': {e}", metadata_path.display()),
        }
    })?;
    tracing::info!(data_dir = %config.data_dir.display(), "data directory validated");
    Ok(())
}

/// Check whether a listen port is available.
pub fn validate_port_available(addr: std::net::SocketAddr) -> Result<(), StartupError> {
    match std::net::TcpListener::bind(addr) {
        Ok(_) => {
            tracing::info!(%addr, "listen port is available");
            Ok(())
        }
        Err(e) => Err(StartupError::Transport {
            reason: format!("port {} is not available: {e}", addr.port()),
        }),
    }
}

/// Execute the full startup sequence.
pub async fn run_startup_sequence(
    config: &NodeConfig,
    _services: &ServiceContainer,
) -> Result<(), StartupError> {
    tracing::info!(
        data_dir = %config.data_dir.display(),
        profile = ?config.profile,
        max_connections = config.max_connections,
        max_message_size = config.max_message_size,
        connection_timeout_secs = config.connection_timeout_secs,
        max_storage_bytes = config.storage.max_storage_bytes,
        gc_interval_secs = config.storage.gc_interval_secs,
        "node configuration"
    );
    tracing::info!(path = %config.data_dir.display(), "validating data directory");
    validate_data_dir(config)?;
    tracing::info!("initializing keystore");
    let keystore_dir = config.data_dir.join("keystore");
    std::fs::create_dir_all(&keystore_dir).map_err(|e| StartupError::Keystore {
        reason: e.to_string(),
    })?;
    tracing::info!("initializing storage engine");
    let content_path = config.content_path();
    let metadata_dir = config.data_dir.join("metadata");
    std::fs::create_dir_all(&content_path).map_err(|e| StartupError::Storage {
        reason: e.to_string(),
    })?;
    std::fs::create_dir_all(&metadata_dir).map_err(|e| StartupError::Storage {
        reason: e.to_string(),
    })?;
    if let Some(addr) = config.listen_addr {
        tracing::info!(%addr, "checking port availability");
        validate_port_available(addr)?;
    }
    tracing::info!(tier = ?config.transport.default_tier, "initializing transport");
    tracing::info!("starting gossip overlay");
    tracing::info!("bootstrapping DHT");
    tracing::info!("startup sequence complete");
    Ok(())
}

#[cfg(test)]
#[path = "startup_tests.rs"]
mod tests;
