//! Device registration and management for multi-device identity support.
//!
//! Each device that shares the same master secret gets a unique random
//! device ID and descriptive metadata. The device list is stored locally
//! and can be synced across devices via the gossip layer.
//!
//! Since keys are derived deterministically from the master secret via HKDF,
//! any device with the master secret can regenerate the entire key hierarchy.
//! The device manager only tracks *which* devices have been authorized, not
//! the key material itself.

use ephemera_types::EphemeraError;
use rand::rngs::OsRng;
use rand::RngCore;
use serde::{Deserialize, Serialize};

/// The platform/OS a device is running on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Platform {
    /// Desktop (Windows, macOS, Linux).
    Desktop,
    /// Android phone or tablet.
    Android,
    /// Apple iOS device.
    #[allow(clippy::upper_case_acronyms)]
    IOS,
}

impl std::fmt::Display for Platform {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Desktop => write!(f, "Desktop"),
            Self::Android => write!(f, "Android"),
            Self::IOS => write!(f, "iOS"),
        }
    }
}

/// Information about a registered device.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceInfo {
    /// Unique random identifier for this device (16 bytes).
    pub device_id: [u8; 16],
    /// Human-readable name (e.g. "Mike's Desktop").
    pub device_name: String,
    /// Platform the device is running on.
    pub platform: Platform,
    /// Unix timestamp when the device was first registered.
    pub created_at: u64,
    /// Unix timestamp of the last time this device was seen active.
    pub last_seen_at: u64,
}

impl DeviceInfo {
    /// The device ID as a hex string (for display and API responses).
    #[must_use]
    pub fn device_id_hex(&self) -> String {
        hex::encode(self.device_id)
    }
}

/// Manages the set of devices authorized to use a shared identity.
///
/// The device list is held in memory and can be serialized for persistence.
/// Adding or removing a device does not affect key material -- keys are
/// derived from the master secret, not from device registrations.
#[derive(Debug)]
pub struct DeviceManager {
    /// All registered devices.
    devices: Vec<DeviceInfo>,
}

impl DeviceManager {
    /// Create a new empty device manager.
    #[must_use]
    pub fn new() -> Self {
        Self {
            devices: Vec::new(),
        }
    }

    /// Create a device manager pre-populated with existing devices.
    #[must_use]
    pub fn with_devices(devices: Vec<DeviceInfo>) -> Self {
        Self { devices }
    }

    /// Register a new device and return its info.
    ///
    /// Generates a random 16-byte device ID and records the current time
    /// as both `created_at` and `last_seen_at`.
    pub fn register_device(&mut self, name: &str, platform: Platform) -> DeviceInfo {
        let mut device_id = [0u8; 16];
        OsRng.fill_bytes(&mut device_id);

        let now = current_unix_timestamp();

        let info = DeviceInfo {
            device_id,
            device_name: name.to_string(),
            platform,
            created_at: now,
            last_seen_at: now,
        };

        self.devices.push(info.clone());
        info
    }

    /// List all registered devices.
    #[must_use]
    pub fn list_devices(&self) -> &[DeviceInfo] {
        &self.devices
    }

    /// Update the `last_seen_at` timestamp for a device.
    ///
    /// # Errors
    ///
    /// Returns an error if the device ID is not found.
    pub fn touch_device(&mut self, device_id: &[u8; 16]) -> Result<(), EphemeraError> {
        let device = self
            .devices
            .iter_mut()
            .find(|d| &d.device_id == device_id)
            .ok_or_else(|| EphemeraError::InvalidKey {
                reason: format!(
                    "device not found: {}",
                    hex::encode(device_id)
                ),
            })?;

        device.last_seen_at = current_unix_timestamp();
        Ok(())
    }

    /// Revoke (remove) a device by its ID.
    ///
    /// This only removes the device record -- it does NOT invalidate keys,
    /// because keys are derived from the master secret, not from device
    /// registrations. To fully revoke a compromised device, the user would
    /// need to rotate their master secret.
    ///
    /// # Errors
    ///
    /// Returns an error if the device ID is not found.
    pub fn revoke_device(&mut self, device_id: &[u8; 16]) -> Result<DeviceInfo, EphemeraError> {
        let pos = self
            .devices
            .iter()
            .position(|d| &d.device_id == device_id)
            .ok_or_else(|| EphemeraError::InvalidKey {
                reason: format!(
                    "device not found: {}",
                    hex::encode(device_id)
                ),
            })?;

        Ok(self.devices.remove(pos))
    }

    /// Get a device by its ID.
    #[must_use]
    pub fn get_device(&self, device_id: &[u8; 16]) -> Option<&DeviceInfo> {
        self.devices.iter().find(|d| &d.device_id == device_id)
    }

    /// The number of registered devices.
    #[must_use]
    pub fn device_count(&self) -> usize {
        self.devices.len()
    }

    /// Serialize the device list for persistence.
    ///
    /// # Errors
    ///
    /// Returns an error if serialization fails.
    pub fn to_json(&self) -> Result<Vec<u8>, EphemeraError> {
        serde_json::to_vec(&self.devices).map_err(|e| EphemeraError::SerializationError {
            reason: format!("device list serialization failed: {e}"),
        })
    }

    /// Deserialize a device list from JSON.
    ///
    /// # Errors
    ///
    /// Returns an error if deserialization fails.
    pub fn from_json(data: &[u8]) -> Result<Self, EphemeraError> {
        let devices: Vec<DeviceInfo> =
            serde_json::from_slice(data).map_err(|e| EphemeraError::SerializationError {
                reason: format!("device list deserialization failed: {e}"),
            })?;
        Ok(Self { devices })
    }
}

impl Default for DeviceManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Get the current Unix timestamp in seconds.
fn current_unix_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
#[path = "device_tests.rs"]
mod tests;
