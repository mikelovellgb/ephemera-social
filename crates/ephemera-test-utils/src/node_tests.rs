use super::*;

#[tokio::test]
async fn test_node_creates_successfully() {
    crate::init_test_tracing();
    let node = TestNode::new().await.unwrap();
    assert!(!node.is_running());
    assert!(node.data_dir().exists());
}

#[tokio::test]
async fn test_node_starts_and_stops() {
    crate::init_test_tracing();
    let mut node = TestNode::started().await.unwrap();
    assert!(node.is_running());
    node.shutdown().await.unwrap();
    assert!(!node.is_running());
}

#[tokio::test]
async fn test_node_rpc_echo() {
    crate::init_test_tracing();
    let node = TestNode::new().await.unwrap();
    let resp = node.node_rpc("meta.status", serde_json::json!({})).await;
    assert!(resp.error.is_none(), "meta.status should succeed");
}

#[tokio::test]
async fn test_node_pair_creates_two_distinct_nodes() {
    crate::init_test_tracing();
    let (a, b) = TestNode::pair().await.unwrap();
    assert_ne!(
        a.identity().node_id(),
        b.identity().node_id(),
        "pair should have different identities"
    );
}
