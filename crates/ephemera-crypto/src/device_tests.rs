use super::*;

#[test]
fn test_device_registration() {
    let mut mgr = DeviceManager::new();
    assert_eq!(mgr.device_count(), 0);

    let info = mgr.register_device("Mike's Desktop", Platform::Desktop);
    assert_eq!(info.device_name, "Mike's Desktop");
    assert_eq!(info.platform, Platform::Desktop);
    assert!(info.created_at > 0);
    assert_eq!(info.created_at, info.last_seen_at);

    let devices = mgr.list_devices();
    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0].device_id, info.device_id);
    assert_eq!(devices[0].device_name, "Mike's Desktop");
}

#[test]
fn test_device_revocation() {
    let mut mgr = DeviceManager::new();
    let d1 = mgr.register_device("Desktop", Platform::Desktop);
    let d2 = mgr.register_device("Phone", Platform::Android);
    assert_eq!(mgr.device_count(), 2);

    let revoked = mgr.revoke_device(&d1.device_id).unwrap();
    assert_eq!(revoked.device_name, "Desktop");
    assert_eq!(mgr.device_count(), 1);

    // The remaining device should be the phone
    let devices = mgr.list_devices();
    assert_eq!(devices[0].device_id, d2.device_id);
    assert_eq!(devices[0].device_name, "Phone");
}

#[test]
fn test_revoke_nonexistent_device() {
    let mut mgr = DeviceManager::new();
    mgr.register_device("Desktop", Platform::Desktop);

    let fake_id = [0xFF; 16];
    let result = mgr.revoke_device(&fake_id);
    assert!(result.is_err());
}

#[test]
fn test_device_touch() {
    let mut mgr = DeviceManager::new();
    let info = mgr.register_device("Desktop", Platform::Desktop);
    let original_last_seen = info.last_seen_at;

    // Touch should update last_seen_at (may be same second)
    mgr.touch_device(&info.device_id).unwrap();
    let updated = mgr.get_device(&info.device_id).unwrap();
    assert!(updated.last_seen_at >= original_last_seen);
}

#[test]
fn test_touch_nonexistent_device() {
    let mut mgr = DeviceManager::new();
    let fake_id = [0xFF; 16];
    assert!(mgr.touch_device(&fake_id).is_err());
}

#[test]
fn test_device_unique_ids() {
    let mut mgr = DeviceManager::new();
    let d1 = mgr.register_device("A", Platform::Desktop);
    let d2 = mgr.register_device("B", Platform::Desktop);
    assert_ne!(d1.device_id, d2.device_id);
}

#[test]
fn test_device_id_hex() {
    let info = DeviceInfo {
        device_id: [0xAA; 16],
        device_name: "Test".into(),
        platform: Platform::Desktop,
        created_at: 1000,
        last_seen_at: 1000,
    };
    assert_eq!(info.device_id_hex(), "aa".repeat(16));
}

#[test]
fn test_platform_display() {
    assert_eq!(Platform::Desktop.to_string(), "Desktop");
    assert_eq!(Platform::Android.to_string(), "Android");
    assert_eq!(Platform::IOS.to_string(), "iOS");
}

#[test]
fn test_device_json_roundtrip() {
    let mut mgr = DeviceManager::new();
    mgr.register_device("Desktop", Platform::Desktop);
    mgr.register_device("Phone", Platform::Android);
    mgr.register_device("iPad", Platform::IOS);

    let json = mgr.to_json().unwrap();
    let recovered = DeviceManager::from_json(&json).unwrap();

    assert_eq!(recovered.device_count(), 3);
    let orig = mgr.list_devices();
    let rec = recovered.list_devices();
    for i in 0..3 {
        assert_eq!(orig[i].device_id, rec[i].device_id);
        assert_eq!(orig[i].device_name, rec[i].device_name);
        assert_eq!(orig[i].platform, rec[i].platform);
    }
}

#[test]
fn test_device_manager_default() {
    let mgr = DeviceManager::default();
    assert_eq!(mgr.device_count(), 0);
}

#[test]
fn test_get_device() {
    let mut mgr = DeviceManager::new();
    let info = mgr.register_device("Desktop", Platform::Desktop);

    let found = mgr.get_device(&info.device_id);
    assert!(found.is_some());
    assert_eq!(found.unwrap().device_name, "Desktop");

    let not_found = mgr.get_device(&[0xFF; 16]);
    assert!(not_found.is_none());
}

#[test]
fn test_with_devices_constructor() {
    let devices = vec![
        DeviceInfo {
            device_id: [1; 16],
            device_name: "A".into(),
            platform: Platform::Desktop,
            created_at: 100,
            last_seen_at: 200,
        },
        DeviceInfo {
            device_id: [2; 16],
            device_name: "B".into(),
            platform: Platform::Android,
            created_at: 150,
            last_seen_at: 250,
        },
    ];

    let mgr = DeviceManager::with_devices(devices);
    assert_eq!(mgr.device_count(), 2);
    assert_eq!(mgr.list_devices()[0].device_name, "A");
    assert_eq!(mgr.list_devices()[1].device_name, "B");
}
