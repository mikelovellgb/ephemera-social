use super::*;
use std::path::PathBuf;

fn test_config() -> NodeConfig {
    NodeConfig::default_for(&PathBuf::from("/tmp/ephemera-test"))
}

#[test]
fn valid_config_passes() {
    assert!(validate_config(&test_config()).is_ok());
}

#[test]
fn empty_data_dir_rejected() {
    let mut config = test_config();
    config.data_dir = PathBuf::from("");
    assert!(validate_config(&config).is_err());
}

#[test]
fn tiny_storage_rejected() {
    let mut config = test_config();
    config.storage.max_storage_bytes = 1024;
    assert!(validate_config(&config).is_err());
}

#[test]
fn low_gc_interval_rejected() {
    let mut config = test_config();
    config.storage.gc_interval_secs = 1;
    assert!(validate_config(&config).is_err());
}

#[test]
fn zero_max_connections_rejected() {
    let mut config = test_config();
    config.max_connections = 0;
    let err = validate_config(&config).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("max_connections"), "error was: {msg}");
}

#[test]
fn tiny_message_size_rejected() {
    let mut config = test_config();
    config.max_message_size = 512;
    let err = validate_config(&config).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("max_message_size"), "error was: {msg}");
}

#[test]
fn zero_connection_timeout_rejected() {
    let mut config = test_config();
    config.connection_timeout_secs = 0;
    let err = validate_config(&config).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("connection_timeout"), "error was: {msg}");
}

#[test]
fn test_node_startup_validation_missing_data_dir() {
    let mut config = test_config();
    config.data_dir = PathBuf::from("");
    let result = validate_config(&config);
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(msg.contains("data_dir"), "error: {msg}");
}

#[test]
fn test_node_startup_validation_data_dir_writable() {
    let dir = tempfile::tempdir().unwrap();
    let config = NodeConfig::default_for(dir.path());
    assert!(validate_data_dir(&config).is_ok());
}

#[test]
fn test_resource_limits_enforced() {
    let config = test_config();
    assert_eq!(config.max_connections, 256);
    assert_eq!(config.max_message_size, 64 * 1024);
    assert_eq!(config.connection_timeout_secs, 30);
    assert_eq!(config.storage.gc_interval_secs, 60);
    assert_eq!(config.storage.max_storage_bytes, 10 * 1024 * 1024 * 1024);

    assert!(validate_config(&config).is_ok());

    let mut bad = config.clone();
    bad.max_connections = 0;
    assert!(validate_config(&bad).is_err());

    bad = config.clone();
    bad.max_message_size = 0;
    assert!(validate_config(&bad).is_err());

    bad = config;
    bad.connection_timeout_secs = 0;
    assert!(validate_config(&bad).is_err());
}

#[test]
fn port_availability_check() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let bound_addr = listener.local_addr().unwrap();

    let result = validate_port_available(bound_addr);
    assert!(result.is_err(), "expected port conflict for {bound_addr}");

    drop(listener);

    let result = validate_port_available(bound_addr);
    assert!(result.is_ok(), "expected port to be free for {bound_addr}");
}
