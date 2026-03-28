//! Content-addressed blob storage on the filesystem.
//!
//! Blobs are stored using BLAKE3 hashes with a 2-char directory prefix.
//! Day-partitioned blobs go under `content/<YYYY-MM-DD>/` for bulk expiry.

use crate::StoreError;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Global atomic counter for generating unique temporary file suffixes.
/// Combined with process ID, this ensures no two concurrent writes collide.
static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Content-addressed filesystem blob store with optional encryption at rest.
pub struct ContentStore {
    base_dir: PathBuf,
    /// Optional encryption key for at-rest encryption.
    /// When `Some`, blobs are sealed before writing and opened after reading.
    encryption_key: Option<[u8; 32]>,
}

impl Drop for ContentStore {
    fn drop(&mut self) {
        // Zeroize the encryption key on drop.
        if let Some(ref mut key) = self.encryption_key {
            zeroize::Zeroize::zeroize(key);
        }
    }
}

impl ContentStore {
    /// Open (or create) a content store rooted at `base_dir` (no encryption).
    pub fn open(base_dir: impl Into<PathBuf>) -> Result<Self, StoreError> {
        let base_dir = base_dir.into();
        std::fs::create_dir_all(&base_dir)?;
        Ok(Self {
            base_dir,
            encryption_key: None,
        })
    }

    /// Open (or create) a content store with at-rest encryption.
    ///
    /// All blobs will be sealed with XChaCha20-Poly1305 before writing
    /// and opened when reading. The content hash is still computed over
    /// the plaintext.
    pub fn open_encrypted(base_dir: impl Into<PathBuf>, key: [u8; 32]) -> Result<Self, StoreError> {
        let base_dir = base_dir.into();
        std::fs::create_dir_all(&base_dir)?;
        Ok(Self {
            base_dir,
            encryption_key: Some(key),
        })
    }

    /// Seal plaintext if an encryption key is configured.
    fn seal_if_encrypted(&self, data: &[u8]) -> Result<Vec<u8>, StoreError> {
        match &self.encryption_key {
            Some(key) => ephemera_crypto::encryption::seal(key, data)
                .map_err(|e| StoreError::Integrity(format!("encryption failed: {e}"))),
            None => Ok(data.to_vec()),
        }
    }

    /// Open ciphertext if an encryption key is configured.
    fn open_if_encrypted(&self, data: &[u8]) -> Result<Vec<u8>, StoreError> {
        match &self.encryption_key {
            Some(key) => ephemera_crypto::encryption::open(key, data)
                .map_err(|e| StoreError::Integrity(format!("decryption failed: {e}"))),
            None => Ok(data.to_vec()),
        }
    }

    /// The root directory of this store.
    #[must_use]
    pub fn base_dir(&self) -> &Path {
        &self.base_dir
    }

    /// Store a blob. Returns the BLAKE3 hex hash used as the key.
    ///
    /// The hash is computed over the **plaintext**. If an encryption key
    /// is configured, the data is encrypted before being written to disk.
    ///
    /// If a blob with the same hash already exists the call is a no-op
    /// (content-addressing makes duplicates safe to skip).
    pub fn put(&self, data: &[u8]) -> Result<String, StoreError> {
        let hash = blake3::hash(data);
        let hex = hash.to_hex().to_string();
        let path = self.hash_to_path(&hex);

        if path.exists() {
            return Ok(hex);
        }

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Encrypt before writing if a key is configured.
        let bytes_to_write = self.seal_if_encrypted(data)?;

        // Write to a uniquely-named temporary file, then rename for atomicity.
        // The unique suffix (PID + atomic counter) prevents races when two
        // concurrent writes target the same content hash.
        let tmp_path = Self::unique_tmp_path(&path);
        std::fs::write(&tmp_path, &bytes_to_write)?;
        std::fs::rename(&tmp_path, &path)?;

        Ok(hex)
    }

    /// Store a blob into a date-partitioned directory.
    ///
    /// The blob is stored at `<base_dir>/content/<date_str>/<hex_hash>.blob`.
    /// Returns the BLAKE3 hex hash (computed over plaintext).
    ///
    /// `date_str` should be in `YYYY-MM-DD` format.
    pub fn put_partitioned(&self, data: &[u8], date_str: &str) -> Result<String, StoreError> {
        let hash = blake3::hash(data);
        let hex = hash.to_hex().to_string();
        let path = self.date_partition_path(date_str, &hex);

        if path.exists() {
            return Ok(hex);
        }

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let bytes_to_write = self.seal_if_encrypted(data)?;

        let tmp_path = Self::unique_tmp_path(&path);
        std::fs::write(&tmp_path, &bytes_to_write)?;
        std::fs::rename(&tmp_path, &path)?;

        Ok(hex)
    }

    /// Generate a unique temporary file path for atomic write-then-rename.
    ///
    /// Uses PID + atomic counter to guarantee uniqueness even under
    /// concurrent writes from the same process.
    fn unique_tmp_path(final_path: &Path) -> PathBuf {
        let seq = TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        final_path.with_extension(format!("tmp.{pid}.{seq}"))
    }

    /// Retrieve a blob from a date-partitioned directory.
    ///
    /// If encryption is enabled, the stored ciphertext is decrypted
    /// before being returned.
    pub fn get_partitioned(&self, date_str: &str, hex_hash: &str) -> Result<Vec<u8>, StoreError> {
        let path = self.date_partition_path(date_str, hex_hash);
        if !path.exists() {
            return Err(StoreError::NotFound(format!(
                "blob {hex_hash} in partition {date_str}"
            )));
        }
        let raw = std::fs::read(&path)?;
        self.open_if_encrypted(&raw)
    }

    /// Check whether a blob exists in a date-partitioned directory.
    #[must_use]
    pub fn exists_partitioned(&self, date_str: &str, hex_hash: &str) -> bool {
        self.date_partition_path(date_str, hex_hash).exists()
    }

    /// Delete a blob from a date-partitioned directory.
    pub fn delete_partitioned(&self, date_str: &str, hex_hash: &str) -> Result<bool, StoreError> {
        let path = self.date_partition_path(date_str, hex_hash);
        if !path.exists() {
            return Ok(false);
        }
        std::fs::remove_file(&path)?;
        Ok(true)
    }

    /// Delete an entire day directory if it exists.
    ///
    /// Returns `Ok(true)` if the directory was removed, `Ok(false)` if it
    /// did not exist.
    pub fn delete_day_partition(&self, date_str: &str) -> Result<bool, StoreError> {
        let dir = self.base_dir.join("content").join(date_str);
        if !dir.exists() {
            return Ok(false);
        }
        std::fs::remove_dir_all(&dir)?;
        Ok(true)
    }

    /// Check whether a day partition directory exists.
    #[must_use]
    pub fn day_partition_exists(&self, date_str: &str) -> bool {
        self.base_dir.join("content").join(date_str).exists()
    }

    /// List all day partition directory names (e.g., `["2026-03-25", "2026-03-26"]`).
    pub fn list_day_partitions(&self) -> Result<Vec<String>, StoreError> {
        let content_dir = self.base_dir.join("content");
        if !content_dir.exists() {
            return Ok(Vec::new());
        }

        let mut partitions = Vec::new();
        for entry in std::fs::read_dir(&content_dir)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                if let Some(name) = entry.file_name().to_str() {
                    // Validate date format: YYYY-MM-DD.
                    if name.len() == 10 && name.as_bytes()[4] == b'-' && name.as_bytes()[7] == b'-'
                    {
                        partitions.push(name.to_string());
                    }
                }
            }
        }
        partitions.sort();
        Ok(partitions)
    }

    /// Check whether a day partition directory is empty.
    pub fn is_day_partition_empty(&self, date_str: &str) -> Result<bool, StoreError> {
        let dir = self.base_dir.join("content").join(date_str);
        if !dir.exists() {
            return Ok(true);
        }
        let mut entries = std::fs::read_dir(&dir)?;
        Ok(entries.next().is_none())
    }

    /// Convert a Unix timestamp to a date string for partitioning.
    #[must_use]
    pub fn date_str_from_timestamp(unix_secs: u64) -> String {
        let dt = chrono::DateTime::from_timestamp(unix_secs as i64, 0)
            .unwrap_or_else(|| chrono::DateTime::from_timestamp(0, 0).unwrap());
        dt.format("%Y-%m-%d").to_string()
    }

    /// Retrieve a blob by its hex hash. Returns `StoreError::NotFound` if
    /// the blob does not exist, `StoreError::Integrity` if the hash does
    /// not match on read-back.
    ///
    /// If encryption is enabled, the stored ciphertext is decrypted first,
    /// then the plaintext hash is verified.
    pub fn get(&self, hex_hash: &str) -> Result<Vec<u8>, StoreError> {
        let path = self.hash_to_path(hex_hash);
        if !path.exists() {
            return Err(StoreError::NotFound(format!("blob {hex_hash}")));
        }

        let raw = std::fs::read(&path)?;
        let data = self.open_if_encrypted(&raw)?;

        // Verify integrity against plaintext.
        let actual = blake3::hash(&data).to_hex().to_string();
        if actual != hex_hash {
            return Err(StoreError::Integrity(format!(
                "hash mismatch: expected {hex_hash}, got {actual}"
            )));
        }

        Ok(data)
    }

    /// Check whether a blob with the given hex hash exists.
    #[must_use]
    pub fn exists(&self, hex_hash: &str) -> bool {
        self.hash_to_path(hex_hash).exists()
    }

    /// Delete a blob by its hex hash. Returns `Ok(true)` if a file was
    /// removed, `Ok(false)` if no such blob existed.
    pub fn delete(&self, hex_hash: &str) -> Result<bool, StoreError> {
        let path = self.hash_to_path(hex_hash);
        if !path.exists() {
            return Ok(false);
        }
        std::fs::remove_file(&path)?;

        // Remove the parent directory if it is now empty (best-effort).
        if let Some(parent) = path.parent() {
            let _ = std::fs::remove_dir(parent); // fails silently if not empty
        }

        Ok(true)
    }

    /// Map a hex hash to a filesystem path (hash-based layout).
    fn hash_to_path(&self, hex_hash: &str) -> PathBuf {
        let (prefix, rest) = hex_hash.split_at(2.min(hex_hash.len()));
        self.base_dir.join(prefix).join(format!("{rest}.blob"))
    }

    /// Map a date + hex hash to a filesystem path (date-partitioned layout).
    fn date_partition_path(&self, date_str: &str, hex_hash: &str) -> PathBuf {
        self.base_dir
            .join("content")
            .join(date_str)
            .join(format!("{hex_hash}.blob"))
    }
}

#[cfg(test)]
#[path = "content_store_tests.rs"]
mod tests;
