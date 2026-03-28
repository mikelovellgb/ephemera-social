//! RPC authentication via a local cookie token.
//!
//! On node startup, a random 32-byte token is generated and written to
//! `{data_dir}/rpc_token` as hex. The client reads this file and includes
//! the token as `Authorization: Bearer <hex_token>` in every RPC request.
//!
//! This is the same pattern used by Bitcoin Core's cookie authentication:
//! it authenticates any process that can read the data directory, which is
//! appropriate for a localhost-only service.

use std::path::{Path, PathBuf};
use subtle::ConstantTimeEq;

/// The filename where the RPC authentication token is stored.
const RPC_TOKEN_FILENAME: &str = "rpc_token";

/// Length of the RPC authentication token in bytes.
const RPC_TOKEN_LEN: usize = 32;

/// Manages the RPC authentication token lifecycle.
#[derive(Clone)]
pub struct RpcAuth {
    /// The raw 32-byte token.
    token: [u8; RPC_TOKEN_LEN],
    /// Path to the token file on disk.
    token_path: PathBuf,
}

impl RpcAuth {
    /// Generate a new random token and write it to `{data_dir}/rpc_token`.
    ///
    /// # Errors
    ///
    /// Returns an error if the token file cannot be written.
    pub fn generate(data_dir: &Path) -> Result<Self, std::io::Error> {
        let mut token = [0u8; RPC_TOKEN_LEN];
        rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut token);
        let token_path = data_dir.join(RPC_TOKEN_FILENAME);

        // Write hex-encoded token to disk.
        std::fs::write(&token_path, hex::encode(token))?;

        tracing::info!(
            path = %token_path.display(),
            "RPC authentication token written"
        );

        Ok(Self { token, token_path })
    }

    /// Load an existing token from `{data_dir}/rpc_token`.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read or the token is malformed.
    pub fn load(data_dir: &Path) -> Result<Self, std::io::Error> {
        let token_path = data_dir.join(RPC_TOKEN_FILENAME);
        let hex_str = std::fs::read_to_string(&token_path)?;
        let hex_str = hex_str.trim();

        let bytes = hex::decode(hex_str).map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("invalid hex in rpc_token: {e}"),
            )
        })?;

        if bytes.len() != RPC_TOKEN_LEN {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "rpc_token has wrong length: expected {RPC_TOKEN_LEN} bytes, got {}",
                    bytes.len()
                ),
            ));
        }

        let mut token = [0u8; RPC_TOKEN_LEN];
        token.copy_from_slice(&bytes);

        Ok(Self { token, token_path })
    }

    /// Validate a `Bearer <hex_token>` authorization header value.
    ///
    /// Uses constant-time comparison to prevent timing side-channels.
    pub fn validate_bearer(&self, header_value: &str) -> bool {
        let stripped = header_value.strip_prefix("Bearer ").unwrap_or("");
        let Ok(candidate) = hex::decode(stripped.trim()) else {
            return false;
        };
        if candidate.len() != RPC_TOKEN_LEN {
            return false;
        }
        self.token.ct_eq(&candidate).unwrap_u8() == 1
    }

    /// Return the hex-encoded token string (for the client to use).
    #[must_use]
    pub fn token_hex(&self) -> String {
        hex::encode(self.token)
    }

    /// Remove the token file from disk (called during shutdown).
    pub fn cleanup(&self) {
        if self.token_path.exists() {
            let _ = std::fs::remove_file(&self.token_path);
        }
    }
}

impl Drop for RpcAuth {
    fn drop(&mut self) {
        // Zeroize the token from memory.
        zeroize::Zeroize::zeroize(&mut self.token);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_and_validate() {
        let dir = tempfile::tempdir().unwrap();
        let auth = RpcAuth::generate(dir.path()).unwrap();

        // Valid bearer
        let bearer = format!("Bearer {}", auth.token_hex());
        assert!(auth.validate_bearer(&bearer));

        // Invalid bearer
        assert!(!auth.validate_bearer(
            "Bearer 0000000000000000000000000000000000000000000000000000000000000000"
        ));
        assert!(!auth.validate_bearer("Bearer bad"));
        assert!(!auth.validate_bearer(""));
    }

    #[test]
    fn load_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let auth1 = RpcAuth::generate(dir.path()).unwrap();

        let auth2 = RpcAuth::load(dir.path()).unwrap();
        assert_eq!(auth1.token_hex(), auth2.token_hex());
    }

    #[test]
    fn cleanup_removes_file() {
        let dir = tempfile::tempdir().unwrap();
        let auth = RpcAuth::generate(dir.path()).unwrap();
        let path = dir.path().join(RPC_TOKEN_FILENAME);
        assert!(path.exists());

        auth.cleanup();
        assert!(!path.exists());
    }
}
