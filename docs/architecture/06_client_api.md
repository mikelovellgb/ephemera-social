# Ephemera: Client Architecture & API Specification

**Parent document:** [ARCHITECTURE.md](./ARCHITECTURE.md) Section 9
**Version:** 1.0
**Date:** 2026-03-26
**Status:** Implementation-ready specification

---

## 1. Overview

Ephemera ships as a single Tauri 2.x desktop binary (Windows, macOS, Linux) containing a SolidJS frontend rendered in a native webview and a Rust backend that embeds the full `ephemera-node` library. There is no separate daemon process for the PoC. The frontend communicates with the backend exclusively through a JSON-RPC 2.0 API surface carried over Tauri's `invoke()` bridge, and receives real-time updates through Tauri's event system.

This document specifies:
- The Tauri 2.x application architecture and process model
- The complete JSON-RPC 2.0 API (all methods, params, results, error codes)
- The real-time event system (backend-to-frontend push)
- The SolidJS frontend structure (components, routing, state management)
- The onboarding flow
- Offline support and sync strategy
- Performance targets
- UI/UX guidelines

**Cross-references:**
- Identity and crypto primitives: [01_identity_crypto.md](./01_identity_crypto.md)
- Network protocol and transport: [02_network_protocol.md](./02_network_protocol.md)
- Storage engine and data model: [03_storage_data.md](./03_storage_data.md)
- Social features and messaging: [04_social_features.md](./04_social_features.md)
- Moderation and safety: [05_moderation_safety.md](./05_moderation_safety.md)

---

## 2. Tauri 2.x Application Architecture

### 2.1 Process Model

```
+---------------------------------------------------------------+
|  Tauri 2.x Process                                            |
|                                                               |
|  +-------------------------+   +----------------------------+ |
|  |  Webview (OS-native)    |   |  Rust Main Thread          | |
|  |                         |   |                            | |
|  |  SolidJS application    |   |  Tauri command handlers    | |
|  |  (TypeScript + HTML)    |   |  JSON-RPC 2.0 dispatcher   | |
|  |                         |   |                            | |
|  |  invoke("rpc", {...})   +-->+  fn rpc_handler()          | |
|  |                         |   |    |                       | |
|  |  listen("ephemera://")  +<--+  EphemeraNode (embedded)   | |
|  |                         |   |    |                       | |
|  +-------------------------+   |  +-- IdentityManager       | |
|                                |  +-- PostService           | |
|                                |  +-- SocialGraph           | |
|                                |  +-- MessageService        | |
|                                |  +-- ModerationService     | |
|                                |  +-- StorageEngine         | |
|                                |  +-- TransportLayer        | |
|                                |  +-- GossipOverlay         | |
|                                |  +-- EventBus (broadcast)  | |
|                                +----------------------------+ |
+---------------------------------------------------------------+
```

The Tauri process hosts both the webview and the Rust backend in a single OS process. The webview runs the SolidJS application. The Rust side embeds `ephemera-node` as a library, which manages all networking, storage, cryptography, and protocol logic.

### 2.2 Invoke Bridge

All frontend-to-backend communication uses a single Tauri command:

```rust
// src-tauri/src/main.rs

#[tauri::command]
async fn rpc(
    state: tauri::State<'_, AppState>,
    request: serde_json::Value,
) -> Result<serde_json::Value, String> {
    state.node.handle_rpc(request).await.map_err(|e| e.to_string())
}
```

The frontend calls this via:

```typescript
import { invoke } from "@tauri-apps/api/core";

async function rpc<T>(method: string, params?: Record<string, unknown>): Promise<T> {
  const request = {
    jsonrpc: "2.0",
    method,
    params: params ?? {},
    id: nextRequestId(),
  };
  const response = await invoke("rpc", { request });
  if (response.error) {
    throw new RpcError(response.error.code, response.error.message, response.error.data);
  }
  return response.result as T;
}
```

### 2.3 Why a Single Command

A single `rpc` command, rather than one Tauri command per operation, provides:

1. **Daemon-mode portability.** The same JSON-RPC 2.0 messages serialize over Unix domain socket or WebSocket for post-PoC daemon mode. No API redesign required.
2. **Middleware uniformity.** Logging, rate limiting, and error handling live in one dispatcher rather than duplicated across dozens of command handlers.
3. **Type generation.** A single schema generates the TypeScript client types.

### 2.4 AppState

```rust
pub struct AppState {
    pub node: Arc<EphemeraNode>,
    pub app_handle: tauri::AppHandle,
}
```

`EphemeraNode` is the composition root from `ephemera-node`. It owns all subsystems and exposes `handle_rpc()` for request dispatch and an event subscription channel for push events.

### 2.5 Event Forwarding

The Rust backend emits events through `ephemera-events` (a `tokio::sync::broadcast` channel). A dedicated background task listens on this channel and forwards events to the webview via Tauri's event system:

```rust
// src-tauri/src/events.rs

async fn event_forwarder(
    app_handle: tauri::AppHandle,
    mut rx: broadcast::Receiver<EphemeraEvent>,
) {
    while let Ok(event) = rx.recv().await {
        let (event_name, payload) = serialize_event(&event);
        app_handle.emit(&event_name, payload).ok();
    }
}
```

All event names are prefixed with `ephemera://` to namespace them within Tauri's event system.

### 2.6 Tauri Configuration

Key `tauri.conf.json` settings:

```json
{
  "app": {
    "withGlobalTauri": false,
    "security": {
      "csp": "default-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' blob: data:",
      "dangerousDisableAssetCspModification": false
    }
  },
  "bundle": {
    "active": true,
    "targets": ["nsis", "dmg", "appimage"],
    "identifier": "org.ephemera.app"
  },
  "plugins": {}
}
```

No Tauri plugins are used for the PoC. All functionality is provided by the embedded `ephemera-node`.

### 2.7 Application Data Directories

| Platform | Path |
|----------|------|
| Windows | `%APPDATA%\org.ephemera.app\` |
| macOS | `~/Library/Application Support/org.ephemera.app/` |
| Linux | `~/.local/share/org.ephemera.app/` |

Subdirectory layout:

```
org.ephemera.app/
  keystore.bin              # Encrypted keystore (Argon2id + XChaCha20-Poly1305)
  node.db                   # SQLite metadata database
  content/                  # fjall content store (ciphertext)
    YYYY-MM-DD/             # Time-partitioned directories
  media/                    # Local media cache (encrypted chunks)
  config.toml               # User configuration overrides
  node_identity.bin         # Encrypted node identity key
  logs/                     # Structured log files (tracing)
```

---

## 3. JSON-RPC 2.0 API

### 3.1 Protocol Conventions

**Request format:**

```json
{
  "jsonrpc": "2.0",
  "method": "namespace.method_name",
  "params": { ... },
  "id": 1
}
```

**Success response:**

```json
{
  "jsonrpc": "2.0",
  "result": { ... },
  "id": 1
}
```

**Error response:**

```json
{
  "jsonrpc": "2.0",
  "error": {
    "code": -32000,
    "message": "Human-readable description",
    "data": { ... }
  },
  "id": 1
}
```

**Conventions:**
- All timestamps are `u64` Unix milliseconds UTC.
- All content hashes are hex-encoded BLAKE3 hashes with version prefix (66 hex chars: 2 for version byte + 64 for hash).
- All identity keys are Bech32m-encoded strings with `eph1` prefix.
- Cursor-based pagination: results include a `cursor` field (opaque string). Pass it as `after` in the next request.
- All string params are UTF-8. The backend validates grapheme cluster counts where relevant.
- TTL values are in seconds (u32).

### 3.2 Global Error Codes

| Code | Name | Description |
|------|------|-------------|
| -32700 | `PARSE_ERROR` | Invalid JSON |
| -32600 | `INVALID_REQUEST` | Missing required JSON-RPC fields |
| -32601 | `METHOD_NOT_FOUND` | Unknown method name |
| -32602 | `INVALID_PARAMS` | Params failed validation |
| -32603 | `INTERNAL_ERROR` | Unexpected backend failure |
| -32000 | `NOT_INITIALIZED` | Node has not completed onboarding |
| -32001 | `KEYSTORE_LOCKED` | Keystore passphrase required |
| -32002 | `NETWORK_UNAVAILABLE` | No peer connections established |
| -32003 | `RATE_LIMITED` | Action exceeds rate limit. `data.retry_after_ms` indicates wait time |
| -32004 | `CONTENT_EXPIRED` | Referenced content has expired |
| -32005 | `NOT_FOUND` | Referenced resource does not exist locally |
| -32006 | `PERMISSION_DENIED` | Action not allowed (e.g., warming period restriction) |
| -32007 | `CONTENT_REJECTED` | Content failed validation (CSAM match, size limit, etc.). Generic message, never reveals detection specifics |
| -32008 | `ALREADY_EXISTS` | Duplicate action (e.g., duplicate connection request) |
| -32009 | `POW_FAILED` | PoW computation failed or timed out |
| -32010 | `STORAGE_FULL` | Local storage quota exceeded (500 MB cap) |

---

### 3.3 `identity` Namespace

#### `identity.create`

Create a new pseudonym identity. Called during onboarding or when creating additional pseudonyms.

**Params:**

```json
{
  "display_name": "quiet-fox-42",
  "passphrase": "user-chosen-passphrase"
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `display_name` | string | yes | 1-30 chars. Alphanumeric, hyphens, underscores. Validated client-side and server-side. |
| `passphrase` | string | yes | Keystore encryption passphrase. Min 8 chars. Used with Argon2id. |

**Result:**

```json
{
  "pseudonym_id": "eph1qw5d...",
  "display_name": "quiet-fox-42",
  "avatar_seed": "a1b2c3d4e5f6...",
  "created_at": 1711411200000,
  "is_warming": true,
  "warming_expires_at": 1712016000000
}
```

| Field | Type | Description |
|-------|------|-------------|
| `pseudonym_id` | string | Bech32m-encoded Ed25519 public key with `eph1` prefix. |
| `display_name` | string | The chosen display name. |
| `avatar_seed` | string | Hex-encoded 16 bytes derived from public key. Used by the frontend to generate identicon art. |
| `created_at` | u64 | Unix millis. |
| `is_warming` | bool | `true` during the 7-day warming period. |
| `warming_expires_at` | u64 | Unix millis when warming period ends. |

**Errors:** `-32602` (invalid display name or weak passphrase), `-32009` (identity creation PoW timed out), `-32010` (storage full).

**Notes:** Identity creation includes ~30s of Equihash PoW computation. The frontend must show a progress indicator. The PoW runs on a background thread and the invoke() call blocks until complete.

---

#### `identity.recover`

Recover identity from a BIP-39 mnemonic phrase.

**Params:**

```json
{
  "mnemonic": "abandon abandon abandon ... about",
  "passphrase": "new-passphrase-for-this-device"
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `mnemonic` | string | yes | 12 or 24 BIP-39 words, space-separated. |
| `passphrase` | string | yes | New keystore passphrase for this device. |

**Result:**

```json
{
  "pseudonym_id": "eph1qw5d...",
  "display_name": "quiet-fox-42",
  "pseudonym_count": 1,
  "recovered_connections": 0,
  "note": "Profile data will sync from the network. This may take several minutes."
}
```

**Errors:** `-32602` (invalid mnemonic, wrong word count, checksum failure).

---

#### `identity.get_profile`

Retrieve a pseudonym's profile. Works for the local user's own profile and for remote pseudonyms (fetched from local cache or DHT).

**Params:**

```json
{
  "pseudonym_id": "eph1qw5d..."
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `pseudonym_id` | string | yes | Bech32m pseudonym address. |

**Result:**

```json
{
  "pseudonym_id": "eph1qw5d...",
  "display_name": "quiet-fox-42",
  "bio": "Exploring the ephemeral web.",
  "avatar_seed": "a1b2c3d4e5f6...",
  "avatar_cid": "01abcdef...",
  "created_at": 1711411200000,
  "is_connection": true,
  "connection_status": "connected",
  "is_blocked": false,
  "is_muted": false,
  "is_self": false,
  "is_warming": false
}
```

| Field | Type | Description |
|-------|------|-------------|
| `pseudonym_id` | string | Bech32m address. |
| `display_name` | string | Display name (from LWW-Register, latest wins). |
| `bio` | string or null | User bio, max 200 chars. Null if not set. |
| `avatar_seed` | string | Hex-encoded seed for identicon generation. Always present. |
| `avatar_cid` | string or null | Content hash of custom avatar image. Null if using identicon. |
| `created_at` | u64 | Profile creation timestamp. |
| `is_connection` | bool | Whether a mutual connection exists. |
| `connection_status` | string or null | One of: `"pending_outgoing"`, `"pending_incoming"`, `"connected"`, `null`. |
| `is_blocked` | bool | Whether this pseudonym is blocked locally. |
| `is_muted` | bool | Whether this pseudonym is muted locally. |
| `is_self` | bool | Whether this is the local user's own pseudonym. |
| `is_warming` | bool | Whether this identity is in its 7-day warming period. |

**Errors:** `-32005` (pseudonym not found in local cache or DHT), `-32002` (network unavailable and not in cache).

---

#### `identity.update_profile`

Update the local user's profile. Publishes a signed `ProfileUpdate` CRDT event.

**Params:**

```json
{
  "display_name": "quiet-fox-42",
  "bio": "Exploring the ephemeral web.",
  "avatar_path": "/tmp/avatar.jpg"
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `display_name` | string | no | New display name. 1-30 chars. |
| `bio` | string | no | New bio. Max 200 chars. Pass empty string to clear. |
| `avatar_path` | string or null | no | Path to new avatar image file. Max 2 MB input. Processed to 256x256 WebP. Pass `null` to revert to identicon. |

Only provided fields are updated. Omitted fields remain unchanged.

**Result:**

```json
{
  "pseudonym_id": "eph1qw5d...",
  "display_name": "quiet-fox-42",
  "bio": "Exploring the ephemeral web.",
  "avatar_cid": "01abcdef...",
  "updated_at": 1711411260000
}
```

**Errors:** `-32602` (validation failure), `-32007` (avatar rejected by CSAM filter), `-32000` (not initialized).

---

#### `identity.get_recovery_phrase`

Retrieve the BIP-39 mnemonic for backup. Requires passphrase confirmation.

**Params:**

```json
{
  "passphrase": "user-passphrase"
}
```

**Result:**

```json
{
  "mnemonic": "abandon abandon abandon ... about",
  "word_count": 12
}
```

**Errors:** `-32001` (wrong passphrase).

**Notes:** The frontend must display this on a dedicated screen with warnings about not screenshotting or sharing. The mnemonic is never stored in frontend state beyond the display session.

---

#### `identity.list_pseudonyms`

List all pseudonyms owned by the local user.

**Params:** `{}`

**Result:**

```json
{
  "pseudonyms": [
    {
      "pseudonym_id": "eph1qw5d...",
      "display_name": "quiet-fox-42",
      "index": 0,
      "is_active": true,
      "created_at": 1711411200000,
      "is_warming": false
    }
  ]
}
```

**Errors:** `-32000` (not initialized).

---

#### `identity.switch_pseudonym`

Switch the active pseudonym. Affects which identity signs posts and messages.

**Params:**

```json
{
  "pseudonym_id": "eph1qw5d..."
}
```

**Result:**

```json
{
  "active_pseudonym_id": "eph1qw5d...",
  "display_name": "quiet-fox-42"
}
```

**Errors:** `-32005` (pseudonym not found), `-32000` (not initialized).

---

### 3.4 `posts` Namespace

#### `posts.create`

Create and publish a new post.

**Params:**

```json
{
  "body": "Hello #ephemera @quiet-fox-42",
  "media": [
    { "path": "/tmp/photo1.jpg" },
    { "path": "/tmp/photo2.jpg" }
  ],
  "ttl_seconds": 86400,
  "sensitivity": null,
  "parent": null
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `body` | string | no | Post text. Max 2,000 grapheme clusters AND 16 KB. Supports constrained Markdown. At least one of `body` or `media` must be present. |
| `media` | array | no | Array of `{ path: string }` objects. Max 4 items. Each file max 10 MB. |
| `ttl_seconds` | u32 | no | Lifetime in seconds. Range: 3600 (1 hour) to 2592000 (30 days). Default: 86400 (24 hours). |
| `sensitivity` | string or null | no | Sensitivity label. One of: `"nudity"`, `"violence"`, `"spoiler"`, or `null`. |
| `parent` | string or null | no | Content hash of parent post for replies. Null for top-level posts. |

**Result:**

```json
{
  "content_hash": "01abcdef...",
  "created_at": 1711411200000,
  "expires_at": 1711497600000,
  "sequence_number": 42,
  "status": "published"
}
```

| Field | Type | Description |
|-------|------|-------------|
| `content_hash` | string | BLAKE3 content ID of the post. |
| `created_at` | u64 | HLC timestamp as Unix millis. |
| `expires_at` | u64 | When the post will expire. |
| `sequence_number` | u64 | Per-author monotonic counter. |
| `status` | string | `"published"` (online) or `"queued"` (offline, will publish on reconnect). |

**Errors:** `-32602` (validation: text too long, too many photos, invalid TTL, file not found), `-32007` (CSAM rejection -- generic error, no details), `-32003` (queued offline), `-32006` (warming period: text-only limit, rate limit), `-32009` (PoW failed).

**Notes:** This call returns immediately after local storage (optimistic local-first). Network propagation happens asynchronously. If offline, the post is queued and the status is `"queued"`.

---

#### `posts.get`

Retrieve a single post by content hash.

**Params:**

```json
{
  "content_hash": "01abcdef..."
}
```

**Result:**

```json
{
  "content_hash": "01abcdef...",
  "author": "eph1qw5d...",
  "author_display_name": "quiet-fox-42",
  "body": "Hello #ephemera @quiet-fox-42",
  "body_html": "<p>Hello <a href=\"ephemera://tag/ephemera\">#ephemera</a> <a href=\"ephemera://user/eph1qw5d...\">@quiet-fox-42</a></p>",
  "media": [
    {
      "media_type": "image",
      "mime_type": "image/webp",
      "blurhash": "LEHV6nWB2yk8pyo0adR*.7kCMdnj",
      "alt_text": null,
      "width": 1280,
      "height": 960,
      "variants": [
        {
          "quality": "display",
          "cid": "01fedcba...",
          "size_bytes": 245760,
          "url": "ephemera://media/01fedcba..."
        }
      ]
    }
  ],
  "tags": ["ephemera"],
  "mentions": [
    {
      "pseudonym_id": "eph1qw5d...",
      "display_hint": "quiet-fox-42"
    }
  ],
  "parent": null,
  "root": null,
  "depth": 0,
  "created_at": 1711411200000,
  "expires_at": 1711497600000,
  "ttl_seconds": 86400,
  "remaining_seconds": 72000,
  "reactions": {
    "heart": 3,
    "laugh": 1,
    "fire": 0,
    "sad": 0,
    "thinking": 0
  },
  "my_reaction": "heart",
  "reply_count": 5,
  "is_own": true,
  "sensitivity": null
}
```

| Field | Type | Description |
|-------|------|-------------|
| `body_html` | string | Pre-rendered HTML with tags and mentions linked. For display in the webview. |
| `media[].variants[].url` | string | `ephemera://media/<cid>` protocol URL. The frontend uses this to request media bytes from the backend via a custom protocol handler. |
| `remaining_seconds` | u64 | Seconds until expiration at the time of the response. Used for countdown display. |
| `reactions` | object | Counts per emoji. Visible only to the post author (returns zeros for non-authors). |
| `my_reaction` | string or null | The current user's reaction, or null. |
| `reply_count` | u32 | Number of known replies in local storage. |
| `is_own` | bool | Whether the current active pseudonym authored this post. |

**Errors:** `-32005` (not found), `-32004` (expired).

---

#### `posts.list_feed`

Retrieve the chronological feed. Returns posts from connections, followed users, and the local user, in reverse chronological order.

**Params:**

```json
{
  "after": null,
  "limit": 20,
  "filter": "all"
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `after` | string or null | no | Opaque pagination cursor from a previous response. Null for the first page. |
| `limit` | u32 | no | Max items to return. Range: 1-50. Default: 20. |
| `filter` | string | no | One of: `"all"` (default), `"own"` (only the user's posts), `"connections"` (only from mutual connections), `"tag:<name>"` (posts with a specific tag). |

**Result:**

```json
{
  "posts": [ /* array of post objects (same shape as posts.get result) */ ],
  "cursor": "eyJ0cyI6MTcxMTQxMTIwMDAwMCwic2VxIjo0Mn0=",
  "has_more": true,
  "is_caught_up": false
}
```

| Field | Type | Description |
|-------|------|-------------|
| `posts` | array | Array of post objects. Same structure as `posts.get` result. |
| `cursor` | string or null | Opaque cursor for the next page. Null if no more results. |
| `has_more` | bool | Whether more posts exist beyond this page. |
| `is_caught_up` | bool | `true` when the user has seen all available posts (triggers "all caught up" UI). |

**Errors:** `-32602` (invalid filter or limit).

---

#### `posts.list_replies`

Retrieve replies to a specific post, in chronological order.

**Params:**

```json
{
  "content_hash": "01abcdef...",
  "after": null,
  "limit": 20
}
```

**Result:**

```json
{
  "replies": [ /* array of post objects */ ],
  "cursor": "...",
  "has_more": true,
  "parent_exists": true
}
```

| Field | Type | Description |
|-------|------|-------------|
| `parent_exists` | bool | `false` if the parent post has expired. The frontend should display "[original post expired]". |

**Errors:** `-32005` (parent content hash not found and not expired -- truly unknown).

---

#### `posts.delete`

Delete a post authored by the current pseudonym. Propagates a deletion tombstone.

**Params:**

```json
{
  "content_hash": "01abcdef..."
}
```

**Result:**

```json
{
  "deleted": true,
  "content_hash": "01abcdef...",
  "tombstone_ttl_seconds": 259200
}
```

| Field | Type | Description |
|-------|------|-------------|
| `tombstone_ttl_seconds` | u32 | How long the tombstone will propagate (3x original TTL). |

**Errors:** `-32005` (not found), `-32006` (not the author of this post).

---

#### `posts.react`

Add or remove a reaction on a post.

**Params:**

```json
{
  "content_hash": "01abcdef...",
  "emoji": "heart",
  "action": "add"
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `content_hash` | string | yes | Target post. |
| `emoji` | string | yes | One of: `"heart"`, `"laugh"`, `"fire"`, `"sad"`, `"thinking"`. |
| `action` | string | yes | `"add"` or `"remove"`. |

**Result:**

```json
{
  "content_hash": "01abcdef...",
  "emoji": "heart",
  "action": "add",
  "applied": true
}
```

**Errors:** `-32005` (post not found), `-32004` (post expired), `-32602` (invalid emoji).

---

### 3.5 `social` Namespace

#### `social.request_connection`

Send a connection request to another pseudonym.

**Params:**

```json
{
  "target": "eph1abc...",
  "message": "Hey, met you at the meetup!"
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `target` | string | yes | Bech32m pseudonym address of the target. |
| `message` | string | no | Optional introduction message. Max 280 chars. |

**Result:**

```json
{
  "request_id": "01abcdef...",
  "target": "eph1abc...",
  "status": "pending_outgoing",
  "created_at": 1711411200000
}
```

**Errors:** `-32008` (already connected or request already pending), `-32005` (target pseudonym unknown), `-32009` (PoW for stranger request failed), `-32006` (warming period restriction).

---

#### `social.accept_connection`

Accept an incoming connection request.

**Params:**

```json
{
  "source": "eph1abc..."
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `source` | string | yes | The pseudonym who sent the request. |

**Result:**

```json
{
  "source": "eph1abc...",
  "status": "connected",
  "connected_at": 1711411260000
}
```

**Errors:** `-32005` (no pending request from this source).

---

#### `social.reject_connection`

Reject (silently discard) an incoming connection request.

**Params:**

```json
{
  "source": "eph1abc..."
}
```

**Result:**

```json
{
  "source": "eph1abc...",
  "rejected": true
}
```

**Errors:** `-32005` (no pending request from this source).

---

#### `social.remove_connection`

Remove an existing mutual connection.

**Params:**

```json
{
  "target": "eph1abc..."
}
```

**Result:**

```json
{
  "target": "eph1abc...",
  "removed": true,
  "disconnected_at": 1711411320000
}
```

**Errors:** `-32005` (no connection with this pseudonym).

---

#### `social.list_connections`

List all connections and pending requests.

**Params:**

```json
{
  "status": "all",
  "after": null,
  "limit": 50
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `status` | string | no | One of: `"all"` (default), `"connected"`, `"pending_incoming"`, `"pending_outgoing"`. |
| `after` | string or null | no | Pagination cursor. |
| `limit` | u32 | no | Max items. Default 50, max 100. |

**Result:**

```json
{
  "connections": [
    {
      "pseudonym_id": "eph1abc...",
      "display_name": "brave-otter-7",
      "avatar_seed": "...",
      "status": "connected",
      "since": 1711411260000,
      "message": null
    },
    {
      "pseudonym_id": "eph1def...",
      "display_name": "wild-crane-13",
      "avatar_seed": "...",
      "status": "pending_incoming",
      "since": 1711411300000,
      "message": "Hey, met you at the meetup!"
    }
  ],
  "cursor": "...",
  "has_more": false,
  "counts": {
    "connected": 12,
    "pending_incoming": 3,
    "pending_outgoing": 1
  }
}
```

**Errors:** `-32602` (invalid status filter).

---

#### `social.follow`

Follow a pseudonym for public content discovery (asymmetric, no consent needed).

**Params:**

```json
{
  "target": "eph1abc..."
}
```

**Result:**

```json
{
  "target": "eph1abc...",
  "following": true
}
```

**Errors:** `-32008` (already following).

---

#### `social.unfollow`

Stop following a pseudonym.

**Params:**

```json
{
  "target": "eph1abc..."
}
```

**Result:**

```json
{
  "target": "eph1abc...",
  "following": false
}
```

**Errors:** `-32005` (not following this pseudonym).

---

#### `social.get_invite_link`

Generate an invite link for the current pseudonym.

**Params:** `{}`

**Result:**

```json
{
  "link": "ephemera://connect/eph1qw5d...",
  "pseudonym_id": "eph1qw5d...",
  "display_name": "quiet-fox-42"
}
```

---

#### `social.resolve_invite_link`

Parse an invite link and look up the referenced pseudonym.

**Params:**

```json
{
  "link": "ephemera://connect/eph1abc..."
}
```

**Result:**

```json
{
  "pseudonym_id": "eph1abc...",
  "display_name": "brave-otter-7",
  "is_connection": false,
  "connection_status": null
}
```

**Errors:** `-32602` (malformed link), `-32005` (pseudonym not found on network).

---

### 3.6 `messages` Namespace

#### `messages.send`

Send an encrypted direct message.

**Params:**

```json
{
  "recipient": "eph1abc...",
  "body": "Hey, what's up?",
  "media": null,
  "ttl_seconds": 1209600
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `recipient` | string | yes | Bech32m pseudonym address. |
| `body` | string | yes | Message text. Max 10,000 grapheme clusters AND 32 KB. |
| `media` | object or null | no | `{ path: string }` for a single photo attachment. Max 1 photo per message. |
| `ttl_seconds` | u32 | no | Lifetime. Range: 3600-2592000. Default: 1209600 (14 days). |

**Result:**

```json
{
  "message_id": "01abcdef...",
  "recipient": "eph1abc...",
  "created_at": 1711411200000,
  "expires_at": 1712620800000,
  "status": "sent"
}
```

| Field | Type | Description |
|-------|------|-------------|
| `message_id` | string | BLAKE3 hash of the encrypted envelope. |
| `status` | string | `"sent"` (deposited at dead drop), `"queued"` (offline, will send on reconnect), `"pending_request"` (first message, sent as message request with PoW). |

**Errors:** `-32005` (recipient not found), `-32006` (blocked by recipient -- the sender does NOT learn this; the message is silently discarded and `"sent"` is returned), `-32003` (rate limited for stranger DMs), `-32009` (PoW failed for message request).

**Notes:** Messages to blocked users return `"sent"` to prevent information leakage about the block. The backend silently discards the message.

---

#### `messages.list_conversations`

List all message conversations (grouped by peer pseudonym).

**Params:**

```json
{
  "after": null,
  "limit": 20,
  "filter": "all"
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `after` | string or null | no | Pagination cursor. |
| `limit` | u32 | no | Default 20, max 50. |
| `filter` | string | no | One of: `"all"` (default), `"unread"`, `"requests"` (message requests from strangers). |

**Result:**

```json
{
  "conversations": [
    {
      "peer": "eph1abc...",
      "peer_display_name": "brave-otter-7",
      "peer_avatar_seed": "...",
      "last_message_preview": "Hey, what's up?",
      "last_message_at": 1711411200000,
      "unread_count": 2,
      "is_request": false,
      "is_connection": true
    }
  ],
  "cursor": "...",
  "has_more": false,
  "total_unread": 5
}
```

| Field | Type | Description |
|-------|------|-------------|
| `last_message_preview` | string | First 100 chars of the last message body. |
| `is_request` | bool | True if this is a message request from a stranger (no prior conversation). |

**Errors:** `-32000` (not initialized).

---

#### `messages.get_messages`

Retrieve messages in a conversation with a specific peer.

**Params:**

```json
{
  "peer": "eph1abc...",
  "after": null,
  "limit": 30
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `peer` | string | yes | Bech32m pseudonym address of the conversation partner. |
| `after` | string or null | no | Pagination cursor. |
| `limit` | u32 | no | Default 30, max 100. |

**Result:**

```json
{
  "messages": [
    {
      "message_id": "01abcdef...",
      "sender": "eph1qw5d...",
      "recipient": "eph1abc...",
      "body": "Hey, what's up?",
      "media": null,
      "created_at": 1711411200000,
      "expires_at": 1712620800000,
      "remaining_seconds": 1200000,
      "is_own": true,
      "is_read": true,
      "status": "delivered"
    }
  ],
  "cursor": "...",
  "has_more": true
}
```

| Field | Type | Description |
|-------|------|-------------|
| `status` | string | `"sent"` (deposited, not yet confirmed), `"delivered"` (recipient has retrieved from dead drop), `"read"` (recipient marked as read), `"queued"` (offline, not yet sent), `"failed"` (delivery failed after retries). |

**Errors:** `-32005` (no conversation with this peer).

---

#### `messages.mark_read`

Mark all messages in a conversation as read up to a given message.

**Params:**

```json
{
  "peer": "eph1abc...",
  "up_to_message_id": "01abcdef..."
}
```

**Result:**

```json
{
  "peer": "eph1abc...",
  "marked_count": 3,
  "unread_remaining": 0
}
```

**Errors:** `-32005` (conversation or message not found).

---

#### `messages.accept_request`

Accept a message request from a stranger, starting a conversation.

**Params:**

```json
{
  "peer": "eph1abc..."
}
```

**Result:**

```json
{
  "peer": "eph1abc...",
  "accepted": true,
  "conversation_started_at": 1711411200000
}
```

**Errors:** `-32005` (no message request from this peer).

---

#### `messages.reject_request`

Reject a message request. Silently discards it.

**Params:**

```json
{
  "peer": "eph1abc..."
}
```

**Result:**

```json
{
  "peer": "eph1abc...",
  "rejected": true
}
```

**Errors:** `-32005` (no message request from this peer).

---

### 3.7 `moderation` Namespace

#### `moderation.report`

Report a post for abuse. Stored locally for MVP. Post-MVP triggers distributed moderation flow.

**Params:**

```json
{
  "content_hash": "01abcdef...",
  "reason": "harassment",
  "description": "Targeted harassment in replies."
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `content_hash` | string | yes | Content hash of the reported post. |
| `reason` | string | yes | One of: `"spam"`, `"harassment"`, `"hate_speech"`, `"violence"`, `"csam"`, `"other"`. |
| `description` | string | no | Optional free-text description. Max 500 chars. |

**Result:**

```json
{
  "report_id": "01abcdef...",
  "content_hash": "01abcdef...",
  "reason": "harassment",
  "created_at": 1711411200000,
  "status": "submitted"
}
```

**Errors:** `-32005` (content not found), `-32008` (already reported this content).

---

#### `moderation.block`

Block a pseudonym. All their content is hidden. They cannot send connection requests or DMs.

**Params:**

```json
{
  "target": "eph1abc...",
  "reason": "Spamming my replies"
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `target` | string | yes | Bech32m pseudonym address to block. |
| `reason` | string | no | Local note, never transmitted to the network. Max 200 chars. |

**Result:**

```json
{
  "target": "eph1abc...",
  "blocked": true,
  "blocked_at": 1711411200000
}
```

**Errors:** `-32008` (already blocked).

---

#### `moderation.unblock`

Remove a block on a pseudonym.

**Params:**

```json
{
  "target": "eph1abc..."
}
```

**Result:**

```json
{
  "target": "eph1abc...",
  "unblocked": true
}
```

**Errors:** `-32005` (not blocked).

---

#### `moderation.mute`

Mute a pseudonym. Their content is hidden from the feed. Can be time-limited.

**Params:**

```json
{
  "target": "eph1abc...",
  "duration_seconds": 86400
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `target` | string | yes | Bech32m pseudonym address. |
| `duration_seconds` | u32 or null | no | Duration of the mute in seconds. Null for permanent. |

**Result:**

```json
{
  "target": "eph1abc...",
  "muted": true,
  "muted_at": 1711411200000,
  "expires_at": 1711497600000
}
```

| Field | Type | Description |
|-------|------|-------------|
| `expires_at` | u64 or null | When the mute expires. Null if permanent. |

**Errors:** `-32008` (already muted).

---

#### `moderation.unmute`

Remove a mute on a pseudonym.

**Params:**

```json
{
  "target": "eph1abc..."
}
```

**Result:**

```json
{
  "target": "eph1abc...",
  "unmuted": true
}
```

**Errors:** `-32005` (not muted).

---

#### `moderation.list_blocked`

List all blocked pseudonyms.

**Params:**

```json
{
  "after": null,
  "limit": 50
}
```

**Result:**

```json
{
  "blocked": [
    {
      "pseudonym_id": "eph1abc...",
      "display_name": "spam-bot-99",
      "blocked_at": 1711411200000,
      "reason": "Spamming my replies"
    }
  ],
  "cursor": "...",
  "has_more": false
}
```

---

#### `moderation.list_muted`

List all muted pseudonyms.

**Params:**

```json
{
  "after": null,
  "limit": 50
}
```

**Result:**

```json
{
  "muted": [
    {
      "pseudonym_id": "eph1def...",
      "display_name": "loud-parrot-5",
      "muted_at": 1711411200000,
      "expires_at": 1711497600000
    }
  ],
  "cursor": "...",
  "has_more": false
}
```

---

### 3.8 `node` Namespace

#### `node.status`

Get the current node status, connectivity, and health information.

**Params:** `{}`

**Result:**

```json
{
  "state": "online",
  "uptime_seconds": 3600,
  "peer_count": 23,
  "relay_connected": true,
  "transport_tier": "T2",
  "sync_state": "synced",
  "sync_progress": 1.0,
  "last_sync_at": 1711411200000,
  "storage": {
    "used_bytes": 104857600,
    "quota_bytes": 524288000,
    "usage_percent": 20.0,
    "post_count": 1234,
    "media_chunk_count": 567,
    "oldest_content_at": 1709251200000
  },
  "identity": {
    "active_pseudonym": "eph1qw5d...",
    "display_name": "quiet-fox-42",
    "pseudonym_count": 1,
    "is_warming": false
  },
  "network": {
    "gossip_topics": 5,
    "dht_routing_entries": 150,
    "bandwidth_in_bps": 51200,
    "bandwidth_out_bps": 25600
  }
}
```

| Field | Type | Description |
|-------|------|-------------|
| `state` | string | One of: `"offline"`, `"connecting"`, `"online"`, `"syncing"`. |
| `sync_state` | string | One of: `"idle"`, `"syncing"`, `"synced"`. |
| `sync_progress` | f64 | 0.0 to 1.0. Progress of anti-entropy sync. |
| `transport_tier` | string | Current active transport: `"T1"`, `"T2"`, `"T3"`, or `"none"`. |

**Errors:** none (always succeeds, returns current state).

---

#### `node.peers`

List connected peers (network-level, not user-level).

**Params:**

```json
{
  "limit": 50
}
```

**Result:**

```json
{
  "peers": [
    {
      "node_id": "...",
      "address": "relay",
      "connected_since": 1711411200000,
      "latency_ms": 150,
      "is_relay": false
    }
  ],
  "total_count": 23
}
```

**Notes:** Node IDs are displayed as truncated hashes. IP addresses are never exposed to the frontend.

---

#### `node.storage_stats`

Detailed storage breakdown.

**Params:** `{}`

**Result:**

```json
{
  "total_used_bytes": 104857600,
  "quota_bytes": 524288000,
  "breakdown": {
    "posts_bytes": 52428800,
    "media_bytes": 41943040,
    "messages_bytes": 5242880,
    "metadata_bytes": 3145728,
    "keystore_bytes": 2097152
  },
  "gc_last_run_at": 1711411140000,
  "gc_next_run_at": 1711411200000,
  "expired_pending_gc": 12,
  "day_directories": 30
}
```

---

#### `node.set_transport_tier`

Change the active transport privacy tier.

**Params:**

```json
{
  "tier": "T2"
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `tier` | string | yes | One of: `"T1"`, `"T2"`, `"T3"`. |

**Result:**

```json
{
  "tier": "T2",
  "note": "Switching transport tier. New connections will use T2. Existing connections will drain."
}
```

**Errors:** `-32602` (invalid tier), `-32603` (T1 not available in PoC).

---

### 3.9 `media` Namespace

#### `media.get`

Retrieve media bytes by content hash. Used by the custom `ephemera://media/` protocol handler in the webview.

**Params:**

```json
{
  "cid": "01fedcba..."
}
```

**Result:** Binary media bytes returned as base64-encoded string. For large media, the frontend uses the `ephemera://media/<cid>` custom protocol which streams bytes directly.

```json
{
  "cid": "01fedcba...",
  "mime_type": "image/webp",
  "size_bytes": 245760,
  "data_base64": "UklGRl..."
}
```

**Errors:** `-32005` (media not found locally), `-32002` (not available on network).

**Notes:** For normal display, the frontend uses the `ephemera://media/<cid>` custom protocol handler registered with Tauri. This method exists as a fallback and for programmatic access.

---

### 3.10 `feed` Namespace

#### `feed.discover`

Browse public posts from outside the user's social graph. No algorithmic ranking -- purely reverse chronological.

**Params:**

```json
{
  "after": null,
  "limit": 20,
  "tag": null
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `after` | string or null | no | Pagination cursor. |
| `limit` | u32 | no | Default 20, max 50. |
| `tag` | string or null | no | Filter by hashtag (local index search). |

**Result:**

```json
{
  "posts": [ /* array of post objects */ ],
  "cursor": "...",
  "has_more": true
}
```

**Notes:** Discover feed only shows public posts from pseudonyms not in the user's social graph. This is the entry point for finding new people to connect with. No ranking, no trending, no algorithmic amplification.

---

## 4. Real-Time Event System

### 4.1 Architecture

The Rust backend pushes events to the SolidJS frontend via Tauri's built-in event system. Events flow through the internal `ephemera-events` broadcast channel and are serialized to JSON for the webview.

**Frontend listener setup:**

```typescript
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

// Listen for all Ephemera events
const unlisteners: UnlistenFn[] = [];

async function setupEventListeners() {
  unlisteners.push(
    await listen<NewPostEvent>("ephemera://new_post", (event) => {
      feedStore.prependPost(event.payload);
    }),
    await listen<NewMessageEvent>("ephemera://new_message", (event) => {
      messageStore.addMessage(event.payload);
    }),
    // ... additional listeners
  );
}

// Cleanup on unmount
function teardownEventListeners() {
  unlisteners.forEach((unlisten) => unlisten());
}
```

### 4.2 Event Catalog

All events use the prefix `ephemera://`. Payloads are JSON objects.

---

#### `ephemera://new_post`

A new post has been received from the network and stored locally.

```json
{
  "content_hash": "01abcdef...",
  "author": "eph1abc...",
  "author_display_name": "brave-otter-7",
  "body_preview": "Just discovered something amazing...",
  "has_media": true,
  "media_blurhash": "LEHV6nWB2yk8pyo0adR*.7kCMdnj",
  "created_at": 1711411200000,
  "expires_at": 1711497600000,
  "is_reply": false,
  "parent": null,
  "is_from_connection": true
}
```

**Frontend action:** Prepend to feed if visible, show unread indicator if scrolled down. Animate in with fade effect.

---

#### `ephemera://post_expired`

A post has expired and been removed from local storage.

```json
{
  "content_hash": "01abcdef...",
  "author": "eph1abc...",
  "expired_at": 1711497600000
}
```

**Frontend action:** Remove from feed with dissolve animation. If the user was viewing the post detail, show "[This post has expired]" overlay.

---

#### `ephemera://post_deleted`

A post was deleted by its author (tombstone received).

```json
{
  "content_hash": "01abcdef...",
  "author": "eph1abc...",
  "deleted_at": 1711411320000
}
```

**Frontend action:** Remove from feed with fade-out animation.

---

#### `ephemera://new_reaction`

A reaction was received on one of the user's own posts.

```json
{
  "content_hash": "01abcdef...",
  "emoji": "heart",
  "reactor": "eph1abc...",
  "reactor_display_name": "brave-otter-7",
  "action": "add",
  "new_count": 4
}
```

**Frontend action:** Update reaction counts on the post. Brief toast notification if the app is active.

---

#### `ephemera://new_message`

A new direct message has been received and decrypted.

```json
{
  "message_id": "01abcdef...",
  "sender": "eph1abc...",
  "sender_display_name": "brave-otter-7",
  "body_preview": "Hey, what's up?",
  "has_media": false,
  "created_at": 1711411200000,
  "is_request": false,
  "conversation_unread_count": 3
}
```

**Frontend action:** Update conversation list, show notification badge, play notification sound if enabled. If the conversation is currently open, append the message with slide-in animation.

---

#### `ephemera://message_status`

Delivery status update for a sent message.

```json
{
  "message_id": "01abcdef...",
  "recipient": "eph1abc...",
  "status": "delivered"
}
```

| Value | Description |
|-------|-------------|
| `"sent"` | Deposited at dead drop. |
| `"delivered"` | Recipient retrieved from dead drop. |
| `"read"` | Recipient marked as read. |
| `"failed"` | Delivery failed after retries. |

**Frontend action:** Update message status indicator (single check, double check, etc.).

---

#### `ephemera://connection_request`

An incoming connection request has been received.

```json
{
  "source": "eph1abc...",
  "source_display_name": "brave-otter-7",
  "message": "Hey, met you at the meetup!",
  "created_at": 1711411200000
}
```

**Frontend action:** Show notification badge on the Connections tab. Add to pending incoming list.

---

#### `ephemera://connection_accepted`

An outgoing connection request was accepted.

```json
{
  "target": "eph1abc...",
  "target_display_name": "brave-otter-7",
  "connected_at": 1711411260000
}
```

**Frontend action:** Update connection status, show toast, move from pending to connected list.

---

#### `ephemera://connection_removed`

A connection was removed by the other party.

```json
{
  "peer": "eph1abc...",
  "peer_display_name": "brave-otter-7",
  "disconnected_at": 1711411320000
}
```

**Frontend action:** Update connection status silently (no notification to avoid social pressure).

---

#### `ephemera://network_state`

Network connectivity state changed.

```json
{
  "state": "online",
  "peer_count": 23,
  "transport_tier": "T2"
}
```

**Frontend action:** Update the three-dot connectivity indicator in the header.

---

#### `ephemera://sync_progress`

Anti-entropy sync progress update.

```json
{
  "state": "syncing",
  "progress": 0.65,
  "items_synced": 130,
  "items_total": 200
}
```

**Frontend action:** Update sync indicator. When `state` becomes `"synced"`, the initial loading state can be dismissed.

---

#### `ephemera://content_expiring`

Content is entering its final 24 hours before expiration. Used to trigger the decay visual effect.

```json
{
  "content_hash": "01abcdef...",
  "expires_at": 1711497600000,
  "remaining_seconds": 86400
}
```

**Frontend action:** Begin the fade/dissolve visual effect on this post. The effect intensifies as `remaining_seconds` decreases.

---

#### `ephemera://compose_queue_update`

A queued offline post has been published or failed.

```json
{
  "content_hash": "01abcdef...",
  "status": "published",
  "previous_status": "queued"
}
```

**Frontend action:** Update the post's status indicator. Remove the "pending" badge. If `"failed"`, show a retry option.

---

#### `ephemera://recovery_prompt`

Triggered after the user's first post or first connection request. Prompts the user to back up their recovery phrase.

```json
{
  "trigger": "first_post",
  "pseudonym_id": "eph1qw5d..."
}
```

**Frontend action:** Show a non-blocking modal encouraging the user to back up their recovery phrase. Dismissible, but shown again after 24 hours if not completed. Tracks state in local preferences.

---

## 5. SolidJS Frontend Structure

### 5.1 Technology Stack

| Layer | Choice | Notes |
|-------|--------|-------|
| Framework | SolidJS 1.9+ | Fine-grained reactivity, no virtual DOM, small bundle |
| Router | @solidjs/router | File-system-inspired routing |
| State | createStore / createSignal | No external state manager. SolidJS primitives are sufficient. |
| Styling | CSS Modules + CSS custom properties | Dark theme via custom properties. No CSS-in-JS runtime. |
| Icons | Phosphor Icons (SVG, tree-shakeable) | Consistent, accessible icon set |
| Build | Vite 6.x | Fast dev server, optimized production builds |
| Types | TypeScript 5.x (strict mode) | Full type safety on the frontend |

### 5.2 Directory Structure

```
src-ui/
  index.html
  src/
    main.tsx                          # Entry point, router setup, event listener init
    App.tsx                           # Root component, layout shell, theme provider

    api/
      rpc.ts                          # JSON-RPC 2.0 client wrapper (invoke bridge)
      events.ts                       # Tauri event listener setup and dispatch
      types.ts                        # Generated TypeScript types for all RPC params/results

    stores/
      identity.ts                     # Active pseudonym, profile data, warming state
      feed.ts                         # Feed posts, pagination cursor, caught-up state
      connections.ts                  # Connection list, pending requests, counts
      messages.ts                     # Conversations, unread counts, message cache
      node.ts                         # Network state, sync progress, storage stats
      compose.ts                      # Compose queue for offline posts
      ui.ts                           # Theme, notifications, modal state

    views/
      Onboarding/
        Welcome.tsx                   # Welcome screen with logo and tagline
        AgeTerms.tsx                  # Age confirmation + terms (single tap)
        NameSelection.tsx             # Adjective-animal name picker
        AvatarSelection.tsx           # Generative identicon grid
        Creating.tsx                  # PoW progress screen during identity creation
      Feed/
        FeedView.tsx                  # Main feed with virtual scroll
        PostCard.tsx                  # Individual post display
        PostDetail.tsx                # Full post view with replies
        ComposeModal.tsx              # Post composer (text + photo + TTL)
        MediaGallery.tsx              # Photo viewer overlay
        ExpiryIndicator.tsx           # Countdown / fade effect component
      Connections/
        ConnectionsView.tsx           # Tabs: Connected / Pending / Discover
        ConnectionCard.tsx            # Connection entry with accept/reject
        InviteSheet.tsx               # QR code and invite link display
        ProfileView.tsx               # Pseudonym profile page
      Messages/
        ConversationsView.tsx         # Conversation list
        ChatView.tsx                  # Individual conversation thread
        MessageBubble.tsx             # Single message display
        MessageRequestBanner.tsx      # Accept/reject message request
      Settings/
        SettingsView.tsx              # Settings index
        ProfileEditor.tsx             # Edit display name, bio, avatar
        RecoveryPhrase.tsx            # View/backup recovery phrase
        NodeStatus.tsx                # Network status, peer count, storage
        PrivacySettings.tsx           # Transport tier, etc.
        About.tsx                     # Version, licenses, links
      Discover/
        DiscoverView.tsx              # Public post browser (no ranking)
        TagBrowser.tsx                # Browse posts by hashtag (local index)

    components/
      Layout/
        AppShell.tsx                  # Main layout: sidebar + content area
        Sidebar.tsx                   # Navigation sidebar
        Header.tsx                    # Top bar: search, connectivity indicator, compose
        ConnectivityDots.tsx          # Three-dot network status indicator
      Common/
        Avatar.tsx                    # Identicon/avatar renderer (from seed or CID)
        Button.tsx                    # Styled button variants
        Input.tsx                     # Text input with character count
        Modal.tsx                     # Modal overlay
        Toast.tsx                     # Toast notification
        Spinner.tsx                   # Loading spinner
        Badge.tsx                     # Notification badge
        TTLPicker.tsx                 # TTL selection slider (1h to 30d)
        ReactionBar.tsx               # Five-emoji reaction selector
        VirtualList.tsx               # Virtual scrolling list for feeds
        MediaPreview.tsx              # Blurhash placeholder + progressive load
        MarkdownRenderer.tsx          # Constrained Markdown to HTML renderer
        TimeAgo.tsx                   # Relative time display ("3m ago")
        CountdownTimer.tsx            # Expiry countdown with visual decay
        EmptyState.tsx                # "All caught up" / "No messages" illustrations

    hooks/
      useRpc.ts                       # Reactive wrapper around rpc() calls
      useEvent.ts                     # Tauri event subscription with auto-cleanup
      useFeed.ts                      # Feed pagination and auto-refresh
      useConversation.ts              # Message pagination and auto-scroll
      useOnline.ts                    # Network connectivity reactive signal
      useCountdown.ts                 # Countdown timer for expiry display

    theme/
      variables.css                   # CSS custom properties (night/dawn)
      global.css                      # Reset, typography, base styles
      animations.css                  # Fade, dissolve, slide-in keyframes
```

### 5.3 Component Architecture

```
App.tsx
  |
  +-- <Router>
        |
        +-- Onboarding (if !initialized)
        |     +-- Welcome -> AgeTerms -> NameSelection -> AvatarSelection -> Creating
        |
        +-- AppShell (if initialized)
              |
              +-- Sidebar
              |     +-- Nav items: Feed, Discover, Connections, Messages, Settings
              |     +-- Active pseudonym display
              |     +-- ConnectivityDots
              |
              +-- <main> (content area, routed)
                    |
                    +-- /feed            -> FeedView
                    +-- /feed/:id        -> PostDetail
                    +-- /discover        -> DiscoverView
                    +-- /discover/tag/:t -> TagBrowser
                    +-- /connections     -> ConnectionsView
                    +-- /connections/:id -> ProfileView
                    +-- /messages        -> ConversationsView
                    +-- /messages/:peer  -> ChatView
                    +-- /settings        -> SettingsView
                    +-- /settings/*      -> (sub-routes)
```

### 5.4 State Management

SolidJS's built-in reactivity is used exclusively. No Redux, no MobX, no external state libraries.

**Pattern: stores as singletons exported from module scope.**

```typescript
// stores/feed.ts
import { createStore, produce } from "solid-js/store";
import { createSignal } from "solid-js";
import { rpc } from "../api/rpc";
import type { Post, FeedResponse } from "../api/types";

interface FeedState {
  posts: Post[];
  cursor: string | null;
  hasMore: boolean;
  isCaughtUp: boolean;
  isLoading: boolean;
}

const [feedState, setFeedState] = createStore<FeedState>({
  posts: [],
  cursor: null,
  hasMore: true,
  isCaughtUp: false,
  isLoading: false,
});

// Derived signals
const [unreadCount, setUnreadCount] = createSignal(0);

// Actions
async function loadFeed(filter = "all") {
  setFeedState("isLoading", true);
  try {
    const res = await rpc<FeedResponse>("posts.list_feed", {
      after: feedState.cursor,
      limit: 20,
      filter,
    });
    setFeedState(produce((s) => {
      s.posts.push(...res.posts);
      s.cursor = res.cursor;
      s.hasMore = res.has_more;
      s.isCaughtUp = res.is_caught_up;
    }));
  } finally {
    setFeedState("isLoading", false);
  }
}

function prependPost(post: Post) {
  setFeedState(produce((s) => {
    s.posts.unshift(post);
  }));
}

function removePost(contentHash: string) {
  setFeedState(produce((s) => {
    s.posts = s.posts.filter((p) => p.content_hash !== contentHash);
  }));
}

export { feedState, unreadCount, loadFeed, prependPost, removePost };
```

**Cross-store coordination:** Events dispatch to multiple stores. For example, `ephemera://connection_accepted` updates both the connections store and triggers a feed refresh.

### 5.5 Routing

Routes map directly to views. All routes are protected by an initialization guard that redirects to onboarding if the identity has not been created.

```typescript
// main.tsx
import { Router, Route, Navigate } from "@solidjs/router";

<Router>
  <Route path="/onboarding/*" component={OnboardingLayout} />
  <Route path="/" component={AppShell}>
    <Route path="/" component={() => <Navigate href="/feed" />} />
    <Route path="/feed" component={FeedView} />
    <Route path="/feed/:id" component={PostDetail} />
    <Route path="/discover" component={DiscoverView} />
    <Route path="/discover/tag/:tag" component={TagBrowser} />
    <Route path="/connections" component={ConnectionsView} />
    <Route path="/connections/:id" component={ProfileView} />
    <Route path="/messages" component={ConversationsView} />
    <Route path="/messages/:peer" component={ChatView} />
    <Route path="/settings" component={SettingsView} />
    <Route path="/settings/profile" component={ProfileEditor} />
    <Route path="/settings/recovery" component={RecoveryPhrase} />
    <Route path="/settings/node" component={NodeStatus} />
    <Route path="/settings/privacy" component={PrivacySettings} />
    <Route path="/settings/about" component={About} />
  </Route>
</Router>
```

### 5.6 Custom Protocol Handler

Media is served through a Tauri custom protocol handler registered as `ephemera://media/`. This avoids base64 encoding overhead for images.

```rust
// src-tauri/src/main.rs

fn main() {
    tauri::Builder::default()
        .register_asynchronous_uri_scheme_protocol("ephemera", |ctx, request, responder| {
            let state = ctx.app_handle().state::<AppState>();
            let path = request.uri().path();

            tokio::spawn(async move {
                if path.starts_with("/media/") {
                    let cid = &path[7..];
                    match state.node.get_media_bytes(cid).await {
                        Ok((mime, bytes)) => {
                            responder.respond(
                                http::Response::builder()
                                    .header("Content-Type", mime)
                                    .body(bytes)
                                    .unwrap()
                            );
                        }
                        Err(_) => {
                            responder.respond(
                                http::Response::builder()
                                    .status(404)
                                    .body(vec![])
                                    .unwrap()
                            );
                        }
                    }
                }
            });
        })
        .invoke_handler(tauri::generate_handler![rpc])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

The frontend references media as:

```html
<img src="ephemera://media/01fedcba..." alt="..." />
```

---

## 6. Onboarding Flow

### 6.1 Design Goals

- **Fast:** Under 20 seconds from launch to feed (excluding PoW).
- **Minimal friction:** No registration, no email, no phone, no verification.
- **No decisions required:** Good defaults everywhere. All choices are optional tweaks.
- **Deferred complexity:** Recovery phrase is prompted after the user's first post, not during onboarding.

### 6.2 Screen Sequence

```
Welcome -> AgeTerms -> NameSelection -> AvatarSelection -> Creating -> Feed
```

---

#### Screen 1: Welcome

**Duration:** 2-3 seconds (auto-advance or tap).

**Content:**
- Ephemera logo (night sky motif, scattered dots forming a constellation that slowly fades).
- Tagline: "Conversations that fade. Just like they should."
- Subtle background animation: particles drifting and dissolving.
- Single "Get Started" button.

**Technical:** No backend calls. Pure presentation.

---

#### Screen 2: Age Confirmation + Terms

**Duration:** Single tap.

**Content:**
- Text: "By continuing, you confirm you are 16 or older and agree to our community guidelines."
- "Community guidelines" is a link that opens an in-app sheet (not a browser).
- Single button: "I Agree and I'm 16+"
- No checkboxes, no multi-step consent.

**Technical:** No backend calls. Sets a local flag (`age_confirmed: true`) in app preferences.

---

#### Screen 3: Name Selection

**Duration:** ~5 seconds.

**Content:**
- Header: "Choose your name"
- Pre-filled with a generated adjective-animal pattern (e.g., "quiet-fox-42"). Three alternate suggestions shown as chips below.
- Editable text input. Real-time validation (1-30 chars, alphanumeric/hyphens/underscores).
- "Shuffle" button to regenerate suggestions.
- "Next" button.

**Technical:** Name generation runs in the frontend using a word list bundled with the app. No backend call until the final creation step.

**Name generation algorithm:**
```
adjectives = ["quiet", "brave", "wild", "calm", "swift", "warm", "cool", "bright", ...]  // ~200 words
animals = ["fox", "otter", "crane", "wolf", "hawk", "bear", "lynx", "dove", ...]        // ~100 words
number = random(1, 99)
name = `${randomChoice(adjectives)}-${randomChoice(animals)}-${number}`
```

---

#### Screen 4: Avatar Selection

**Duration:** ~3 seconds.

**Content:**
- Header: "Your avatar"
- A 3x3 grid of procedurally generated identicons, each derived from a different random seed.
- The first slot uses the actual pseudonym's public key seed (after creation).
- Tap to select. Selected avatar has a highlight ring.
- "Next" button.
- Small text: "You can always change this later."

**Technical:** Identicons are rendered client-side using a deterministic algorithm seeded by the `avatar_seed` (16 bytes from the public key hash). The generation is purely visual -- geometric shapes, color gradients -- no external dependencies.

**Note:** At this step the frontend pre-generates a temporary seed for display purposes. The actual `avatar_seed` derived from the pseudonym's public key is assigned after identity creation completes.

---

#### Screen 5: Creating Identity (PoW Progress)

**Duration:** ~30 seconds (Equihash proof-of-work).

**Content:**
- Header: "Creating your identity"
- Subtext: "This takes about 30 seconds. We're generating a cryptographic proof that makes spam expensive."
- Animated progress indicator (indeterminate, since PoW time is unpredictable).
- Constellation animation in the background (dots appearing and connecting).
- No cancel button (the operation is non-interruptible).

**Technical:** Calls `identity.create` RPC with the chosen name and a generated passphrase. For the PoC, the passphrase is device-generated and stored in OS keychain (no user-facing passphrase step during onboarding). The PoW computation runs on a background thread.

**On completion:** Auto-navigate to the feed. The feed will be empty or near-empty for a new user. Show a friendly empty state.

---

### 6.3 Post-Onboarding Prompts

**Recovery phrase prompt (deferred):**
Triggered by the `ephemera://recovery_prompt` event after the user's first post or first connection request. Shows a non-blocking modal:

- Header: "Back up your identity"
- Text: "Your identity exists only on this device. If you lose it, your connections and messages are gone forever. Write down your recovery phrase and store it somewhere safe."
- "Back Up Now" button -> navigates to Settings > Recovery Phrase.
- "Remind Me Later" button -> dismisses; re-prompts after 24 hours.
- After 3 dismissals, prompts weekly instead of daily.

---

## 7. Offline Support

### 7.1 Local-First Architecture

The local SQLite database is the source of truth for all user-facing data. The network is a sync mechanism, not a data source. The app is fully usable (with limitations) when offline.

### 7.2 What Works Offline

| Feature | Offline Behavior |
|---------|-----------------|
| Read feed | Full feed from local cache, rendered in < 1 second |
| Read messages | All previously synced conversations available |
| Compose posts | Queued locally, published on reconnect |
| Compose messages | Queued locally, sent on reconnect |
| React to posts | Applied locally, synced on reconnect |
| Edit profile | Applied locally, published on reconnect |
| View connections | Full list from local cache |
| Browse tags | Local index fully searchable |
| Delete own posts | Tombstone queued, propagated on reconnect |
| Block/mute | Applied immediately (local-only operations) |

### 7.3 What Requires Network

| Feature | Offline Behavior |
|---------|-----------------|
| Create identity | Requires PoW and prekey publication. Fully offline-blocked. |
| Receive new posts | No new content until reconnect |
| Receive new messages | No new messages until reconnect |
| Connection requests | Queued, sent on reconnect |
| Accept connections | Queued, sent on reconnect |
| Fetch remote profiles | Returns cached version or "unavailable" |
| Discover (public posts) | Only previously-cached public posts |

### 7.4 Compose Queue

Posts and messages created offline enter a local queue. Each queued item has a status lifecycle:

```
draft -> queued -> publishing -> published
                             \-> failed (with retry option)
```

**Queue storage:** SQLite table `compose_queue`:

```sql
CREATE TABLE compose_queue (
    id           TEXT PRIMARY KEY,     -- UUID
    item_type    TEXT NOT NULL,        -- 'post' or 'message'
    payload      BLOB NOT NULL,        -- CBOR-encoded creation params
    status       TEXT NOT NULL,        -- 'queued', 'publishing', 'published', 'failed'
    created_at   INTEGER NOT NULL,     -- Unix millis
    attempts     INTEGER DEFAULT 0,
    last_error   TEXT,
    content_hash TEXT                  -- Set after successful publication
);
```

**Retry policy:**
- Automatic retry on reconnect.
- Exponential backoff: 1s, 2s, 4s, 8s, 16s, 30s (cap).
- Maximum 10 attempts. After 10 failures, status becomes `"failed"` and requires manual retry.
- Queued items are processed in FIFO order to preserve chronological intent.

**Frontend display:**
- Queued posts appear in the feed immediately with a "pending" indicator (small clock icon).
- When published, the indicator disappears and the `content_hash` updates.
- Failed items show a "retry" button with a brief error description.

### 7.5 Sync Strategy

**Startup sync sequence:**

```
1. App launch
2. Render feed from local SQLite cache (< 1 second)
3. Initialize transport layer (background)
4. Connect to bootstrap nodes / cached peers (3-10 seconds)
5. Begin gossip subscription (subscribe to connection feeds)
6. Begin anti-entropy sync:
   a. Exchange Merkle tree roots with peers
   b. Identify divergences
   c. Fetch missing content
   d. Process in chronological order
7. Sync complete (30-120 seconds)
8. Emit ephemera://sync_progress { state: "synced" }
9. Process compose queue (publish any offline items)
```

**Ongoing sync:**
- Gossip delivers new content in real-time.
- Anti-entropy runs every 120 seconds as background reconciliation.
- Dead drop polling for messages: every 30 seconds when active, every 5 minutes when idle.

**Connectivity transitions:**
- **Online -> Offline:** The feed continues to display. New content stops arriving. Compose queue accepts new items. The connectivity indicator changes to "offline" (grey dots).
- **Offline -> Online:** Transport layer reconnects. Gossip subscriptions resume. Compose queue drains. Anti-entropy resync runs. Connectivity indicator transitions through "connecting" (amber dots) to "online" (green dots).

---

## 8. Performance Targets

### 8.1 Application Metrics

| Metric | Target | Measurement |
|--------|--------|-------------|
| Binary size (compressed) | < 25 MB | Tauri MSI/DMG/AppImage. Verified at each release. |
| Binary size (installed) | < 80 MB | Includes webview runtime on Windows. |
| Cold start to feed rendered | < 1 second | From process start to first meaningful paint (local data). |
| Warm start | < 500 ms | App already in memory, window restored. |
| Network ready | 3-10 seconds | At least one peer connection established. |
| Full sync | 30-120 seconds | Anti-entropy complete, all missed content backfilled. |
| Post creation (local) | < 30 ms | From "Post" button tap to optimistic update in feed. |
| Post creation (network P95) | < 2 seconds | To 95% of connected peers. |
| Feed scroll | 60 FPS | Virtual scrolling, offscreen recycling. Verified with browser devtools. |
| Memory (idle) | < 200 MB | App at rest, feed loaded, no active sync. |
| Memory (active) | < 350 MB | 150 MB app + 200 MB node (L1 cache, gossip buffers, transport). |
| Memory (peak) | < 500 MB | During media processing, anti-entropy, or PoW computation. |
| CPU (idle) | < 2% | No user interaction, no sync activity. |
| CPU (active sync) | < 15% | During anti-entropy or gossip burst. |
| Storage (base) | < 50 MB | Empty app, no user data, no content cache. |
| Storage (cap) | 500 MB | Default quota. Configurable up to 2 GB for full-node mode. |

### 8.2 Frontend Performance

| Metric | Target | Approach |
|--------|--------|----------|
| First Contentful Paint | < 200 ms | SolidJS SSG-like prerender, no hydration overhead. |
| Bundle size (JS) | < 100 KB gzipped | SolidJS tree-shaking, code splitting per route. |
| Feed render (50 posts) | < 16 ms | Virtual scrolling, only visible posts in DOM. |
| Image load (display) | < 500 ms | Blurhash instant, progressive load from local cache or network. |
| Compose modal open | < 50 ms | Lazy-loaded but prewarmed after first use. |
| Route transition | < 100 ms | No full-page reloads. SolidJS transitions. |

### 8.3 Virtual Scrolling

The feed uses virtual scrolling to handle large post lists efficiently:

- **Viewport window:** Only posts within the visible viewport (plus a 3-post overscan buffer above and below) are rendered in the DOM.
- **Height estimation:** Each post's height is measured once on first render and cached. Estimated height for unmeasured posts: 120px (text-only), 360px (with media).
- **Scroll anchoring:** When new posts are prepended at the top, the scroll position is preserved so the user's current view does not jump.
- **Recycling:** DOM nodes for posts that scroll out of the viewport are removed. SolidJS's fine-grained reactivity means only changed fields are updated when a recycled slot receives new data.

---

## 9. UI/UX Design Language

### 9.1 Theme

**Dark theme ("Night") is the default.** Light theme ("Dawn") is available as an accessibility option.

**Night theme palette:**

| Token | Value | Usage |
|-------|-------|-------|
| `--bg-primary` | `#0a0a0f` | App background |
| `--bg-secondary` | `#12121a` | Card/surface background |
| `--bg-tertiary` | `#1a1a26` | Input fields, hover states |
| `--text-primary` | `#e8e8ed` | Body text |
| `--text-secondary` | `#8888a0` | Secondary text, timestamps |
| `--text-muted` | `#555570` | Placeholder text, disabled |
| `--accent-primary` | `#6366f1` | Buttons, links, active states (indigo) |
| `--accent-secondary` | `#818cf8` | Hover states, secondary actions |
| `--accent-glow` | `rgba(99, 102, 241, 0.15)` | Glow effect behind focused elements |
| `--danger` | `#ef4444` | Destructive actions, errors |
| `--warning` | `#f59e0b` | Warnings, expiry indicators |
| `--success` | `#10b981` | Connected, published, success states |
| `--border` | `#1e1e2e` | Subtle borders |
| `--shadow` | `rgba(0, 0, 0, 0.5)` | Drop shadows |

**Dawn theme palette** (light): Inverted luminance with warm off-whites (`#faf9f6` background, `#1a1a2e` text). Same accent hue.

### 9.2 Visual Language of Impermanence

Every visual decision reinforces the ephemeral nature of the platform:

**Fade effects:**
- Posts within their final 24 hours gradually reduce opacity. At 24h remaining: 100% opacity. At 1h remaining: 60% opacity. At 0h: dissolve animation (2 seconds).
- CSS implementation:
  ```css
  .post-card {
    --decay-factor: calc(var(--remaining-seconds) / 86400);
    opacity: calc(0.6 + 0.4 * clamp(0, var(--decay-factor), 1));
    transition: opacity 30s ease;
  }
  ```
- The `--remaining-seconds` CSS variable is updated by the component every 60 seconds.

**Countdown indicators:**
- Every post displays a relative time badge: "23h left", "6h left", "45m left".
- The badge color transitions from `--text-secondary` (> 12h) to `--warning` (< 12h) to `--danger` (< 1h).
- Countdown updates every 60 seconds (> 1h remaining), every 10 seconds (< 1h remaining).

**Dissolve animation (post expiry):**
```css
@keyframes dissolve {
  0% { opacity: 1; filter: blur(0); transform: scale(1); }
  50% { opacity: 0.4; filter: blur(2px); transform: scale(0.98); }
  100% { opacity: 0; filter: blur(8px); transform: scale(0.95); height: 0; margin: 0; padding: 0; }
}

.post-card--expiring {
  animation: dissolve 2s ease-out forwards;
}
```

**Particle effects (subtle):**
- The app background includes very faint, slow-moving particles that drift and fade out, evoking stars or dust.
- Implemented with CSS animations on pseudo-elements. No canvas or WebGL -- keeps CPU usage minimal.
- Disabled when `prefers-reduced-motion` is set.

**Empty states:**
- "All caught up" illustration: a clear night sky with a few stars fading in and out.
- "No messages yet" illustration: two constellations on opposite sides of the screen, not yet connected.

### 9.3 Typography

| Element | Font | Size | Weight | Line Height |
|---------|------|------|--------|-------------|
| Body text | System font stack | 15px | 400 | 1.5 |
| Post body | System font stack | 15px | 400 | 1.6 |
| Display name | System font stack | 14px | 600 | 1.3 |
| Timestamp | System font stack | 12px | 400 | 1.3 |
| Heading (section) | System font stack | 18px | 600 | 1.3 |
| Code inline | `"SF Mono", "Fira Code", monospace` | 13px | 400 | 1.4 |
| TTL badge | System font stack | 11px | 600 | 1.0 |

**System font stack:**
```css
font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif;
```

No custom fonts are loaded. The system font stack ensures native feel across platforms and zero font-loading latency.

### 9.4 Accessibility Requirements

Ephemera targets WCAG 2.1 AA compliance.

| Requirement | Implementation |
|-------------|----------------|
| Color contrast | All text meets 4.5:1 ratio (AA). Accent on dark bg: 5.2:1. Verified with axe-core. |
| Keyboard navigation | All interactive elements reachable via Tab. Focus ring visible (2px `--accent-primary` outline). Escape closes modals. |
| Screen reader | All images have `alt` text (or `alt=""` for decorative). ARIA landmarks on all major sections. Live regions for new posts and messages. |
| Reduced motion | `@media (prefers-reduced-motion: reduce)` disables: particle background, dissolve animation (instant remove instead), slide-in transitions (instant appear). |
| Font scaling | All sizes in `rem`. Layout accommodates 200% font scaling without horizontal scrolling. |
| High contrast | Respects `@media (prefers-contrast: more)`. Increases border visibility, disables subtle glow effects. |
| Focus management | Modal open traps focus. Modal close returns focus to trigger element. Route changes move focus to main content heading. |
| Touch targets | All buttons and interactive elements have minimum 44x44px touch target area. |

### 9.5 Layout

**Desktop layout (>= 1024px):**
```
+------+-------------------------------------+
|      |  Header (connectivity, compose btn) |
| Side |-------------------------------------+
| bar  |                                     |
| (64  |  Main Content Area                  |
|  px) |  (max-width: 640px, centered)       |
|      |                                     |
|      |                                     |
+------+-------------------------------------+
```

- Sidebar: 64px wide, icon-only navigation. Expands to 240px on hover with labels.
- Main content: max 640px wide, centered in the remaining space. Optimal reading width for feed content.
- No right sidebar for PoC.

**Narrow layout (< 1024px):**
- Sidebar collapses to bottom tab bar (mobile-style, even on desktop for narrow windows).
- Full-width content area.

### 9.6 Notification System

**In-app toast notifications:**
- Position: top-right, stacked vertically.
- Auto-dismiss: 4 seconds for informational, 8 seconds for actionable.
- Max 3 visible toasts. Older toasts collapse.
- Types: info (blue), success (green), warning (amber), error (red).
- Actions: some toasts include a single action button (e.g., "View" on new message notification).

**OS-level notifications:**
- New DMs trigger an OS notification if the app window is not focused.
- Connection requests trigger an OS notification.
- New posts do NOT trigger OS notifications (too noisy).
- Respect OS do-not-disturb settings. Tauri's notification API is used.

### 9.7 Post Card Anatomy

```
+--------------------------------------------------+
| [Avatar]  Display Name          [23h left]       |
|           @eph1qw5...                            |
|                                                  |
| Post body text with #hashtags and @mentions      |
| rendered as styled links. Constrained Markdown   |
| formatting applied (bold, italic, etc).          |
|                                                  |
| +------+ +------+ +------+ +------+             |
| |      | |      | |      | |      |  <- photos  |
| | img1 | | img2 | | img3 | | img4 |    (grid)   |
| |      | |      | |      | |      |             |
| +------+ +------+ +------+ +------+             |
|                                                  |
| [heart] [laugh] [fire] [sad] [thinking]  [reply] |
|                                                  |
| 5 replies                                        |
+--------------------------------------------------+
```

- Avatar: 40x40px identicon or custom image.
- Display name: bold, truncated at 20 chars visible.
- Pseudonym address: truncated, copyable on click.
- TTL badge: top-right corner, color-coded by urgency.
- Photo grid: 1 photo = full width. 2 photos = side by side. 3 photos = 1 large + 2 small. 4 photos = 2x2 grid. All photos show blurhash placeholder during load.
- Reaction bar: visible on hover (desktop). Tapping a reaction toggles it.
- Reply count: clickable, navigates to PostDetail.

### 9.8 Sensitivity Overlay

Posts with a sensitivity label (`nudity`, `violence`, `spoiler`) are displayed with a blur overlay:

```
+--------------------------------------------------+
| [Avatar]  Display Name          [23h left]       |
|                                                  |
|  +--------------------------------------------+ |
|  |                                            | |
|  |     [!] This post may contain nudity       | |
|  |                                            | |
|  |          [ Show Content ]                  | |
|  |                                            | |
|  +--------------------------------------------+ |
+--------------------------------------------------+
```

- Background is a heavily blurred version of the actual content (10px Gaussian blur).
- Warning text describes the sensitivity label.
- "Show Content" reveals the actual content for the current session (not persisted).
- User can disable sensitivity overlays in Settings for specific categories.

---

## 10. Error Handling and Edge Cases

### 10.1 Frontend Error Handling Strategy

```typescript
// api/rpc.ts

class RpcError extends Error {
  code: number;
  data?: Record<string, unknown>;

  constructor(code: number, message: string, data?: Record<string, unknown>) {
    super(message);
    this.code = code;
    this.data = data;
  }
}

// Global error handler for RPC calls
function handleRpcError(error: RpcError): void {
  switch (error.code) {
    case -32000: // NOT_INITIALIZED
      navigateTo("/onboarding");
      break;
    case -32001: // KEYSTORE_LOCKED
      showModal("unlock-keystore");
      break;
    case -32003: // RATE_LIMITED
      showToast("warning", `Slow down. Try again in ${Math.ceil(error.data?.retry_after_ms / 1000)}s.`);
      break;
    case -32007: // CONTENT_REJECTED
      showToast("error", "Unable to share this content.");
      break;
    case -32010: // STORAGE_FULL
      showToast("error", "Storage full. Old content will be cleared automatically.");
      break;
    default:
      showToast("error", error.message);
      console.error("[RPC Error]", error.code, error.message, error.data);
  }
}
```

### 10.2 Edge Cases

| Scenario | Behavior |
|----------|----------|
| Expired post in reply chain | Parent shows "[original post expired]" placeholder. Reply remains visible with its own TTL. |
| Author deletes post with active replies | Post disappears. Replies remain as orphans with "[deleted]" parent placeholder. |
| Message to user who has blocked you | `messages.send` returns `"sent"` (no leak). Message is silently discarded by the backend. |
| Network partition during compose | Post is queued. Status is `"queued"`. Published on reconnect. |
| Simultaneous profile updates (multi-device, post-PoC) | LWW-Register with HLC timestamp resolves conflict. Latest write wins. |
| Media chunk unavailable on network | Blurhash placeholder stays visible. Retry in background with exponential backoff. After 5 failures, show "[media unavailable]" overlay. |
| PoW timeout during identity creation | Show error on the Creating screen with "Try Again" button. Suggest closing other resource-heavy applications. |
| Recovery with mnemonic on a second device | Identity restored but connections and local data do not transfer. New node identity generated. Profile syncs from DHT. Connections require re-establishing (post-PoC: multi-device sync addresses this). |
| Very long post (near limits) | Character counter shows remaining grapheme clusters. Counter turns red at < 50 remaining. "Post" button disabled at limit. Wire size check is backend-enforced. |
| Rapid scrolling through feed | Virtual scroller recycles nodes. Blurhash placeholders shown for fast-scrolling images. Actual image load deferred until scroll settles (300ms debounce). |

---

## 11. Type Definitions (TypeScript)

Core TypeScript types for the frontend, generated from the Rust data model:

```typescript
// api/types.ts

/** Bech32m-encoded Ed25519 public key with eph1 prefix */
type PseudonymId = string;

/** Hex-encoded BLAKE3 hash with version prefix (66 chars) */
type ContentHash = string;

/** Unix milliseconds UTC */
type Timestamp = number;

/** Opaque pagination cursor */
type Cursor = string;

interface Post {
  content_hash: ContentHash;
  author: PseudonymId;
  author_display_name: string;
  body: string | null;
  body_html: string | null;
  media: MediaAttachment[];
  tags: string[];
  mentions: MentionTag[];
  parent: ContentHash | null;
  root: ContentHash | null;
  depth: number;
  created_at: Timestamp;
  expires_at: Timestamp;
  ttl_seconds: number;
  remaining_seconds: number;
  reactions: ReactionCounts;
  my_reaction: ReactionEmoji | null;
  reply_count: number;
  is_own: boolean;
  sensitivity: SensitivityLabel | null;
}

interface MediaAttachment {
  media_type: "image";
  mime_type: string;
  blurhash: string | null;
  alt_text: string | null;
  width: number | null;
  height: number | null;
  variants: MediaVariant[];
}

interface MediaVariant {
  quality: "original" | "display" | "thumbnail";
  cid: ContentHash;
  size_bytes: number;
  url: string;
}

interface MentionTag {
  pseudonym_id: PseudonymId;
  display_hint: string;
}

interface ReactionCounts {
  heart: number;
  laugh: number;
  fire: number;
  sad: number;
  thinking: number;
}

type ReactionEmoji = "heart" | "laugh" | "fire" | "sad" | "thinking";

type SensitivityLabel = "nudity" | "violence" | "spoiler";

interface Connection {
  pseudonym_id: PseudonymId;
  display_name: string;
  avatar_seed: string;
  status: "connected" | "pending_incoming" | "pending_outgoing";
  since: Timestamp;
  message: string | null;
}

interface Conversation {
  peer: PseudonymId;
  peer_display_name: string;
  peer_avatar_seed: string;
  last_message_preview: string;
  last_message_at: Timestamp;
  unread_count: number;
  is_request: boolean;
  is_connection: boolean;
}

interface Message {
  message_id: string;
  sender: PseudonymId;
  recipient: PseudonymId;
  body: string;
  media: MediaAttachment | null;
  created_at: Timestamp;
  expires_at: Timestamp;
  remaining_seconds: number;
  is_own: boolean;
  is_read: boolean;
  status: "sent" | "delivered" | "read" | "queued" | "failed";
}

interface Profile {
  pseudonym_id: PseudonymId;
  display_name: string;
  bio: string | null;
  avatar_seed: string;
  avatar_cid: ContentHash | null;
  created_at: Timestamp;
  is_connection: boolean;
  connection_status: "pending_outgoing" | "pending_incoming" | "connected" | null;
  is_blocked: boolean;
  is_muted: boolean;
  is_self: boolean;
  is_warming: boolean;
}

interface NodeStatus {
  state: "offline" | "connecting" | "online" | "syncing";
  uptime_seconds: number;
  peer_count: number;
  relay_connected: boolean;
  transport_tier: "T1" | "T2" | "T3" | "none";
  sync_state: "idle" | "syncing" | "synced";
  sync_progress: number;
  last_sync_at: Timestamp;
  storage: StorageStats;
  identity: IdentitySummary;
  network: NetworkStats;
}

interface StorageStats {
  used_bytes: number;
  quota_bytes: number;
  usage_percent: number;
  post_count: number;
  media_chunk_count: number;
  oldest_content_at: Timestamp;
}

interface IdentitySummary {
  active_pseudonym: PseudonymId;
  display_name: string;
  pseudonym_count: number;
  is_warming: boolean;
}

interface NetworkStats {
  gossip_topics: number;
  dht_routing_entries: number;
  bandwidth_in_bps: number;
  bandwidth_out_bps: number;
}
```

---

## 12. Security Considerations for the Frontend

### 12.1 Content Security Policy

The CSP restricts the webview to prevent XSS and injection:

```
default-src 'self';
style-src 'self' 'unsafe-inline';
img-src 'self' blob: data: ephemera:;
font-src 'self';
connect-src 'self';
script-src 'self';
```

- No remote resource loading. All assets are bundled.
- `ephemera:` scheme allowed for media protocol handler.
- `'unsafe-inline'` for styles only (required by SolidJS's reactivity system for dynamic styles). No inline scripts.

### 12.2 Input Sanitization

- All user-generated Markdown is rendered through a strict allowlist parser. No raw `innerHTML` except through the sanitized `body_html` field from the backend.
- URLs in post bodies are rendered as `<a>` tags with `rel="noopener noreferrer"` and `target="_blank"`.
- Display names are escaped before rendering. No HTML interpretation.

### 12.3 Sensitive Data Handling

- **Recovery phrase:** Displayed only on the dedicated RecoveryPhrase screen. Never stored in component state beyond the display session. Cleared from memory on navigation away.
- **Passphrase:** Never stored in frontend state. Passed directly to `invoke()` and discarded.
- **Message content:** Plaintext messages exist only in the SolidJS store and are not persisted to browser storage (localStorage, IndexedDB). All persistence is in the encrypted SQLite backend.
- **No telemetry, no analytics, no error reporting to external services.** Logs are local-only.

### 12.4 Deep Link Handling

The app registers `ephemera://` as a custom URL scheme:

| URL Pattern | Action |
|-------------|--------|
| `ephemera://connect/<pubkey>` | Open connection request dialog for the specified pseudonym. |
| `ephemera://post/<content_hash>` | Navigate to post detail view. |
| `ephemera://media/<cid>` | Internal media fetch (not user-facing). |

Deep links are validated before processing. Malformed URLs are silently ignored.

---

*Ephemera Client Architecture & API Specification v1.0 -- 2026-03-26*
*Implementation sub-document 06 of the Ephemera Unified Architecture.*
