//! Cryptographic error types.

use ephemera_types::EphemeraError;

/// Errors arising from cryptographic operations.
#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    /// Signature verification failed.
    #[error("signature verification failed")]
    SignatureInvalid,

    /// Encryption failed.
    #[error("encryption failed: {reason}")]
    EncryptionFailed {
        /// Human-readable reason.
        reason: String,
    },

    /// Decryption failed (wrong key, corrupted ciphertext, or tampered AAD).
    #[error("decryption failed: {reason}")]
    DecryptionFailed {
        /// Human-readable reason.
        reason: String,
    },

    /// Invalid key material.
    #[error("invalid key material: {reason}")]
    InvalidKey {
        /// Human-readable reason.
        reason: String,
    },
}

impl From<CryptoError> for EphemeraError {
    fn from(err: CryptoError) -> Self {
        match err {
            CryptoError::SignatureInvalid => EphemeraError::SignatureInvalid {
                reason: "signature verification failed".into(),
            },
            CryptoError::EncryptionFailed { reason } => EphemeraError::EncryptionError { reason },
            CryptoError::DecryptionFailed { reason } => EphemeraError::EncryptionError { reason },
            CryptoError::InvalidKey { reason } => EphemeraError::InvalidKey { reason },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_conversion_signature() {
        let err: EphemeraError = CryptoError::SignatureInvalid.into();
        assert!(matches!(err, EphemeraError::SignatureInvalid { .. }));
        assert!(err.is_security_violation());
    }

    #[test]
    fn test_error_conversion_encryption() {
        let err: EphemeraError = CryptoError::EncryptionFailed {
            reason: "test".into(),
        }
        .into();
        assert!(matches!(err, EphemeraError::EncryptionError { .. }));
    }

    #[test]
    fn test_error_conversion_decryption() {
        let err: EphemeraError = CryptoError::DecryptionFailed {
            reason: "bad".into(),
        }
        .into();
        assert!(matches!(err, EphemeraError::EncryptionError { .. }));
    }

    #[test]
    fn test_error_conversion_invalid_key() {
        let err: EphemeraError = CryptoError::InvalidKey {
            reason: "short".into(),
        }
        .into();
        assert!(matches!(err, EphemeraError::InvalidKey { .. }));
    }

    #[test]
    fn test_error_conversion_via_question_mark() {
        fn fallible() -> Result<(), EphemeraError> {
            Err(CryptoError::SignatureInvalid)?;
            Ok(())
        }
        assert!(fallible().is_err());
    }
}
