//! Keystore, pseudonym, and multi-device identity management.
//!
//! Manages the node's key hierarchy: master secret, node identity key,
//! derived pseudonym key pairs, key export/import, and device registration.

use super::error::{MutexResultExt, NodeServiceError};
use ephemera_crypto::{
    derive_pseudonym_key, load_keystore, save_keystore, DeviceManager, KeyExport,
    KeystoreContents, MasterSecret, Platform, PseudonymEntry, SigningKeyPair,
};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Mutex;

/// Manages identity key material, key export/import, and device registration.
pub struct IdentityService {
    pub(crate) keystore_path: PathBuf,
    pub(crate) active_keypair: Mutex<Option<SigningKeyPair>>,
    pub(crate) active_index: Mutex<u32>,
    pub(crate) master_secret: Mutex<Option<MasterSecret>>,
    pub(crate) pseudonym_count: Mutex<u32>,
    pub(crate) device_manager: Mutex<DeviceManager>,
}

impl IdentityService {
    /// Create a new identity: generate master secret, derive first pseudonym,
    /// save encrypted keystore.
    pub async fn create(&self, passphrase: &str) -> Result<Value, String> {
        let master = MasterSecret::generate();
        let pseudonym_kp =
            derive_pseudonym_key(master.as_bytes(), 0).map_err(NodeServiceError::Crypto)?;
        let pubkey_hex = hex::encode(pseudonym_kp.public_bytes());
        let signing_kp = SigningKeyPair::from_bytes(&pseudonym_kp.secret_bytes());
        let node_kp = ephemera_crypto::keys::KeyPair::generate();
        let contents = KeystoreContents {
            master_secret: *master.as_bytes(),
            node_secret: *node_kp.secret_bytes(),
            pseudonym_secrets: vec![PseudonymEntry {
                index: 0,
                secret: *pseudonym_kp.secret_bytes(),
            }],
        };
        save_keystore(&self.keystore_path, passphrase.as_bytes(), &contents)
            .map_err(|e| NodeServiceError::Keystore(format!("save failed: {e}")))?;
        *self.active_keypair.lock().map_mutex_err("active_keypair")? = Some(signing_kp);
        *self.master_secret.lock().map_mutex_err("master_secret")? = Some(master);
        *self.pseudonym_count.lock().map_mutex_err("pseudonym_count")? = 1;
        Ok(serde_json::json!({ "pseudonym_pubkey": pubkey_hex, "pseudonym_index": 0 }))
    }

    /// Unlock the keystore with the given passphrase and load keys into memory.
    pub async fn unlock(&self, passphrase: &str) -> Result<Value, String> {
        let contents = load_keystore(&self.keystore_path, passphrase.as_bytes())
            .map_err(|e| NodeServiceError::Keystore(format!("unlock failed: {e}")))?;
        let master = MasterSecret::from_bytes(contents.master_secret);
        let pseudonym_count = contents.pseudonym_secrets.len() as u32;
        let active_entry = contents
            .pseudonym_secrets
            .first()
            .ok_or_else(|| NodeServiceError::Keystore("keystore has no pseudonyms".into()))?;
        let signing_kp = SigningKeyPair::from_bytes(&active_entry.secret);
        *self.active_keypair.lock().map_mutex_err("active_keypair")? = Some(signing_kp);
        *self.active_index.lock().map_mutex_err("active_index")? = active_entry.index;
        *self.master_secret.lock().map_mutex_err("master_secret")? = Some(master);
        *self.pseudonym_count.lock().map_mutex_err("pseudonym_count")? = pseudonym_count;
        Ok(serde_json::json!({ "unlocked": true, "pseudonym_count": pseudonym_count }))
    }

    /// Lock the keystore: wipe keys from memory.
    pub async fn lock(&self) -> Result<Value, String> {
        *self.active_keypair.lock().map_mutex_err("active_keypair")? = None;
        *self.master_secret.lock().map_mutex_err("master_secret")? = None;
        Ok(serde_json::json!({ "locked": true }))
    }

    /// Check whether a keystore file exists on disk (no unlock required).
    pub fn has_keystore(&self) -> bool {
        self.keystore_path.exists()
    }

    /// Get the active pseudonym's public key.
    pub async fn get_active(&self) -> Result<Value, String> {
        let kp_guard = self.active_keypair.lock().map_mutex_err("active_keypair")?;
        let kp = kp_guard.as_ref().ok_or(NodeServiceError::IdentityLocked)?;
        let pubkey_hex = hex::encode(kp.public_key().as_bytes());
        let idx = *self.active_index.lock().map_mutex_err("active_index")?;
        Ok(serde_json::json!({ "pubkey": pubkey_hex, "index": idx }))
    }

    /// List all pseudonym indices and their public keys.
    pub async fn list_pseudonyms(&self) -> Result<Value, String> {
        let ms_guard = self.master_secret.lock().map_mutex_err("master_secret")?;
        let master = ms_guard.as_ref().ok_or(NodeServiceError::IdentityLocked)?;
        let count = *self.pseudonym_count.lock().map_mutex_err("pseudonym_count")?;
        let mut pseudonyms = Vec::new();
        for i in 0..count {
            let kp =
                derive_pseudonym_key(master.as_bytes(), i).map_err(NodeServiceError::Crypto)?;
            pseudonyms.push(serde_json::json!({
                "index": i, "pubkey": hex::encode(kp.public_bytes()),
            }));
        }
        Ok(serde_json::json!({ "pseudonyms": pseudonyms }))
    }

    /// Switch to a different pseudonym by index.
    pub async fn switch_pseudonym(&self, index: u64) -> Result<Value, String> {
        let ms_guard = self.master_secret.lock().map_mutex_err("master_secret")?;
        let master = ms_guard.as_ref().ok_or(NodeServiceError::IdentityLocked)?;
        let kp = derive_pseudonym_key(master.as_bytes(), index as u32)
            .map_err(NodeServiceError::Crypto)?;
        let signing_kp = SigningKeyPair::from_bytes(&kp.secret_bytes());
        let pubkey_hex = hex::encode(kp.public_bytes());
        drop(ms_guard);
        *self.active_keypair.lock().map_mutex_err("active_keypair")? = Some(signing_kp);
        *self.active_index.lock().map_mutex_err("active_index")? = index as u32;
        Ok(serde_json::json!({ "switched": true, "pubkey": pubkey_hex }))
    }

    /// Export the master secret as a 24-word BIP39 mnemonic.
    pub async fn export_mnemonic(&self) -> Result<Value, String> {
        let ms_guard = self.master_secret.lock().map_mutex_err("master_secret")?;
        let master = ms_guard.as_ref().ok_or(NodeServiceError::IdentityLocked)?;
        Ok(serde_json::json!({ "mnemonic": KeyExport::to_mnemonic(master) }))
    }

    /// Export the master secret as a QR code SVG image.
    ///
    /// Returns `qr_svg` (renderable SVG string), `qr_hex` (hex-encoded raw
    /// payload for programmatic use), and `length` (byte count of the raw
    /// payload).
    pub async fn export_qr(&self) -> Result<Value, String> {
        use qrcode::QrCode;
        use qrcode::render::svg;

        let ms_guard = self.master_secret.lock().map_mutex_err("master_secret")?;
        let master = ms_guard.as_ref().ok_or(NodeServiceError::IdentityLocked)?;
        let qr_bytes = KeyExport::to_qr_bytes(master);
        let hex_payload = hex::encode(&qr_bytes);

        let code = QrCode::new(hex_payload.as_bytes())
            .map_err(|e| format!("QR encode error: {e}"))?;
        let svg_string = code
            .render::<svg::Color>()
            .min_dimensions(200, 200)
            .build();

        Ok(serde_json::json!({
            "qr_svg": svg_string,
            "qr_hex": hex_payload,
            "length": qr_bytes.len(),
        }))
    }

    /// Generate a QR code SVG for the invite link `ephemera://connect/<pubkey_hex>`.
    ///
    /// Returns `qr_svg` (renderable SVG), `invite_link` (the encoded URI), and
    /// `pubkey` (hex-encoded public key).
    pub async fn invite_qr(&self) -> Result<Value, String> {
        use qrcode::QrCode;
        use qrcode::render::svg;

        let kp_guard = self.active_keypair.lock().map_mutex_err("active_keypair")?;
        let kp = kp_guard.as_ref().ok_or(NodeServiceError::IdentityLocked)?;
        let pubkey_hex = hex::encode(kp.public_key().as_bytes());
        let invite_link = format!("ephemera://connect/{pubkey_hex}");

        let code = QrCode::new(invite_link.as_bytes())
            .map_err(|e| format!("QR encode error: {e}"))?;
        let svg_string = code
            .render::<svg::Color>()
            .min_dimensions(200, 200)
            .build();

        Ok(serde_json::json!({
            "qr_svg": svg_string,
            "invite_link": invite_link,
            "pubkey": pubkey_hex,
        }))
    }

    /// Export as an encrypted backup (hex-encoded).
    pub async fn export_backup(&self, passphrase: &str) -> Result<Value, String> {
        let ms_guard = self.master_secret.lock().map_mutex_err("master_secret")?;
        let master = ms_guard.as_ref().ok_or(NodeServiceError::IdentityLocked)?;
        let backup = KeyExport::to_encrypted_backup(master, passphrase)
            .map_err(NodeServiceError::Crypto)?;
        Ok(serde_json::json!({ "backup_hex": hex::encode(&backup), "length": backup.len() }))
    }

    /// Import identity from a BIP39 mnemonic and save to a new keystore.
    pub async fn import_mnemonic(
        &self,
        words: &[String],
        passphrase: &str,
    ) -> Result<Value, String> {
        let master = KeyExport::from_mnemonic(words).map_err(NodeServiceError::Crypto)?;
        self.import_master_secret(master, passphrase).await
    }

    /// Import identity from an encrypted backup and save to a new keystore.
    pub async fn import_backup(
        &self,
        backup_hex: &str,
        backup_passphrase: &str,
        new_passphrase: &str,
    ) -> Result<Value, String> {
        let data = hex::decode(backup_hex).map_err(NodeServiceError::HexDecode)?;
        let master = KeyExport::from_encrypted_backup(&data, backup_passphrase)
            .map_err(NodeServiceError::Crypto)?;
        self.import_master_secret(master, new_passphrase).await
    }

    async fn import_master_secret(
        &self,
        master: MasterSecret,
        passphrase: &str,
    ) -> Result<Value, String> {
        let pseudonym_kp =
            derive_pseudonym_key(master.as_bytes(), 0).map_err(NodeServiceError::Crypto)?;
        let pubkey_hex = hex::encode(pseudonym_kp.public_bytes());
        let signing_kp = SigningKeyPair::from_bytes(&pseudonym_kp.secret_bytes());
        let node_kp = ephemera_crypto::keys::KeyPair::generate();
        let contents = KeystoreContents {
            master_secret: *master.as_bytes(),
            node_secret: *node_kp.secret_bytes(),
            pseudonym_secrets: vec![PseudonymEntry {
                index: 0,
                secret: *pseudonym_kp.secret_bytes(),
            }],
        };
        save_keystore(&self.keystore_path, passphrase.as_bytes(), &contents)
            .map_err(|e| NodeServiceError::Keystore(format!("save failed: {e}")))?;
        *self.active_keypair.lock().map_mutex_err("active_keypair")? = Some(signing_kp);
        *self.master_secret.lock().map_mutex_err("master_secret")? = Some(master);
        *self.active_index.lock().map_mutex_err("active_index")? = 0;
        *self.pseudonym_count.lock().map_mutex_err("pseudonym_count")? = 1;
        Ok(serde_json::json!({
            "imported": true, "pseudonym_pubkey": pubkey_hex, "pseudonym_index": 0,
        }))
    }

    /// Register this device with a name and platform.
    pub async fn register_device(&self, name: &str, platform: &str) -> Result<Value, String> {
        let plat = parse_platform(platform)?;
        let mut dm = self.device_manager.lock().map_mutex_err("device_manager")?;
        let info = dm.register_device(name, plat);
        Ok(serde_json::json!({
            "device_id": info.device_id_hex(),
            "device_name": info.device_name,
            "platform": format!("{}", info.platform),
            "created_at": info.created_at,
        }))
    }

    /// List all registered devices.
    pub async fn list_devices(&self) -> Result<Value, String> {
        let dm = self.device_manager.lock().map_mutex_err("device_manager")?;
        let devices: Vec<Value> = dm
            .list_devices()
            .iter()
            .map(|d| serde_json::json!({
                "device_id": d.device_id_hex(),
                "device_name": d.device_name,
                "platform": format!("{}", d.platform),
                "created_at": d.created_at,
                "last_seen_at": d.last_seen_at,
            }))
            .collect();
        Ok(serde_json::json!({ "devices": devices }))
    }

    /// Revoke a device by its hex-encoded ID.
    pub async fn revoke_device(&self, device_id_hex: &str) -> Result<Value, String> {
        let id_bytes = hex::decode(device_id_hex).map_err(NodeServiceError::HexDecode)?;
        if id_bytes.len() != 16 {
            return Err(NodeServiceError::InvalidInput(format!(
                "device_id must be 16 bytes, got {}", id_bytes.len()
            )).into());
        }
        let mut device_id = [0u8; 16];
        device_id.copy_from_slice(&id_bytes);
        let mut dm = self.device_manager.lock().map_mutex_err("device_manager")?;
        let revoked = dm.revoke_device(&device_id).map_err(NodeServiceError::Crypto)?;
        Ok(serde_json::json!({ "revoked": true, "device_name": revoked.device_name }))
    }

    /// Import identity from QR-encoded hex bytes and save to a new keystore.
    ///
    /// Accepts `qr_hex` (hex-encoded QR payload from `identity.export_qr`)
    /// and a `passphrase` to encrypt the new keystore.
    pub async fn import_qr(
        &self,
        qr_hex: &str,
        passphrase: &str,
    ) -> Result<Value, String> {
        let data = hex::decode(qr_hex).map_err(NodeServiceError::HexDecode)?;
        let master = KeyExport::from_qr_bytes(&data).map_err(NodeServiceError::Crypto)?;
        self.import_master_secret(master, passphrase).await
    }

    /// Backward-compatible mnemonic backup (delegates to export_mnemonic).
    pub async fn backup_mnemonic(&self, _pass: &str) -> Result<Value, String> {
        self.export_mnemonic().await
    }

    pub(crate) fn get_signing_keypair(&self) -> Result<SigningKeyPair, String> {
        let kp_guard = self.active_keypair.lock().map_mutex_err("active_keypair")?;
        let kp = kp_guard.as_ref().ok_or(NodeServiceError::IdentityLocked)?;
        Ok(SigningKeyPair::from_bytes(&kp.secret_bytes()))
    }
}

fn parse_platform(s: &str) -> Result<Platform, NodeServiceError> {
    match s.to_lowercase().as_str() {
        "desktop" => Ok(Platform::Desktop),
        "android" => Ok(Platform::Android),
        "ios" => Ok(Platform::IOS),
        _ => Err(NodeServiceError::InvalidInput(format!(
            "unknown platform: {s} (expected: desktop, android, ios)"
        ))),
    }
}
