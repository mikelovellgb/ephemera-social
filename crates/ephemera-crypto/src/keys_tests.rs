use super::*;

#[test]
fn keypair_generate_and_recover() {
    let kp = KeyPair::generate();
    let secret = kp.secret_bytes();
    let recovered = KeyPair::from_secret_bytes(&secret).unwrap();
    assert_eq!(kp.public_bytes(), recovered.public_bytes());
}

#[test]
fn secret_bytes_returns_zeroizing() {
    // Verify that secret_bytes() returns a Zeroizing wrapper.
    let kp = KeyPair::generate();
    let secret: Zeroizing<[u8; 32]> = kp.secret_bytes();
    let recovered = KeyPair::from_secret_bytes(&secret).unwrap();
    assert_eq!(kp.public_bytes(), recovered.public_bytes());
}

#[test]
fn pseudonym_derivation_deterministic() {
    let master = MasterSecret::generate();
    let kp1 = derive_pseudonym_key(master.as_bytes(), 0).unwrap();
    let kp2 = derive_pseudonym_key(master.as_bytes(), 0).unwrap();
    assert_eq!(kp1.public_bytes(), kp2.public_bytes());
}

#[test]
fn pseudonym_derivation_different_indices() {
    let master = MasterSecret::generate();
    let kp0 = derive_pseudonym_key(master.as_bytes(), 0).unwrap();
    let kp1 = derive_pseudonym_key(master.as_bytes(), 1).unwrap();
    assert_ne!(kp0.public_bytes(), kp1.public_bytes());
}

#[test]
fn epoch_key_derivation() {
    let master = MasterSecret::generate();
    let ek1 = derive_epoch_key(master.as_bytes(), 1).unwrap();
    let ek2 = derive_epoch_key(master.as_bytes(), 2).unwrap();
    assert_ne!(ek1, ek2);

    // Deterministic
    let ek1b = derive_epoch_key(master.as_bytes(), 1).unwrap();
    assert_eq!(ek1, ek1b);
}

#[test]
fn hkdf_derive_basic() {
    let ikm = [0x42u8; 32];
    let key = hkdf_derive(&ikm, None, b"test-info").unwrap();
    assert_eq!(key.len(), 32);
    assert!(key.iter().any(|&b| b != 0));
}

#[test]
fn master_secret_debug_redacted() {
    let ms = MasterSecret::generate();
    let dbg = format!("{ms:?}");
    assert!(dbg.contains("REDACTED"));
}

#[test]
fn public_key_bytes_serde() {
    let kp = KeyPair::generate();
    let pkb = PublicKeyBytes(kp.public_bytes());
    let json = serde_json::to_string(&pkb).unwrap();
    let recovered: PublicKeyBytes = serde_json::from_str(&json).unwrap();
    assert_eq!(pkb, recovered);
}
