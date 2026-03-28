<p align="center">
  <h1 align="center">Ephemera</h1>
  <p align="center"><strong>Decentralized social media where everything expires.</strong></p>
</p>

<p align="center">
  <a href="https://github.com/ephemera-social/ephemera/actions"><img src="https://img.shields.io/github/actions/workflow/status/ephemera-social/ephemera/ci.yml?branch=main&style=flat-square&logo=github" alt="CI"></a>
  <a href="LICENSE-MIT"><img src="https://img.shields.io/badge/license-MIT%2FApache--2.0-blue?style=flat-square" alt="License"></a>
  <a href="https://www.rust-lang.org"><img src="https://img.shields.io/badge/rust-1.80%2B-orange?style=flat-square&logo=rust" alt="Rust 1.80+"></a>
  <a href="https://github.com/ephemera-social/ephemera"><img src="https://img.shields.io/badge/status-alpha-yellow?style=flat-square" alt="Status: Alpha"></a>
</p>

<p align="center">
  Ephemera is a peer-to-peer social platform where all content &mdash; posts, messages,
  media, profiles &mdash; automatically and irrecoverably expires after 30 days.
  No central servers. No accounts. No permanent record.
</p>

<!--
  TODO: Replace with an actual screenshot of the desktop client.
  Recommended: 1280x720 PNG showing the feed view with a few posts,
  the sidebar navigation, and the compose panel open.
  Place the file at docs/assets/screenshot-feed.png

  ![Ephemera Desktop](docs/assets/screenshot-feed.png)
-->

---

## Features

- **Ephemeral by design** -- Every piece of content carries a TTL (max 30 days) enforced at the protocol level. Expired content is garbage-collected *and* cryptographically shredded via epoch key deletion. Nothing persists.

- **Zero infrastructure** -- Fully peer-to-peer. Nodes discover each other over a Kademlia DHT and propagate content through gossip, all built on [Iroh](https://iroh.computer/) QUIC transport. No servers to run, no domains to register, no cloud bills to pay.

- **End-to-end encrypted messaging** -- Private conversations use X3DH key agreement with Double Ratchet for forward secrecy and post-compromise security. Group chats use sender-key ratchets. Sealed-sender delivery hides metadata from relay nodes.

- **Anonymous identity** -- Users are Ed25519 key pairs. No email, no phone number, no registration. Pseudonyms are derived from a master key with epoch-based rotation to limit long-term linkability.

- **Rich media** -- Photos and short-form video with automatic EXIF stripping, transcoding, content-addressed chunked storage, and end-to-end encryption of media blobs before they touch the network.

- **Community moderation** -- Threshold-voting moderation quorums, configurable content filters, reputation scoring, and client-side CSAM hash matching -- all without breaking encryption or deanonymizing users.

- **Anti-abuse** -- Proof-of-work stamps (Equihash) on every post, adaptive rate limiting, and distributed reputation counters make spam and coordinated abuse expensive.

- **Cross-platform** -- Desktop via Tauri 2.x (Windows, macOS, Linux) with a SolidJS frontend. Android via Tauri mobile. Every client embeds a full node.

---

## Quick Start

### Prerequisites

| Requirement | Notes |
|-------------|-------|
| [Rust](https://rustup.rs/) 1.80+ (stable) | Install via `rustup` |
| C compiler | Required by `rusqlite` bundled SQLite build |
| Android SDK + NDK r26+ | Only for mobile builds |
| JDK 17+ | Only for mobile builds |

### Build

```bash
# Clone the repository
git clone https://github.com/ephemera-social/ephemera.git
cd ephemera

# Build all crates
cargo build --workspace

# Run the full test suite
cargo test --workspace
```

### Run a Node

```bash
# Create an identity
cargo run --bin ephemera -- init --passphrase "your-secret-passphrase"

# Create a post
cargo run --bin ephemera -- post "Hello, Ephemera!" --ttl 86400

# View your feed
cargo run --bin ephemera -- feed --limit 20
```

### Run Two Nodes Locally

```bash
# Terminal 1 -- Node A
EPHEMERA_DATA_DIR=./data-a cargo run --bin ephemera -- status

# Terminal 2 -- Node B (discovers A via localhost)
EPHEMERA_DATA_DIR=./data-b cargo run --bin ephemera -- status
```

Or use the bundled demo script: `./scripts/run_demo.sh`

### Run the Desktop Client

```bash
cargo install tauri-cli@^2
cargo tauri dev --manifest-path crates/ephemera-client/src-tauri/Cargo.toml
```

### Build Android APK

```bash
./scripts/build_android.sh --release
```

See [scripts/build_android.sh](scripts/build_android.sh) for environment setup details.

---

## Architecture Overview

```
+---------------------------------------------------------------+
|  User Device (Tauri 2.x)                                      |
|                                                                |
|  +--------------------+   +----------------------------------+ |
|  |  SolidJS Frontend  |   |  Rust Backend (ephemera-node)    | |
|  |                    |   |                                  | |
|  |  Feed, Compose,    |<->|  Identity    | Storage (SQLite)  | |
|  |  Messages, Groups  |   |  Crypto      | Media pipeline    | |
|  |  Settings          |   |  Gossip+DHT  | Moderation        | |
|  +--------------------+   +------+-------+-------------------+ |
+----------------------------------+-----------------------------+
                                   |
               +-------------------+-------------------+
               |                   |                   |
               v                   v                   v
       +--------------+   +--------------+   +-----------------+
       | T3: Iroh     |   | T2: Arti     |   | T1: Sphinx      |
       | Single-hop   |   | 3-hop Tor    |   | Mixnet          |
       | (50-200ms)   |   | (200-800ms)  |   | (2-30s)         |
       +--------------+   +--------------+   +-----------------+
               |                   |                   |
               +-------------------+-------------------+
                                   |
                                   v
       +---------------------------------------------------+
       |              Ephemera P2P Network                  |
       |                                                    |
       |  Kademlia DHT   |   Gossip (PlumTree)   |  CRDTs  |
       +---------------------------------------------------+
```

### Crate Map

The workspace is organized into 20 focused crates, each owning a single domain. No source file exceeds 300 lines.

| Crate | Purpose |
|-------|---------|
| **ephemera-types** | Shared primitives: `ContentHash`, `IdentityKey`, `Timestamp`, `Ttl`, protocol constants |
| **ephemera-crypto** | Ed25519 signing, X25519 key exchange, XChaCha20-Poly1305 AEAD, BLAKE3, Argon2id keystore |
| **ephemera-config** | Layered configuration: defaults, TOML files, environment variable overrides |
| **ephemera-events** | Internal async event bus for decoupled inter-subsystem communication |
| **ephemera-store** | Content-addressed blob store + SQLite metadata engine with TTL garbage collection |
| **ephemera-crdt** | Conflict-free replicated data types (OR-Set, G-Counter, ExpiringSet) for distributed state |
| **ephemera-post** | Post data model, validation, rich-text Markdown, and canonical serialization |
| **ephemera-social** | Social graph (follow/block), handle registry, feed assembly |
| **ephemera-media** | Media pipeline: EXIF stripping, image resize, video transcode, chunked encryption |
| **ephemera-protocol** | Wire protocol, envelope framing, CBOR/bincode codecs, version negotiation |
| **ephemera-transport** | QUIC transport with tiered privacy (direct, Tor, mixnet) |
| **ephemera-gossip** | Topic-based gossip pub/sub for content propagation |
| **ephemera-dht** | TTL-aware Kademlia DHT for peer discovery and prekey lookups |
| **ephemera-message** | Private messaging: X3DH + Double Ratchet (1-to-1), sender-key ratchets (group) |
| **ephemera-abuse** | Anti-abuse: proof-of-work stamps, rate limiting, reputation scoring, spam detection |
| **ephemera-mod** | Community moderation: threshold votes, content filters, moderator election |
| **ephemera-node** | Composition root: wires all subsystems into a running node |
| **ephemera-client** | Tauri 2.x desktop + mobile client with embedded node |
| **ephemera-cli** | Command-line interface for node operations and development |
| **ephemera-test-utils** | Shared test infrastructure: fixtures, mock nodes, network simulators |

Deep-dive specifications live in [`docs/architecture/`](docs/architecture/ARCHITECTURE.md).

---

## How It Works

### Identity

Your identity is an Ed25519 master key stored in an Argon2id-encrypted keystore on your device. From this master key, Ephemera derives pseudonymous sub-keys using HKDF, one per epoch (7 days). Pseudonyms rotate automatically. A 12-word BIP-39 mnemonic lets you recover your identity on a new device. No server ever sees your keys.

### Posting

Write text (Markdown supported), attach photos or short video, set a TTL (1 hour to 30 days), and hit send. Your client signs the post with your current pseudonym key, attaches a proof-of-work stamp, encrypts any media, and publishes to the gossip network. Peers who follow you or share a topic will receive it within seconds.

### Connecting

To follow someone, exchange handles via invite links, QR codes, or an out-of-band channel. Handles are human-readable names (`@alice`) registered in the DHT and signed by the owner's key. There is no global directory to scrape.

### Messaging

Private messages use X3DH for initial key agreement and the Double Ratchet for ongoing conversation encryption. Sealed-sender delivery means relay nodes cannot see who is messaging whom. Group chats use sender-key ratchets with member management via CRDTs.

### Content Expiry

Every piece of content has a TTL. When a TTL elapses, two things happen: (1) the garbage collector deletes the ciphertext from local storage, and (2) the epoch key that encrypted it is deleted, making any surviving copies on other nodes permanently unreadable. This is cryptographic shredding -- data destruction is a mathematical guarantee, not a policy promise.

---

## Security Model

### What Ephemera protects against

- **Mass surveillance** -- No central server to subpoena. Content is end-to-end encrypted. Transport is onion-routed (Tor or mixnet tiers). No metadata collection.
- **Data permanence** -- Protocol-enforced 30-day TTL with cryptographic shredding. No screenshot protection (that is not achievable), but the canonical copies are guaranteed to become unrecoverable.
- **Identity linkability** -- Pseudonym rotation every epoch. Node identity and user identity are cryptographically separated. No account registration.
- **Spam and abuse** -- Proof-of-work, rate limiting, reputation scoring, and community moderation quorums.

### What Ephemera does not protect against

- **A motivated adversary with physical access to your device** -- If someone has your device unlocked, they can read your data. Use full-disk encryption.
- **Screenshots and out-of-band copying** -- Once content is decrypted for display, a recipient can capture it. Ephemeral does not mean invisible.
- **Global passive adversaries** -- A nation-state adversary performing traffic analysis across all network links may correlate timing. The mixnet tier (T1) raises the cost significantly but cannot offer absolute guarantees.
- **Compromised devices** -- If malware runs on your device, no amount of cryptography will help.

We believe honesty about limitations is more valuable than false promises. For the full threat model and mitigation details, see the [Trust & Safety specification](docs/architecture/08_trust_safety.md).

---

## Contributing

Contributions are welcome and appreciated. Here is how to get involved:

1. **Fork and branch** -- Create a feature branch from `main`.
2. **Small, focused PRs** -- One concern per pull request.
3. **Tests required** -- All new code must include unit tests. `cargo test --workspace` must pass.
4. **Lint clean** -- `cargo clippy --workspace -- -D warnings` and `cargo fmt --all -- --check` must pass.
5. **300-line limit** -- No single `.rs` file may exceed ~300 lines. If it does, split it into focused modules.
6. **No unsafe** -- The workspace denies `unsafe_code`. If you believe an exception is needed, open an issue first.

### Code of Conduct

We are committed to providing a welcoming and inclusive experience for everyone. All participants are expected to uphold respectful, constructive, and harassment-free interaction. Abuse of the contribution process or community spaces will not be tolerated.

---

## Roadmap

### Done

- [x] Ed25519 identity model with Argon2id-encrypted keystore
- [x] X3DH + Double Ratchet encrypted messaging
- [x] Post creation, signing, validation, and canonical serialization
- [x] Social graph (follow, block, handles)
- [x] Media pipeline (image resize, video transcode, chunked encryption)
- [x] Gossip-based content propagation
- [x] TTL enforcement and garbage collection
- [x] Proof-of-work anti-spam
- [x] Rate limiting and reputation scoring
- [x] Community moderation (reports, content filters, threshold voting)
- [x] Tauri desktop client shell
- [x] CLI for node operations
- [x] Comprehensive test suite with adversarial and fuzz testing

### In Progress

- [ ] Wire moderation pipeline into gossip ingest and post creation paths
- [ ] Client-side CSAM perceptual hash checking in media pipeline
- [ ] NCMEC automated reporting pipeline
- [ ] SolidJS frontend views (feed, compose, messaging, settings)

### Planned

- [ ] Mixnet transport tier (Sphinx/Loopix)
- [ ] Android builds via Tauri mobile
- [ ] Offline-first sync with CRDT merge on reconnect
- [ ] Group communities with roles and permissions
- [ ] Voice messages (Opus encoding, chunked delivery)
- [ ] Relay incentive mechanism
- [ ] Independent security audit

---

## License

Licensed under either of

- [Apache License, Version 2.0](LICENSE-APACHE)
- [MIT License](LICENSE-MIT)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the work by you, as defined in the Apache-2.0 license, shall be dual-licensed as above, without any additional terms or conditions.
