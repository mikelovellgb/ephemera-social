# Ephemera: Identity & Cryptography Specification

**Parent document:** [ARCHITECTURE.md](./ARCHITECTURE.md) Section 3
**Version:** 1.0
**Date:** 2026-03-26
**Status:** Implementation-ready specification

---

## 1. Overview

Ephemera uses a two-identity architecture with strict separation between network-layer identities and user-layer identities. All user identities are pseudonymous Ed25519 key pairs derived from a master key. There is no registration, no email, no phone number. The cryptographic design provides forward secrecy, post-compromise security, and cryptographic shredding of all content after 30 days.

This document specifies:
- The identity model and key hierarchy
- All cryptographic primitives and their parameters
- Wire formats for signed messages
- Key derivation paths
- Backup and recovery flows
- The threat model

**Cross-references:**
- Messaging encryption (X3DH, Double Ratchet): [04_social_features.md](./04_social_features.md) Section 6
- Epoch key storage and GC: [03_storage_data.md](./03_storage_data.md) Sections 3-4
- Prekey bundle DHT publication: [02_network_protocol.md](./02_network_protocol.md) Section 4
- PoW for identity creation: [05_moderation_safety.md](./05_moderation_safety.md) Section 5

---

## 2. Identity Model

### 2.1 Two-Identity Architecture

Ephemera maintains two completely separate identity layers. The separation is a hard security invariant -- no code path may ever link or derive one from the other.

**Node Identity (network layer):**
- A per-installation Ed25519 key pair generated fresh on first launch.
- Used exclusively for: DHT participation, gossip membership, peer-to-peer QUIC authentication, relay handshakes.
- Publicly visible on the network. Analogous to an IP address.
- NOT derived from user key material. Generated independently using OS CSPRNG.
- Stored in the application data directory, encrypted under the keystore passphrase.
- If the user reinstalls or moves devices, a new node identity is generated. This is intentional -- node identities are disposable.

**Pseudonym Identity (user layer):**
- Ed25519 key pairs derived deterministically from a master key via HKDF.
- Used for: content signing, DM encryption, prekey publication, social graph operations, profile publication.
- Multiple unlinkable pseudonyms per user. Each derived from a different index.
- Published to the network ONLY through anonymous transport circuits (T2/T3). Never associated with the node identity.
- Human-readable address: Bech32m encoding with `eph1` prefix.

### 2.2 Key Hierarchy

```
Master Key (256-bit secret, derived from BIP-39 mnemonic)
  |
  +-- Master Signing Key (Ed25519)
  |     Derivation: HKDF-SHA256(master_key, salt="ephemera-master-signing", info="v1")
  |     Purpose: Signs device authorization certificates. Rarely used (cold storage).
  |
  +-- Device Key (Ed25519, one per device)
  |     Derivation: HKDF-SHA256(master_key, salt="ephemera-device", info="v1" || device_index_u32_be)
  |     Purpose: Authorizes operations on this device. Signs session keys.
  |     |
  |     +-- Session Key (X25519, rotated per application session)
  |     |     Derivation: HKDF-SHA256(device_key_seed, salt="ephemera-session", info=session_id)
  |     |     Purpose: Ephemeral DH for transport-level key agreement.
  |     |
  |     +-- Signing Subkey (Ed25519, long-lived per device)
  |           Derivation: HKDF-SHA256(device_key_seed, salt="ephemera-signing-sub", info="v1")
  |           Purpose: Signs content on behalf of the device. Certified by the device key.
  |
  +-- Pseudonym A (Ed25519, index=0)
  |     Derivation: HKDF-SHA256(master_key, salt="ephemera-pseudonym", info="v1" || 0u32_be)
  |     Purpose: User-facing identity. Signs posts, profiles, social graph events.
  |     |
  |     +-- Pseudonym X25519 Key (for key exchange)
  |           Derivation: HKDF-SHA256(pseudonym_seed, salt="ephemera-pseudonym-x25519", info="v1")
  |           Purpose: Identity key for X3DH prekey bundles.
  |
  +-- Pseudonym B (Ed25519, index=1)
  |     (Same structure as Pseudonym A)
  |
  +-- Pseudonym C (Ed25519, index=2) ...
```

### 2.3 Key Derivation Paths (Formal Specification)

All HKDF operations use HKDF-SHA256 with the extract-then-expand paradigm. The `salt` parameter provides domain separation. The `info` parameter provides context binding.

```
# Master key from mnemonic
master_key = BIP39_to_seed(mnemonic, passphrase="ephemera-v1")  // 64 bytes
master_secret = HKDF-Extract(salt="ephemera-master", ikm=master_key)  // 32 bytes

# Master signing key
master_signing_seed = HKDF-Expand(prk=master_secret, info="ephemera-master-signing\x00v1", len=32)
master_signing_keypair = Ed25519_from_seed(master_signing_seed)

# Device key (index is u32 big-endian)
device_seed = HKDF-Expand(prk=master_secret, info="ephemera-device\x00v1" || index_u32_be, len=32)
device_keypair = Ed25519_from_seed(device_seed)

# Session key (session_id is 16-byte random, generated at app start)
session_seed = HKDF-Expand(prk=device_seed, info="ephemera-session\x00" || session_id, len=32)
session_keypair = X25519_from_seed(session_seed)

# Signing subkey
signing_sub_seed = HKDF-Expand(prk=device_seed, info="ephemera-signing-sub\x00v1", len=32)
signing_sub_keypair = Ed25519_from_seed(signing_sub_seed)

# Pseudonym key (index is u32 big-endian)
pseudonym_seed = HKDF-Expand(prk=master_secret, info="ephemera-pseudonym\x00v1" || index_u32_be, len=32)
pseudonym_keypair = Ed25519_from_seed(pseudonym_seed)

# Pseudonym X25519 key (for X3DH identity key)
pseudonym_x25519_seed = HKDF-Expand(prk=pseudonym_seed, info="ephemera-pseudonym-x25519\x00v1", len=32)
pseudonym_x25519_keypair = X25519_from_seed(pseudonym_x25519_seed)

# Node identity (NOT derived from master key -- independent CSPRNG)
node_seed = OS_CSPRNG(32)
node_keypair = Ed25519_from_seed(node_seed)
```

### 2.4 Pseudonym Addressing

Public keys are encoded for human use with Bech32m (BIP-350):

```
Format: eph1<bech32m-encoded-32-byte-pubkey>
Example: eph1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx

Components:
  HRP (human-readable part): "eph1"
  Data: 32-byte Ed25519 public key
  Checksum: Bech32m (improved error detection vs Bech32)
```

The `eph1` prefix provides:
- Visual disambiguation from Bitcoin/Lightning addresses.
- Error detection (Bech32m detects up to 4 character substitutions).
- Case insensitivity (QR code friendly).

### 2.5 Pseudonym Unlinkability

Each pseudonym is a completely independent Ed25519 key pair. An observer who knows Pseudonym A's public key cannot determine that Pseudonym B belongs to the same user unless the user explicitly links them. Unlinkability depends on:

1. Pseudonyms are published through different anonymous transport circuits.
2. Posting patterns (timing, language, topics) are the user's responsibility.
3. The system does not provide automatic pseudonym rotation or traffic analysis resistance between pseudonyms.

---

## 3. Cryptographic Primitives

### 3.1 Primitive Table

| Primitive | Algorithm | Rust Crate | Parameters | Notes |
|-----------|-----------|------------|------------|-------|
| Identity signatures | Ed25519 | `ed25519-dalek` 2.x | RFC 8032, SHA-512 | Public key IS the identity. Batch verification supported. |
| Key exchange | X25519 | `x25519-dalek` 2.x | RFC 7748 | Diffie-Hellman for session keys and X3DH. |
| Symmetric encryption | XChaCha20-Poly1305 | `chacha20poly1305` 0.10 | 192-bit nonce, 256-bit key | Safe for random nonce generation (no nonce reuse risk). |
| Key derivation | HKDF-SHA256 | `hkdf` 0.12 | RFC 5869 | Pseudonym derivation, session keys, ratchet steps. |
| Password-based KDF | Argon2id | `argon2` 0.5 | m=256MiB, t=3, p=4 | Keystore encryption. Parameters chosen for desktop hardware. |
| Content hashing | BLAKE3 | `blake3` 1.x | 256-bit output | Content addressing, chunk verification, Merkle trees. |
| Secure erasure | zeroize | `zeroize` 1.x | - | Mandatory `impl Zeroize` on all secret types. Drop guard. |
| Secret wrapping | secrecy | `secrecy` 0.8 | - | `Secret<T>` wrapper prevents accidental logging/display. |
| Memory locking | mlock | OS-native | - | Best-effort. Linux: `mlock(2)`. Windows: `VirtualLock`. |

### 3.2 Algorithm Rationale

**XChaCha20-Poly1305 over AES-256-GCM:**
- No AES-NI dependency. ARM devices (future mobile) lack hardware AES acceleration.
- 192-bit nonce eliminates nonce reuse risk with random generation.
- Pure software implementation, no side-channel leakage from hardware instruction timing.

**BLAKE3 over SHA-256:**
- 3.5x faster on modern hardware.
- Built-in tree hashing mode for parallel chunk verification.
- Pure Rust implementation (no C dependencies).
- Keyed MAC mode available (used for HMAC-equivalent operations).

**Equihash over Hashcash for PoW:**
- Memory-hard (resists GPU/ASIC acceleration). Critical for Sybil resistance.
- Asymmetric verification: slow to compute, fast to verify.
- See [05_moderation_safety.md](./05_moderation_safety.md) for PoW parameters.

---

## 4. Encryption Schemes

### 4.1 Epoch Key Encryption (Public Posts)

Public posts are encrypted under rotating epoch keys. This enables cryptographic shredding -- when the epoch key is deleted, all content from that epoch becomes permanently unrecoverable.

**Epoch key lifecycle:**

```
Time: ---|--- Day 1 ---|--- Day 2 ---|--- Day 3 ---|--- ... ---|--- Day 31 ---|--- Day 32 ---|
Key:     EK_1 generated  EK_2 gen     EK_3 gen                  EK_1 deleted   EK_2 deleted
         EK_1 active     EK_2 active  EK_3 active               (shredded)     (shredded)
```

**Epoch key generation:**
```
epoch_number = floor(unix_timestamp / 86400)  // 24-hour epochs, UTC-aligned
epoch_seed = HKDF-Expand(prk=master_secret, info="ephemera-epoch\x00v1" || epoch_number_u64_be, len=32)
epoch_key = epoch_seed  // 256-bit symmetric key for XChaCha20-Poly1305
```

**Post encryption:**
```
nonce = OS_CSPRNG(24)  // 192-bit random nonce (safe with XChaCha20)
ciphertext = XChaCha20-Poly1305.Encrypt(key=EK_N, nonce=nonce, aad=post_id, plaintext=cbor_post)
stored = nonce || ciphertext  // 24 bytes nonce + variable ciphertext + 16 bytes tag
```

**Epoch key distribution:**
- Epoch keys are shared within the social graph. When you connect with someone, you exchange current and recent epoch keys.
- New connections receive the current epoch key and up to 29 previous epoch keys (covering the full 30-day window).
- Keys are transmitted encrypted under the pairwise session key established during the connection handshake.

**Epoch key deletion (cryptographic shredding):**
1. Epoch key `EK_N` is retained for exactly 30 days after epoch N ends.
2. At deletion time: `zeroize(EK_N)` then remove from keystore.
3. Enqueue background sweep: find all content with `epoch_number == N` and delete ciphertext from fjall.
4. This is defense-in-depth with TTL-based GC. Content should already be deleted by TTL, but epoch key deletion ensures it.

### 4.2 Direct Message Encryption (Double Ratchet)

See [04_social_features.md](./04_social_features.md) Section 6 for the complete messaging protocol. Key primitives used:

**X3DH (Extended Triple Diffie-Hellman) for initial key exchange:**
- Identity key: X25519 derived from pseudonym (long-term).
- Signed prekey: X25519, rotated every 7 days, signed by identity key.
- One-time prekeys: X25519, single-use, batch of 100, replenished when < 20 remain.
- Prekey bundles published to DHT via anonymous circuit.

**Double Ratchet for ongoing messages:**
- Symmetric ratchet: HKDF-SHA256 chain.
- DH ratchet: X25519 with new ephemeral keys per message turn.
- Message key: derived from chain key, used once, then deleted.
- Forward secrecy: each message key is deleted immediately after use.
- Post-compromise security: a new DH ratchet step heals after compromise.

### 4.3 Sealed Sender (DMs)

The wire envelope for DMs carries NO author identity:

```
MessageEnvelope {
    recipient:      IdentityKey,     // Recipient's pseudonym pubkey (for routing)
    sender_sealed:  Vec<u8>,         // EMPTY on the wire -- sender is INSIDE ciphertext
    ciphertext:     Vec<u8>,         // Double Ratchet encrypted: { sender_pubkey, message_body }
    timestamp:      u64,             // Unix seconds
    ttl_seconds:    u32,             // Max 30 days, default 14 days
    pow_stamp:      PowStamp,        // Relationship-dependent difficulty
}
```

Relay nodes see only the recipient's public key (for delivery routing) but never learn the sender's identity. The sender's pseudonym is revealed only to the recipient upon decryption.

### 4.4 Local Keystore Format

The keystore encrypts all key material at rest using a passphrase-derived key.

**Binary format:**

```
Offset  Size     Field
0       4        Version (u32 big-endian, currently 0x00000001)
4       12       Argon2id salt (random)
16      24       XChaCha20-Poly1305 nonce (random)
40      variable Ciphertext (encrypted keystore payload)
-16     16       Poly1305 authentication tag
```

**Argon2id parameters:**
```
memory:     256 MiB (262144 KiB)
iterations: 3
parallelism: 4
output:     32 bytes (256-bit key)
```

These parameters target ~1 second derivation time on a modern desktop CPU. They are intentionally aggressive to resist brute-force attacks on the passphrase.

**Keystore payload (plaintext, before encryption):**

```rust
pub struct KeystorePayload {
    pub version: u32,
    pub master_secret: Secret<[u8; 32]>,
    pub device_index: u32,
    pub pseudonyms: Vec<PseudonymEntry>,
    pub epoch_keys: BTreeMap<u64, Secret<[u8; 32]>>,  // epoch_number -> key
    pub ratchet_states: HashMap<IdentityKey, RatchetState>,
    pub node_identity_seed: Secret<[u8; 32]>,
    pub prekey_state: PrekeyState,
}

pub struct PseudonymEntry {
    pub index: u32,
    pub display_name: String,
    pub created_at: u64,
    pub is_active: bool,
}

pub struct PrekeyState {
    pub signed_prekey: Secret<[u8; 32]>,
    pub signed_prekey_id: u32,
    pub signed_prekey_timestamp: u64,
    pub one_time_prekeys: Vec<(u32, Secret<[u8; 32]>)>,  // (id, seed)
    pub next_one_time_id: u32,
}
```

The payload is serialized with bincode before encryption. On keystore open, the passphrase is derived via Argon2id, the ciphertext is decrypted, and the payload is deserialized into memory. All secret fields use `Secret<T>` wrappers and implement `Zeroize` for automatic secure erasure on drop.

---

## 5. Wire Formats

### 5.1 Signed Post Envelope

Every post transmitted on the wire is wrapped in a protobuf `Envelope`:

```protobuf
message Envelope {
    uint32 version = 1;        // Protocol version (currently 1)
    MessageType type = 2;      // POST, DM, SOCIAL_EVENT, DHT_OP, etc.
    bytes author_id = 3;       // 32-byte Ed25519 pseudonym pubkey (public posts)
                               // EMPTY for sealed-sender DMs
    bytes node_id = 4;         // 32-byte forwarding node's network identity
    bytes payload = 5;         // Type-specific content (CBOR-encoded, then encrypted)
    bytes signature = 6;       // 64-byte Ed25519 signature (public posts)
                               // ABSENT for DMs (MAC-based auth inside ciphertext)
    uint64 timestamp = 7;      // Unix seconds (u64)
    uint64 ttl_seconds = 8;    // Content lifetime, max 2,592,000 (30 days)
}

enum MessageType {
    POST = 0;
    DIRECT_MESSAGE = 1;
    SOCIAL_EVENT = 2;          // Follow, unfollow, reaction
    DHT_STORE = 3;
    DHT_FIND = 4;
    PROFILE_UPDATE = 5;
    BLOOM_FILTER_UPDATE = 6;
    MODERATION_VOTE = 7;
    CONTENT_REPORT = 8;
    TOMBSTONE = 9;
    PREKEY_BUNDLE = 10;
}
```

### 5.2 Signature Computation

The signature covers a canonical byte sequence to prevent malleability:

```
signed_bytes = SHA-512(
    "ephemera-sig-v1\x00" ||        // Domain separator (17 bytes)
    version_u32_be ||                // 4 bytes
    type_u32_be ||                   // 4 bytes
    author_id ||                     // 32 bytes
    timestamp_u64_be ||              // 8 bytes
    ttl_seconds_u64_be ||            // 8 bytes
    BLAKE3(payload)                  // 32 bytes (hash of payload, not payload itself)
)

signature = Ed25519_Sign(pseudonym_secret_key, signed_bytes)
```

**Verification procedure:**
1. Reconstruct `signed_bytes` from the envelope fields.
2. Verify `Ed25519_Verify(author_id, signed_bytes, signature)`.
3. Batch verification: accumulate up to 64 signatures, then verify in batch using `ed25519-dalek`'s batch verification (significant speedup).

### 5.3 Content ID Computation

Every piece of content is addressed by its BLAKE3 hash with a version prefix:

```rust
pub struct ContentHash([u8; 33]); // 1 byte version + 32 bytes BLAKE3

impl ContentHash {
    pub fn compute(payload: &[u8]) -> Self {
        let hash = blake3::hash(payload);
        let mut bytes = [0u8; 33];
        bytes[0] = 0x01; // version 1
        bytes[1..].copy_from_slice(hash.as_bytes());
        Self(bytes)
    }

    pub fn version(&self) -> u8 { self.0[0] }
    pub fn hash_bytes(&self) -> &[u8; 32] { self.0[1..].try_into().unwrap() }
}
```

The version byte allows future migration to a different hash function without breaking content addressing. Version 0x01 = BLAKE3. Version 0x00 is reserved for "null hash" (empty content).

---

## 6. Backup and Recovery

### 6.1 BIP-39 Mnemonic

The master key is derived from a BIP-39 mnemonic phrase:

```
Mnemonic: 12 or 24 English words from the BIP-39 wordlist (2048 words)
Entropy:  128 bits (12 words) or 256 bits (24 words)
Default:  24 words (256 bits) for maximum security
Seed:     PBKDF2-HMAC-SHA512(mnemonic, salt="ephemera-v1", iterations=2048, len=64)
Master:   HKDF-Extract(salt="ephemera-master", ikm=seed)
```

**UX flow:**
1. User completes onboarding (Welcome -> Age confirmation -> Name -> Avatar -> Feed).
2. User creates their first post or sends their first connection request.
3. System prompts: "Back up your recovery phrase. You'll need it to recover your identity on a new device."
4. Display 24 words, one screen at a time (6 words per screen, 4 screens).
5. Verification: ask user to tap words 3, 8, and 17 in order.
6. On success: store `recovery_backed_up = true` in local config.
7. If user dismisses: remind again after 24 hours, then weekly.

### 6.2 Shamir's Secret Sharing (Social Recovery)

For users who want distributed backup without writing down words:

```
Scheme:   Shamir's Secret Sharing (GF(256))
Shares:   5 total
Threshold: 3 required for recovery
Secret:   32-byte master_secret (not the mnemonic -- the derived key)
```

**Distribution flow:**
1. User selects 5 trusted connections.
2. For each connection: generate a share, encrypt it under the pairwise session key, send via DM.
3. Recipients' clients store the share in their local keystore (tagged with the sender's pseudonym).
4. The share is a 33-byte blob (1 byte share index + 32 bytes share data).

**Recovery flow:**
1. User installs Ephemera on a new device.
2. Selects "Recover via friends" instead of mnemonic entry.
3. Creates a temporary identity and contacts 3+ of the 5 trustees.
4. Each trustee's client sends the stored share via DM to the temporary identity.
5. Client reconstructs the master secret from any 3 shares.
6. Derives all pseudonyms and keys from the recovered master secret.
7. Temporary identity is discarded.

**Security note:** Trustees cannot individually recover the user's identity. Collusion of 3+ trustees could reconstruct it. Users should choose trustees they trust not to collude.

### 6.3 Device Authorization

When a user adds a new device:

1. New device generates a fresh node identity and device key (new index).
2. User authenticates on the new device using mnemonic or social recovery.
3. The master signing key issues a device authorization certificate:
   ```
   DeviceCert {
       device_pubkey: Ed25519PublicKey,
       device_index: u32,
       authorized_at: u64,
       expires_at: u64,         // Optional, for time-limited device access
       signature: Signature,    // Signed by master signing key
   }
   ```
4. The device cert is stored in the local keystore.
5. Multi-device sync is deferred to post-PoC. For the PoC, only one device is active at a time.

---

## 7. Threat Model

### 7.1 What Ephemera Protects Against

| Threat | Mitigation |
|--------|------------|
| Network observer determining who communicates with whom | Onion routing (T2/T3). Client traffic exits through Tor circuit or relay. |
| Storage nodes reading content | All stored data is ciphertext. Epoch keys held only by social graph participants. |
| Compromised long-term keys revealing past messages | Forward secrecy via Double Ratchet (DMs) + epoch key deletion (public posts). |
| Single compromised node deanonymizing users | No node holds identity-to-IP mappings. Node identity != pseudonym identity. |
| Content recoverable after 30 days | TTL enforcement (4 layers) + cryptographic shredding (epoch key deletion). |
| Brute-force attack on keystore passphrase | Argon2id with 256 MiB memory, 3 iterations. ~1 second per attempt on desktop. |
| Replay attacks on signed messages | HLC timestamps + TTL validation. Expired messages rejected. |
| Signature forgery | Ed25519 (128-bit security level). |
| Key substitution attacks | Content ID binds to author pubkey. Signature covers all envelope fields. |

### 7.2 What Ephemera Does NOT Protect Against

| Threat | Reason |
|--------|--------|
| Compromised client device | If the device is owned, the attacker has access to the keystore passphrase (via keylogger) and all decrypted key material. |
| Modified client bypassing CSAM checks | Client-side enforcement relies on honest clients. Relay verification only works for public content. |
| Global passive adversary (GPA) | Tor's known limitation. An adversary monitoring all network links can perform traffic correlation. |
| Screenshots or photos of the screen | Physical capture of displayed content is outside the cryptographic threat model. |
| Small anonymity set at launch | Mitigated by using Tor's existing anonymity set via Arti. Ephemera-specific traffic is a small subset of Tor traffic. |
| Quantum computing (future) | Ed25519 and X25519 are not quantum-resistant. Post-quantum hybrid (X25519 + ML-KEM-768) is planned but not in PoC. |
| Timing analysis of posting patterns | Pseudonyms with distinctive posting patterns may be linkable. This is a user behavior issue. |
| Compromised recovery trustees (3+ of 5) | Shamir's 3-of-5 threshold means 3 colluding trustees can reconstruct the master key. |

### 7.3 Cryptographic Assumptions

The security of Ephemera rests on:

1. **Discrete logarithm problem on Curve25519** -- Ed25519 signatures and X25519 key exchange.
2. **Security of BLAKE3** -- content addressing and integrity.
3. **IND-CCA2 security of XChaCha20-Poly1305** -- symmetric encryption.
4. **Memory-hardness of Argon2id** -- keystore protection.
5. **Random oracle model for HKDF-SHA256** -- key derivation.
6. **OS CSPRNG quality** -- all random values sourced from `getrandom` crate (Windows: `BCryptGenRandom`, Linux: `getrandom(2)`, macOS: `getentropy(2)`).

---

## 8. Implementation Notes

### 8.1 Secure Memory Handling

All types containing secret key material MUST:

```rust
use zeroize::Zeroize;
use secrecy::{Secret, ExposeSecret};

#[derive(Zeroize)]
#[zeroize(drop)]
pub struct MasterSecret([u8; 32]);

// Wrap in Secret<T> to prevent accidental Display/Debug
pub type ProtectedMasterSecret = Secret<MasterSecret>;
```

- `Zeroize` on drop: when the value goes out of scope, memory is overwritten with zeros.
- `Secret<T>`: prevents `Debug`, `Display`, `Clone` unless explicitly exposed.
- `mlock`: on Linux/macOS, attempt to lock key material pages in RAM to prevent swapping to disk. On Windows, use `VirtualLock` (best-effort; requires elevated privileges for large allocations).

### 8.2 Ed25519 Batch Verification

For incoming gossip messages, accumulate signatures and verify in batch:

```rust
use ed25519_dalek::verify_batch;

// Accumulate up to 64 messages
let messages: Vec<&[u8]> = ...;
let signatures: Vec<Signature> = ...;
let public_keys: Vec<PublicKey> = ...;

// Batch verify (approximately 2x faster than individual verification)
verify_batch(&messages, &signatures, &public_keys)?;
```

Batch verification should be used in the gossip message handler and the post ingestion pipeline. Individual verification is used for single-message paths (user creating a post, verifying a DM).

### 8.3 Nonce Generation

All nonces for XChaCha20-Poly1305 are 192-bit random values generated from the OS CSPRNG:

```rust
use rand::rngs::OsRng;
use rand::RngCore;

let mut nonce = [0u8; 24];
OsRng.fill_bytes(&mut nonce);
```

The 192-bit nonce space of XChaCha20 makes random nonce generation safe. The birthday bound for collision is 2^96 messages, which is astronomically beyond any realistic usage.

### 8.4 Post-Quantum Preparation

The architecture is designed for a future hybrid key exchange:

```
shared_secret = HKDF(X25519_shared_secret || ML-KEM-768_shared_secret)
```

This is NOT implemented in the PoC. The trait-based design of the crypto module allows swapping in hybrid key exchange without architectural changes. The `KeyExchange` trait abstracts the DH operation:

```rust
pub trait KeyExchange {
    type PublicKey;
    type SecretKey;
    type SharedSecret;

    fn generate_keypair() -> (Self::PublicKey, Self::SecretKey);
    fn diffie_hellman(our_secret: &Self::SecretKey, their_public: &Self::PublicKey) -> Self::SharedSecret;
}
```

---

## 9. Constants and Configuration

All cryptographic constants are defined in `ephemera-types` and `ephemera-crypto`:

```rust
// ephemera-types/src/constants.rs
pub const MAX_TTL: Duration = Duration::from_secs(30 * 24 * 60 * 60);        // 30 days
pub const EPOCH_DURATION: Duration = Duration::from_secs(24 * 60 * 60);      // 24 hours
pub const CLOCK_SKEW_TOLERANCE: Duration = Duration::from_secs(5 * 60);      // 5 minutes
pub const EPOCH_KEY_RETENTION: Duration = MAX_TTL;                            // 30 days
pub const TOMBSTONE_RETENTION_MULTIPLIER: u32 = 3;

// ephemera-crypto/src/constants.rs
pub const ARGON2_MEMORY_KIB: u32 = 262_144;    // 256 MiB
pub const ARGON2_ITERATIONS: u32 = 3;
pub const ARGON2_PARALLELISM: u32 = 4;
pub const ARGON2_OUTPUT_LEN: usize = 32;

pub const KEYSTORE_VERSION: u32 = 1;
pub const KEYSTORE_SALT_LEN: usize = 12;
pub const XCHACHA20_NONCE_LEN: usize = 24;
pub const XCHACHA20_TAG_LEN: usize = 16;

pub const ED25519_PUBKEY_LEN: usize = 32;
pub const ED25519_SIGNATURE_LEN: usize = 64;
pub const X25519_PUBKEY_LEN: usize = 32;
pub const BLAKE3_HASH_LEN: usize = 32;
pub const CONTENT_HASH_LEN: usize = 33;       // 1 version + 32 hash

pub const SIGNED_PREKEY_ROTATION: Duration = Duration::from_secs(7 * 24 * 60 * 60);  // 7 days
pub const ONE_TIME_PREKEY_BATCH: usize = 100;
pub const ONE_TIME_PREKEY_REFILL_THRESHOLD: usize = 20;

pub const BATCH_VERIFY_MAX: usize = 64;

pub const DOMAIN_SEPARATOR_SIG: &[u8] = b"ephemera-sig-v1\x00";
pub const DOMAIN_SEPARATOR_MASTER: &[u8] = b"ephemera-master";
pub const DOMAIN_SEPARATOR_DEVICE: &[u8] = b"ephemera-device\x00v1";
pub const DOMAIN_SEPARATOR_PSEUDONYM: &[u8] = b"ephemera-pseudonym\x00v1";
pub const DOMAIN_SEPARATOR_EPOCH: &[u8] = b"ephemera-epoch\x00v1";
pub const DOMAIN_SEPARATOR_SESSION: &[u8] = b"ephemera-session\x00";

pub const BIP39_PASSPHRASE: &str = "ephemera-v1";
pub const BECH32_HRP: &str = "eph1";
```

---

*This document is part of the Ephemera Architecture series. See [ARCHITECTURE.md](./ARCHITECTURE.md) for the master document.*
