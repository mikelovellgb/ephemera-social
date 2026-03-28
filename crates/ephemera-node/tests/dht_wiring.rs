//! Integration tests for DHT wiring into the node runtime.
//!
//! Verifies that handle registration, prekey storage, and profile updates
//! correctly publish records to the DHT storage, and that lookups work
//! through the DHT fallback path.

use ephemera_config::NodeConfig;
use ephemera_events::EventBus;
use ephemera_node::services::dht::DhtNodeService;
use ephemera_node::services::{HandleService, ServiceContainer};
use ephemera_social::handle_validation::PowDifficulty;
use std::sync::Arc;

/// Helper: create a ServiceContainer backed by a real temp directory.
fn make_services() -> (Arc<ServiceContainer>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let config = NodeConfig::default_for(dir.path());
    let event_bus = EventBus::new();
    let svc = Arc::new(ServiceContainer::new(&config, event_bus).unwrap());
    (svc, dir)
}

/// Helper: create an identity so we can register handles and store records.
async fn create_identity(svc: &ServiceContainer) -> String {
    let result = svc.identity.create("test-pass").await.unwrap();
    result["pseudonym_pubkey"]
        .as_str()
        .unwrap()
        .to_string()
}

// -----------------------------------------------------------------------
// Test 1: Register a handle and verify it ends up in DHT storage.
// -----------------------------------------------------------------------
#[tokio::test]
async fn test_handle_stored_in_dht() {
    let (svc, _dir) = make_services();
    let _pubkey = create_identity(&svc).await;

    // Register a handle with low difficulty for fast tests.
    let result = HandleService::register_with_difficulty_and_publish(
        "alice",
        PowDifficulty::Renewal,
        &svc.identity,
        &svc.handle_registry,
        &svc.dht_storage,
    )
    .unwrap();

    assert_eq!(result["handle"].as_str().unwrap(), "@alice");

    // Verify the handle is in DHT storage.
    let dht_result = DhtNodeService::lookup_handle("alice", &svc.dht_storage).unwrap();
    assert!(dht_result.is_some(), "handle should be in DHT");

    let dht_val = dht_result.unwrap();
    assert_eq!(dht_val["name"].as_str().unwrap(), "alice");
    assert_eq!(
        dht_val["owner"].as_str().unwrap(),
        result["owner"].as_str().unwrap()
    );
}

// -----------------------------------------------------------------------
// Test 2: Store a handle in DHT, look it up, verify correct pubkey.
// -----------------------------------------------------------------------
#[tokio::test]
async fn test_handle_lookup_from_dht() {
    let (svc, _dir) = make_services();
    let pubkey = create_identity(&svc).await;

    // Register handle and publish to DHT.
    HandleService::register_with_difficulty_and_publish(
        "bob",
        PowDifficulty::Renewal,
        &svc.identity,
        &svc.handle_registry,
        &svc.dht_storage,
    )
    .unwrap();

    // Look up via the DHT-aware lookup path.
    let lookup = HandleService::lookup_with_dht(
        "bob",
        &svc.handle_registry,
        &svc.dht_storage,
    )
    .unwrap();

    assert!(!lookup.is_null(), "handle lookup should return a result");
    assert_eq!(lookup["owner"].as_str().unwrap(), pubkey);
}

// -----------------------------------------------------------------------
// Test 3: Create identity and store a prekey bundle in DHT.
// -----------------------------------------------------------------------
#[tokio::test]
async fn test_prekey_stored_in_dht() {
    let (svc, _dir) = make_services();
    let pubkey = create_identity(&svc).await;

    // Build a synthetic prekey bundle JSON (the real bundle generation
    // requires X25519 keys; here we just test the DHT storage path).
    let prekey_json = serde_json::json!({
        "identity_key": pubkey,
        "signed_prekey": "aabbccdd",
        "one_time_prekey": null,
    });

    DhtNodeService::store_prekey(
        &pubkey,
        &prekey_json,
        &svc.identity,
        &svc.dht_storage,
    )
    .unwrap();

    // Look it up.
    let result = DhtNodeService::lookup_prekey(&pubkey, &svc.dht_storage).unwrap();
    assert!(result.is_some(), "prekey should be in DHT");

    let val = result.unwrap();
    assert_eq!(val["identity_key"].as_str().unwrap(), pubkey);
}

// -----------------------------------------------------------------------
// Test 4: Update profile and verify it ends up in DHT storage.
// -----------------------------------------------------------------------
#[tokio::test]
async fn test_profile_stored_in_dht() {
    let (svc, _dir) = make_services();
    let pubkey = create_identity(&svc).await;

    // Update profile via the DHT-publishing path.
    svc.profiles
        .update_and_publish(
            Some("Alice W."),
            Some("Ephemeral thoughts"),
            &svc.identity,
            &svc.metadata_db,
            &svc.dht_storage,
        )
        .await
        .unwrap();

    // Verify in DHT.
    let result = DhtNodeService::lookup_profile(&pubkey, &svc.dht_storage).unwrap();
    assert!(result.is_some(), "profile should be in DHT");

    let val = result.unwrap();
    assert_eq!(val["display_name"].as_str().unwrap(), "Alice W.");
    assert_eq!(val["bio"].as_str().unwrap(), "Ephemeral thoughts");
}

// -----------------------------------------------------------------------
// Test 5: DHT sweep removes expired records (smoke test).
// -----------------------------------------------------------------------
#[tokio::test]
async fn test_dht_sweep_expired() {
    let (svc, _dir) = make_services();
    let _pubkey = create_identity(&svc).await;

    // Store a valid record.
    let prekey_json = serde_json::json!({ "test": true });
    DhtNodeService::store_prekey(
        "deadbeef",
        &prekey_json,
        &svc.identity,
        &svc.dht_storage,
    )
    .unwrap();

    let count_before = DhtNodeService::record_count(&svc.dht_storage).unwrap();
    assert!(count_before > 0);

    // Sweep should not remove the record (it's still alive).
    let removed = DhtNodeService::sweep_expired(&svc.dht_storage).unwrap();
    assert_eq!(removed, 0, "no records should be expired yet");

    let count_after = DhtNodeService::record_count(&svc.dht_storage).unwrap();
    assert_eq!(count_before, count_after);
}

// -----------------------------------------------------------------------
// Test 6: DHT status reports correct record count.
// -----------------------------------------------------------------------
#[tokio::test]
async fn test_dht_record_count() {
    let (svc, _dir) = make_services();
    let _pubkey = create_identity(&svc).await;

    assert_eq!(
        DhtNodeService::record_count(&svc.dht_storage).unwrap(),
        0,
        "DHT should start empty"
    );

    // Store two records.
    let json = serde_json::json!({ "x": 1 });
    DhtNodeService::store_prekey("aaa", &json, &svc.identity, &svc.dht_storage).unwrap();
    DhtNodeService::store_profile("bbb", &json, &svc.identity, &svc.dht_storage).unwrap();

    assert_eq!(
        DhtNodeService::record_count(&svc.dht_storage).unwrap(),
        2,
        "should have 2 records"
    );
}

// -----------------------------------------------------------------------
// Test 7: Handle lookup falls through to DHT when not in local registry.
// -----------------------------------------------------------------------
#[tokio::test]
async fn test_handle_dht_fallback() {
    let (svc, _dir) = make_services();
    let pubkey = create_identity(&svc).await;

    // Store a handle directly in the DHT (simulating a record replicated
    // from another node), without going through the local HandleRegistry.
    DhtNodeService::store_handle(
        "remote_user",
        &pubkey,
        1000,
        u64::MAX, // far future expiry
        &svc.identity,
        &svc.dht_storage,
    )
    .unwrap();

    // Local registry should NOT have this handle.
    let local_only = HandleService::lookup("remote_user", &svc.handle_registry).unwrap();
    assert!(local_only.is_null(), "handle should NOT be in local registry");

    // DHT-aware lookup should find it.
    let dht_lookup = HandleService::lookup_with_dht(
        "remote_user",
        &svc.handle_registry,
        &svc.dht_storage,
    )
    .unwrap();
    assert!(!dht_lookup.is_null(), "handle should be found via DHT fallback");
    assert_eq!(dht_lookup["name"].as_str().unwrap(), "remote_user");
}
