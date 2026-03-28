use super::*;

fn node(byte: u8) -> NodeEntry {
    NodeEntry::new(
        NodeId::from_bytes([byte; 32]),
        vec!["127.0.0.1:4433".parse().unwrap()],
    )
}

#[test]
fn bucket_insert_and_update() {
    let mut bucket = KBucket::new(3);
    assert!(bucket.is_empty());

    let result = bucket.insert(node(1));
    assert!(matches!(result, InsertResult::Inserted));
    assert_eq!(bucket.len(), 1);

    // Re-insert same node -> Update
    let result = bucket.insert(node(1));
    assert!(matches!(result, InsertResult::Updated));
    assert_eq!(bucket.len(), 1);
}

#[test]
fn bucket_full_triggers_ping() {
    let mut bucket = KBucket::new(2);
    bucket.insert(node(1));
    bucket.insert(node(2));
    let result = bucket.insert(node(3));
    assert!(matches!(result, InsertResult::BucketFull { .. }));
    assert_eq!(bucket.len(), 2);
}

#[test]
fn bucket_evict_oldest() {
    let mut bucket = KBucket::new(2);
    bucket.insert(node(1));
    bucket.insert(node(2));
    // Overflow goes to replacement cache
    bucket.insert(node(3));
    let evicted = bucket.evict_oldest();
    assert!(evicted.is_some());
    assert_eq!(*evicted.unwrap().id.as_bytes(), [1u8; 32]);
    // Node 3 should be promoted from replacement cache
    assert_eq!(bucket.len(), 2);
}

#[test]
fn routing_table_closest() {
    let local = NodeId::from_bytes([0; 32]);
    let mut rt = RoutingTable::new(local, 20);

    for i in 1..=10u8 {
        rt.insert(node(i));
    }

    let target = NodeId::from_bytes([5; 32]);
    let closest = rt.closest(&target, 3);
    assert_eq!(closest.len(), 3);
    // The first result should be the closest to the target.
    let d0 = closest[0].id.xor_distance(&target);
    let d1 = closest[1].id.xor_distance(&target);
    assert!(d0 <= d1);
}

#[test]
fn routing_table_ignores_self() {
    let local = NodeId::from_bytes([0; 32]);
    let mut rt = RoutingTable::new(local, 20);
    rt.insert(NodeEntry::new(local, vec![]));
    assert_eq!(rt.total_entries(), 0);
}
