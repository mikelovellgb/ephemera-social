use super::*;

#[test]
fn default_config_is_valid() {
    let dir = std::path::PathBuf::from("/tmp/ephemera-test");
    let config = NodeConfig::default_for(&dir);
    assert_eq!(config.data_dir, dir);
    assert_eq!(config.profile, ResourceProfile::Embedded);
    assert_eq!(config.storage.max_storage_bytes, DEFAULT_MAX_STORAGE_BYTES);
    assert_eq!(config.dht.k, 20);
}

#[test]
fn load_or_create_creates_file() {
    let dir = tempfile::tempdir().unwrap();
    let config = NodeConfig::load_or_create(dir.path()).unwrap();
    assert_eq!(config.data_dir, dir.path());
    assert!(dir.path().join("config.toml").exists());
}

#[test]
fn load_or_create_reads_existing() {
    let dir = tempfile::tempdir().unwrap();
    // Create first.
    let _ = NodeConfig::load_or_create(dir.path()).unwrap();
    // Load again.
    let config = NodeConfig::load_or_create(dir.path()).unwrap();
    assert_eq!(config.data_dir, dir.path());
}

#[test]
fn serde_round_trip() {
    let config = NodeConfig::default_for(Path::new("/tmp/test"));
    let toml_str = toml::to_string_pretty(&config).unwrap();
    let recovered: NodeConfig = toml::from_str(&toml_str).unwrap();
    assert_eq!(recovered.data_dir, config.data_dir);
    assert_eq!(recovered.profile, config.profile);
    assert_eq!(recovered.dht.k, config.dht.k);
}

#[test]
fn paths() {
    let config = NodeConfig::default_for(Path::new("/data/ephemera"));
    assert_eq!(
        config.content_path(),
        PathBuf::from("/data/ephemera/content")
    );
    assert_eq!(
        config.metadata_db_path(),
        PathBuf::from("/data/ephemera/metadata.db")
    );
    assert_eq!(
        config.keystore_path(),
        PathBuf::from("/data/ephemera/keystore.enc")
    );
}

#[test]
fn privacy_tier_serde() {
    let json = serde_json::to_string(&PrivacyTier::Stealth).unwrap();
    assert!(json.contains("stealth"));
}

#[test]
fn resource_profile_default() {
    assert_eq!(ResourceProfile::default(), ResourceProfile::Embedded);
}
