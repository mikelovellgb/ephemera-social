use super::*;
use crate::rpc::JsonRpcRequest;
use ephemera_config::NodeConfig;
use ephemera_events::EventBus;

fn test_services() -> Arc<ServiceContainer> {
    let dir = tempfile::tempdir().unwrap();
    let config = NodeConfig::default_for(dir.path());
    let event_bus = EventBus::new();
    let svc = Arc::new(ServiceContainer::new(&config, event_bus).unwrap());
    // Leak the tempdir so it stays alive for the test.
    std::mem::forget(dir);
    svc
}

#[tokio::test]
async fn meta_status_returns_uptime() {
    let router = build_router(test_services());
    let req = JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        method: "meta.status".to_string(),
        params: serde_json::json!({}),
        id: Value::Number(1.into()),
    };
    let resp = router.dispatch(req).await;
    assert!(resp.error.is_none());
    assert!(resp.result.unwrap().get("uptime_seconds").is_some());
}

#[tokio::test]
async fn meta_capabilities_lists_methods() {
    let router = build_router(test_services());
    let req = JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        method: "meta.capabilities".to_string(),
        params: serde_json::json!({}),
        id: Value::Number(2.into()),
    };
    let resp = router.dispatch(req).await;
    assert!(resp.error.is_none());
    let methods = resp.result.unwrap()["methods"].as_array().unwrap().clone();
    assert!(!methods.is_empty());
}

#[tokio::test]
async fn posts_create_returns_hash() {
    let svc = test_services();

    // Create identity first.
    svc.identity.create("test-pass").await.unwrap();

    let router = build_router(Arc::clone(&svc));
    let req = JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        method: "posts.create".to_string(),
        params: serde_json::json!({ "body": "Hello #ephemera!", "ttl_seconds": 86400 }),
        id: Value::Number(3.into()),
    };
    let resp = router.dispatch(req).await;
    assert!(resp.error.is_none(), "error: {:?}", resp.error);
    assert!(resp.result.unwrap().get("content_hash").is_some());
}

#[tokio::test]
async fn missing_param_returns_invalid_params() {
    let router = build_router(test_services());
    let req = JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        method: "identity.create".to_string(),
        params: serde_json::json!({}),
        id: Value::Number(4.into()),
    };
    let resp = router.dispatch(req).await;
    assert_eq!(resp.error.unwrap().code, error_codes::INVALID_PARAMS);
}

#[tokio::test]
async fn test_export_qr_returns_svg() {
    let svc = test_services();
    svc.identity.create("test-pass").await.unwrap();

    let router = build_router(Arc::clone(&svc));
    let req = JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        method: "identity.export_qr".to_string(),
        params: serde_json::json!({}),
        id: Value::Number(5.into()),
    };
    let resp = router.dispatch(req).await;
    assert!(resp.error.is_none(), "error: {:?}", resp.error);

    let result = resp.result.unwrap();
    assert!(result.get("qr_svg").is_some(), "response must contain qr_svg");
    assert!(result.get("qr_hex").is_some(), "response must contain qr_hex");
    assert_eq!(result["length"].as_u64().unwrap(), 37, "QR payload is 37 bytes");
}

#[tokio::test]
async fn test_qr_svg_is_valid() {
    let svc = test_services();
    svc.identity.create("test-pass").await.unwrap();

    let router = build_router(Arc::clone(&svc));
    let req = JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        method: "identity.export_qr".to_string(),
        params: serde_json::json!({}),
        id: Value::Number(6.into()),
    };
    let resp = router.dispatch(req).await;
    let result = resp.result.unwrap();
    let svg = result["qr_svg"].as_str().unwrap();

    // Must be valid SVG structure (may start with <?xml ...?> declaration)
    assert!(svg.contains("<svg"), "SVG must contain <svg tag, got: {}...", &svg[..svg.len().min(80)]);
    assert!(svg.contains("</svg>"), "SVG must contain closing </svg> tag");

    // Must contain the hex-encoded payload somewhere in the QR structure
    let qr_hex = result["qr_hex"].as_str().unwrap();
    assert_eq!(qr_hex.len(), 74, "hex payload should be 74 chars (37 bytes)");
    assert!(hex::decode(qr_hex).is_ok(), "qr_hex must be valid hex");
}

#[tokio::test]
async fn test_export_qr_requires_unlocked_identity() {
    let svc = test_services();
    // Do NOT create/unlock identity

    let router = build_router(Arc::clone(&svc));
    let req = JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        method: "identity.export_qr".to_string(),
        params: serde_json::json!({}),
        id: Value::Number(7.into()),
    };
    let resp = router.dispatch(req).await;
    assert!(resp.error.is_some(), "should fail when identity is locked");
}

#[tokio::test]
async fn test_invite_qr_returns_svg_with_link() {
    let svc = test_services();
    svc.identity.create("test-pass").await.unwrap();

    let router = build_router(Arc::clone(&svc));
    let req = JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        method: "identity.invite_qr".to_string(),
        params: serde_json::json!({}),
        id: Value::Number(8.into()),
    };
    let resp = router.dispatch(req).await;
    assert!(resp.error.is_none(), "error: {:?}", resp.error);

    let result = resp.result.unwrap();

    // Verify SVG is present and well-formed
    let svg = result["qr_svg"].as_str().unwrap();
    assert!(svg.contains("<svg"), "invite QR must contain SVG");
    assert!(svg.contains("</svg>"), "invite QR SVG must be closed");

    // Verify invite link format
    let invite_link = result["invite_link"].as_str().unwrap();
    assert!(
        invite_link.starts_with("ephemera://connect/"),
        "invite link must start with ephemera://connect/, got: {invite_link}"
    );

    // Verify pubkey is valid hex of correct length (32 bytes = 64 hex chars)
    let pubkey = result["pubkey"].as_str().unwrap();
    assert_eq!(pubkey.len(), 64, "pubkey hex should be 64 chars");
    assert!(hex::decode(pubkey).is_ok(), "pubkey must be valid hex");

    // Invite link must end with the pubkey
    assert!(
        invite_link.ends_with(pubkey),
        "invite link must end with pubkey"
    );
}

#[tokio::test]
async fn test_import_qr_rpc() {
    let svc = test_services();

    // Create an identity and export the QR hex.
    svc.identity.create("source-pass").await.unwrap();
    let export_result = svc.identity.export_qr().await.unwrap();
    let qr_hex = export_result["qr_hex"].as_str().unwrap().to_string();

    // Lock the first identity so the import replaces it.
    svc.identity.lock(false).await.unwrap();

    // Import the QR on a fresh service container to prove round-trip works.
    let svc2 = test_services();
    let router = build_router(Arc::clone(&svc2));
    let req = JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        method: "identity.import_qr".to_string(),
        params: serde_json::json!({
            "qr_hex": qr_hex,
            "passphrase": "new-device-pass",
        }),
        id: Value::Number(10.into()),
    };
    let resp = router.dispatch(req).await;
    assert!(resp.error.is_none(), "error: {:?}", resp.error);

    let result = resp.result.unwrap();
    assert_eq!(result["imported"].as_bool(), Some(true));
    assert!(result.get("pseudonym_pubkey").is_some(), "should return pubkey");

    // Verify the imported identity is active (can call get_active).
    let active = svc2.identity.get_active().await.unwrap();
    assert!(active.get("pubkey").is_some(), "imported identity should be active");
}

#[tokio::test]
async fn test_import_qr_invalid_checksum() {
    let svc = test_services();
    svc.identity.create("test-pass").await.unwrap();
    let export_result = svc.identity.export_qr().await.unwrap();
    let qr_hex = export_result["qr_hex"].as_str().unwrap().to_string();

    // Tamper with the hex payload: flip a byte in the middle of the master secret.
    let mut bytes = hex::decode(&qr_hex).unwrap();
    bytes[16] ^= 0xFF;
    let tampered_hex = hex::encode(&bytes);

    let router = build_router(test_services());
    let req = JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        method: "identity.import_qr".to_string(),
        params: serde_json::json!({
            "qr_hex": tampered_hex,
            "passphrase": "test-pass",
        }),
        id: Value::Number(11.into()),
    };
    let resp = router.dispatch(req).await;
    assert!(resp.error.is_some(), "tampered QR data should be rejected");
    let err_msg = resp.error.unwrap().message;
    assert!(
        err_msg.contains("checksum"),
        "error should mention checksum, got: {err_msg}"
    );
}
