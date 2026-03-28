use super::*;
use crate::keys::{derive_pseudonym_key, MasterSecret};

#[test]
fn test_mnemonic_export_import_roundtrip() {
    let master = MasterSecret::generate();
    let words = KeyExport::to_mnemonic(&master);

    assert_eq!(words.len(), 24, "mnemonic must be 24 words");

    let recovered = KeyExport::from_mnemonic(&words).unwrap();
    assert_eq!(
        master.as_bytes(),
        recovered.as_bytes(),
        "recovered master secret must match original"
    );

    // Verify derived keys also match
    let kp_orig = derive_pseudonym_key(master.as_bytes(), 0).unwrap();
    let kp_recovered = derive_pseudonym_key(recovered.as_bytes(), 0).unwrap();
    assert_eq!(kp_orig.public_bytes(), kp_recovered.public_bytes());
}

#[test]
fn test_mnemonic_words_are_valid_bip39() {
    let master = MasterSecret::generate();
    let words = KeyExport::to_mnemonic(&master);

    for word in &words {
        let lower = word.to_lowercase();
        assert!(
            WORDLIST.contains(&lower.as_str()),
            "word '{word}' not in BIP39 wordlist"
        );
    }
}

#[test]
fn test_mnemonic_wrong_word_count() {
    let words: Vec<String> = vec!["abandon".into(); 12];
    let result = KeyExport::from_mnemonic(&words);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("24"), "error should mention expected count");
}

#[test]
fn test_mnemonic_unknown_word() {
    let mut words: Vec<String> = vec!["abandon".into(); 24];
    words[5] = "notaword".into();
    let result = KeyExport::from_mnemonic(&words);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("notaword"),
        "error should mention the bad word"
    );
}

#[test]
fn test_mnemonic_deterministic() {
    let master = MasterSecret::from_bytes([0x42; 32]);
    let words1 = KeyExport::to_mnemonic(&master);
    let words2 = KeyExport::to_mnemonic(&master);
    assert_eq!(words1, words2, "same master secret must produce same mnemonic");
}

#[test]
fn test_qr_export_import_roundtrip() {
    let master = MasterSecret::generate();
    let qr_bytes = KeyExport::to_qr_bytes(&master);

    // Format: version(1) + master_secret(32) + checksum(4) = 37
    assert_eq!(qr_bytes.len(), 37);
    assert_eq!(qr_bytes[0], QR_VERSION);

    let recovered = KeyExport::from_qr_bytes(&qr_bytes).unwrap();
    assert_eq!(master.as_bytes(), recovered.as_bytes());

    // Verify derived keys match
    let kp_orig = derive_pseudonym_key(master.as_bytes(), 0).unwrap();
    let kp_recovered = derive_pseudonym_key(recovered.as_bytes(), 0).unwrap();
    assert_eq!(kp_orig.public_bytes(), kp_recovered.public_bytes());
}

#[test]
fn test_qr_checksum_validation() {
    let master = MasterSecret::generate();
    let mut qr_bytes = KeyExport::to_qr_bytes(&master);

    // Tamper with one byte in the master secret portion
    qr_bytes[16] ^= 0xFF;

    let result = KeyExport::from_qr_bytes(&qr_bytes);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("checksum"),
        "error should mention checksum mismatch"
    );
}

#[test]
fn test_qr_wrong_version() {
    let master = MasterSecret::generate();
    let mut qr_bytes = KeyExport::to_qr_bytes(&master);
    qr_bytes[0] = 0xFF; // bad version
    let result = KeyExport::from_qr_bytes(&qr_bytes);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("version"), "error should mention version");
}

#[test]
fn test_qr_wrong_length() {
    let result = KeyExport::from_qr_bytes(&[0x01; 10]);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("length"), "error should mention length");
}

#[test]
fn test_encrypted_backup_roundtrip() {
    let master = MasterSecret::generate();
    let passphrase = "my-strong-passphrase-2024!";

    let backup = KeyExport::to_encrypted_backup(&master, passphrase).unwrap();
    assert!(backup.len() > 32, "backup should be larger than raw secret");

    let recovered = KeyExport::from_encrypted_backup(&backup, passphrase).unwrap();
    assert_eq!(master.as_bytes(), recovered.as_bytes());

    // Verify derived keys match
    let kp_orig = derive_pseudonym_key(master.as_bytes(), 0).unwrap();
    let kp_recovered = derive_pseudonym_key(recovered.as_bytes(), 0).unwrap();
    assert_eq!(kp_orig.public_bytes(), kp_recovered.public_bytes());
}

#[test]
fn test_wrong_password_backup_fails() {
    let master = MasterSecret::generate();
    let backup = KeyExport::to_encrypted_backup(&master, "correct-pass").unwrap();

    let result = KeyExport::from_encrypted_backup(&backup, "wrong-pass");
    assert!(result.is_err());
}

#[test]
fn test_encrypted_backup_empty_passphrase_rejected() {
    let master = MasterSecret::generate();
    let result = KeyExport::to_encrypted_backup(&master, "");
    assert!(result.is_err());
}

#[test]
fn test_encrypted_backup_corrupt_data() {
    let master = MasterSecret::generate();
    let mut backup = KeyExport::to_encrypted_backup(&master, "pass").unwrap();

    // Tamper with ciphertext
    if let Some(byte) = backup.last_mut() {
        *byte ^= 0xFF;
    }

    let result = KeyExport::from_encrypted_backup(&backup, "pass");
    assert!(result.is_err());
}

#[test]
fn test_encrypted_backup_too_short() {
    let result = KeyExport::from_encrypted_backup(&[0u8; 10], "pass");
    assert!(result.is_err());
}

#[test]
fn test_encrypted_backup_bad_version() {
    let master = MasterSecret::generate();
    let mut backup = KeyExport::to_encrypted_backup(&master, "pass").unwrap();
    // Corrupt the version header
    backup[0] = 0xFF;
    backup[1] = 0xFF;
    let result = KeyExport::from_encrypted_backup(&backup, "pass");
    assert!(result.is_err());
}
