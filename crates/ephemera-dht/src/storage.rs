//! In-memory DHT record storage with TTL awareness.
//!
//! Stores [`DhtRecord`]s and automatically expires them based on their
//! creation timestamp + TTL. A background sweep removes expired entries.

use crate::{DhtConfig, DhtError, DhtRecord, MAX_TTL_SECONDS};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing;

/// In-memory storage for DHT records with TTL-based expiration.
pub struct DhtStorage {
    /// Records indexed by their 32-byte key.
    records: HashMap<[u8; 32], StoredRecord>,
    /// Maximum number of records.
    max_records: usize,
    /// Maximum record value size.
    max_record_size: usize,
}

/// A record together with its computed expiration timestamp.
#[derive(Debug, Clone)]
struct StoredRecord {
    /// The DHT record.
    record: DhtRecord,
    /// Unix timestamp (seconds) at which this record expires.
    expires_at: u64,
}

impl DhtStorage {
    /// Create a new storage with the given configuration.
    pub fn new(config: &DhtConfig) -> Self {
        Self {
            records: HashMap::new(),
            max_records: config.max_records,
            max_record_size: config.max_record_size,
        }
    }

    /// Store a record, validating size and TTL constraints.
    pub fn put(&mut self, record: DhtRecord) -> Result<(), DhtError> {
        // Validate record size.
        if record.value.len() > self.max_record_size {
            return Err(DhtError::RecordTooLarge {
                size: record.value.len(),
                max: self.max_record_size,
            });
        }

        // Validate TTL.
        if record.ttl_seconds > MAX_TTL_SECONDS {
            return Err(DhtError::TtlTooLarge {
                ttl_secs: record.ttl_seconds,
                max_secs: MAX_TTL_SECONDS,
            });
        }

        // Check if already expired.
        let expires_at = record.timestamp + u64::from(record.ttl_seconds);
        if expires_at < Self::now() {
            return Err(DhtError::Expired);
        }

        // Check storage capacity.
        if !self.records.contains_key(&record.key) && self.records.len() >= self.max_records {
            return Err(DhtError::StorageFull {
                count: self.records.len(),
                max: self.max_records,
            });
        }

        let key = record.key;
        self.records
            .insert(key, StoredRecord { record, expires_at });
        Ok(())
    }

    /// Retrieve a record by key, returning `None` if not found or expired.
    pub fn get(&self, key: &[u8; 32]) -> Option<&DhtRecord> {
        self.records.get(key).and_then(|stored| {
            if stored.expires_at < Self::now() {
                None // Expired but not yet cleaned up.
            } else {
                Some(&stored.record)
            }
        })
    }

    /// Remove a record by key.
    pub fn remove(&mut self, key: &[u8; 32]) -> bool {
        self.records.remove(key).is_some()
    }

    /// Number of stored records (including potentially expired ones not yet swept).
    #[must_use]
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// Whether the storage is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// Sweep expired records. Call this periodically (every 60 seconds per spec).
    ///
    /// Returns the number of records removed.
    pub fn sweep_expired(&mut self) -> usize {
        let now = Self::now();
        let before = self.records.len();
        self.records.retain(|key, stored| {
            let keep = stored.expires_at >= now;
            if !keep {
                tracing::debug!(key = hex::encode(key), "sweeping expired DHT record");
            }
            keep
        });
        before - self.records.len()
    }

    /// Get all records (for iteration, replication, etc.).
    pub fn all_records(&self) -> impl Iterator<Item = &DhtRecord> {
        let now = Self::now();
        self.records
            .values()
            .filter(move |s| s.expires_at >= now)
            .map(|s| &s.record)
    }

    /// Current Unix timestamp in seconds.
    fn now() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before Unix epoch")
            .as_secs()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DhtRecordType, MAX_RECORD_SIZE};

    fn test_config() -> DhtConfig {
        DhtConfig {
            max_records: 100,
            max_record_size: MAX_RECORD_SIZE,
            ..DhtConfig::default()
        }
    }

    fn make_record(key_byte: u8, ttl: u32) -> DhtRecord {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        DhtRecord {
            key: [key_byte; 32],
            record_type: DhtRecordType::Profile,
            value: vec![0xDE, 0xAD],
            publisher: [0; 32],
            timestamp: now,
            ttl_seconds: ttl,
            signature: vec![0; 64],
        }
    }

    fn make_expired_record(key_byte: u8) -> DhtRecord {
        DhtRecord {
            key: [key_byte; 32],
            record_type: DhtRecordType::Profile,
            value: vec![0xDE, 0xAD],
            publisher: [0; 32],
            timestamp: 1_000_000, // way in the past
            ttl_seconds: 3600,
            signature: vec![0; 64],
        }
    }

    #[test]
    fn put_and_get() {
        let mut storage = DhtStorage::new(&test_config());
        let record = make_record(1, 3600);
        storage.put(record.clone()).unwrap();
        let retrieved = storage.get(&[1; 32]).unwrap();
        assert_eq!(retrieved.value, vec![0xDE, 0xAD]);
    }

    #[test]
    fn reject_expired_on_put() {
        let mut storage = DhtStorage::new(&test_config());
        let record = make_expired_record(1);
        assert!(matches!(storage.put(record), Err(DhtError::Expired)));
    }

    #[test]
    fn reject_oversized() {
        let mut storage = DhtStorage::new(&test_config());
        let mut record = make_record(1, 3600);
        record.value = vec![0; MAX_RECORD_SIZE + 1];
        assert!(matches!(
            storage.put(record),
            Err(DhtError::RecordTooLarge { .. })
        ));
    }

    #[test]
    fn reject_ttl_too_large() {
        let mut storage = DhtStorage::new(&test_config());
        let record = make_record(1, MAX_TTL_SECONDS + 1);
        assert!(matches!(
            storage.put(record),
            Err(DhtError::TtlTooLarge { .. })
        ));
    }

    #[test]
    fn remove_record() {
        let mut storage = DhtStorage::new(&test_config());
        storage.put(make_record(1, 3600)).unwrap();
        assert!(storage.remove(&[1; 32]));
        assert!(storage.get(&[1; 32]).is_none());
    }

    #[test]
    fn sweep_expired() {
        let mut storage = DhtStorage::new(&test_config());
        // Insert a valid record.
        storage.put(make_record(1, 3600)).unwrap();
        // Manually insert an expired record by tampering with the internal map.
        let now = DhtStorage::now();
        let expired_record = DhtRecord {
            key: [2; 32],
            record_type: DhtRecordType::Profile,
            value: vec![],
            publisher: [0; 32],
            timestamp: now,
            ttl_seconds: 3600,
            signature: vec![0; 64],
        };
        storage.records.insert(
            [2; 32],
            StoredRecord {
                record: expired_record,
                expires_at: 1, // expired long ago
            },
        );
        assert_eq!(storage.len(), 2);
        let swept = storage.sweep_expired();
        assert_eq!(swept, 1);
        assert_eq!(storage.len(), 1);
    }

    #[test]
    fn storage_capacity_limit() {
        let cfg = DhtConfig {
            max_records: 2,
            ..test_config()
        };
        let mut storage = DhtStorage::new(&cfg);
        storage.put(make_record(1, 3600)).unwrap();
        storage.put(make_record(2, 3600)).unwrap();
        let result = storage.put(make_record(3, 3600));
        assert!(matches!(result, Err(DhtError::StorageFull { .. })));
    }
}
