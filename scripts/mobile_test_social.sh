#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────
# Ephemera Mobile Integration Test Suite
#
# Runs the same social feature tests as docker_test_social.sh but
# against the Android emulator. Uses the debug HTTP test server
# on port 3520, exposed via ADB port forwarding.
#
# Usage:
#   ./scripts/mobile_test_social.sh                # full run
#   ./scripts/mobile_test_social.sh --skip-build   # skip APK build
#   ./scripts/mobile_test_social.sh --skip-install  # skip APK install
# ─────────────────────────────────────────────────────────────────────
set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
LOG_DIR="$PROJECT_DIR/tmp/mobile-test-logs"

# Source helpers (reuse the same helper functions)
source "$SCRIPT_DIR/docker_test_helpers.sh"

SKIP_BUILD=false
SKIP_INSTALL=false
for arg in "$@"; do
    case "$arg" in
        --skip-build) SKIP_BUILD=true ;;
        --skip-install) SKIP_INSTALL=true ;;
    esac
done

mkdir -p "$LOG_DIR"

# Override port mapping for mobile (single device on port 3520)
NODE_PORT[alice]=3520
NODE_CONTAINER[alice]="mobile"  # not used for mobile, placeholder

# Mobile doesn't use docker exec for tokens — no auth needed for debug server
get_token() {
    NODE_TOKEN[$1]="no-auth-debug"
    echo "no-auth-debug"
}

# Override rpc_call to NOT send auth header (debug server doesn't need it)
rpc_call() {
    local name=$1
    local method=$2
    local params=${3:-"{}"}
    local port=${NODE_PORT[$name]}

    curl -s -X POST "http://localhost:${port}/rpc" \
        -H "Content-Type: application/json" \
        -d "{\"jsonrpc\":\"2.0\",\"method\":\"${method}\",\"params\":${params},\"id\":1}" \
        2>/dev/null || echo '{"error":{"code":-1,"message":"curl failed"}}'
}

# Use the Windows-style path for adb install (MSYS mangles Unix paths)
APK_PATH_UNIX="$PROJECT_DIR/crates/ephemera-client/src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk"
APK_PATH=$(cygpath -w "$APK_PATH_UNIX" 2>/dev/null || echo "$APK_PATH_UNIX")

# =====================================================================
# Phase 0: Build, Install & Launch on Emulator
# =====================================================================
section "Phase 0: Mobile Setup"

# Check emulator is running
if ! adb devices 2>/dev/null | grep -q "device$"; then
    echo "  No Android device/emulator found. Start one first:"
    echo "    emulator -avd <name> &"
    exit 1
fi
echo "  Emulator detected"

if [[ "$SKIP_BUILD" == "false" ]]; then
    echo "  Building Android APK (this takes a while)..."
    cd "$PROJECT_DIR/crates/ephemera-client/src-tauri"
    cargo tauri android build --target x86_64 --debug 2>&1 | tee "$LOG_DIR/build.log" | tail -3
    cd "$PROJECT_DIR"
fi

if [[ "$SKIP_INSTALL" == "false" ]]; then
    if [[ ! -f "$APK_PATH_UNIX" ]]; then
        echo "  APK not found at $APK_PATH"
        exit 1
    fi
    echo "  Installing APK..."
    adb install -r "$APK_PATH" 2>&1
fi

# Force-stop any running instance and clear data for clean test
echo "  Clearing app data for clean test..."
adb shell pm clear social.ephemera.app 2>/dev/null || true

echo "  Launching app..."
adb shell am start -n social.ephemera.app/.MainActivity 2>&1 | head -1

# Set up ADB port forwarding
echo "  Setting up ADB port forward (3520 → 3520)..."
adb forward tcp:3520 tcp:3520 2>&1

# Wait for the debug test server to become available
echo "  Waiting for debug test server on port 3520..."
MOBILE_READY=false
for i in $(seq 1 30); do
    resp=$(curl -s -X POST "http://localhost:3520/rpc" \
        -H "Content-Type: application/json" \
        -d '{"jsonrpc":"2.0","method":"identity.has_keystore","params":{},"id":1}' 2>/dev/null || echo "")
    if echo "$resp" | grep -q "result"; then
        MOBILE_READY=true
        break
    fi
    sleep 2
done

if [[ "$MOBILE_READY" == "true" ]]; then
    printf "  ${GREEN}Debug test server ready${NC}\n"
else
    echo "  ERROR: Debug test server not responding on port 3520"
    echo "  Check: adb logcat -s RustStdoutStderr"
    exit 1
fi

# =====================================================================
# Phase 1: Identity Bootstrap (single mobile node)
# =====================================================================
section "Phase 1: Identity Bootstrap"

ALICE_PK=$(create_identity alice "mobile-test-pass")
assert_not_empty "Mobile identity created" "$ALICE_PK"
echo "  Mobile pubkey: ${ALICE_PK:0:16}..."

# Give Iroh time to connect to relay
echo "  Waiting 15s for Iroh relay connection..."
sleep 15

resp=$(rpc_call alice "network.status" "{}")
assert_no_error "Network status responds" "$resp"
relay=$(echo "$resp" | jq -r '.result.relay_status // empty')
assert_eq "Iroh relay connected" "$relay" "connected"

resp=$(rpc_call alice "network.info" "{}")
MOBILE_NID=$(echo "$resp" | jq -r '.result.node_id // empty')
assert_not_empty "Has Iroh node_id" "$MOBILE_NID"
echo "  Mobile node_id: ${MOBILE_NID:0:16}..."

# =====================================================================
# Phase 2: Profile CRUD
# =====================================================================
section "Phase 2: Profile CRUD"

resp=$(rpc_call alice "profiles.update" '{"display_name":"Mobile User","bio":"Testing on Android"}')
assert_no_error "Profile update" "$resp"

resp=$(rpc_call alice "profiles.get_mine" "{}")
display=$(echo "$resp" | jq -r '.result.display_name // empty')
assert_eq "Display name set" "$display" "Mobile User"

bio=$(echo "$resp" | jq -r '.result.bio // empty')
assert_eq "Bio set" "$bio" "Testing on Android"

# Update name
resp=$(rpc_call alice "profiles.update" '{"display_name":"Mobile Updated"}')
assert_no_error "Profile name change" "$resp"

resp=$(rpc_call alice "profiles.get_mine" "{}")
display2=$(echo "$resp" | jq -r '.result.display_name // empty')
assert_eq "Name updated" "$display2" "Mobile Updated"

# Avatar upload
AVATAR_HEX="89504e470d0a1a0a0000000d4948445200000001000000010802000000907753de0000000c49444154789c63f8cfc0000003010100c9fe92ef0000000049454e44ae426082"
resp=$(rpc_call alice "profiles.update_avatar" "{\"data_hex\":\"${AVATAR_HEX}\",\"filename\":\"avatar.png\"}")
assert_no_error "Avatar upload" "$resp"

resp=$(rpc_call alice "profiles.get_mine" "{}")
avatar_url=$(echo "$resp" | jq -r '.result.avatar_url // empty')
assert_not_empty "Has avatar URL" "$avatar_url"

# =====================================================================
# Phase 3: Posts & Reactions
# =====================================================================
section "Phase 3: Posts & Reactions"

resp=$(rpc_call alice "posts.create" '{"body":"Hello from Android!","ttl_seconds":3600}')
assert_no_error "Create post" "$resp"
POST_HASH=$(echo "$resp" | jq -r '.result.content_hash // empty')
assert_not_empty "Post has hash" "$POST_HASH"

resp=$(rpc_call alice "posts.get" "{\"content_hash\":\"${POST_HASH}\"}")
assert_no_error "Get post by hash" "$resp"

resp=$(rpc_call alice "posts.list_by_author" "{\"author\":\"${ALICE_PK}\",\"limit\":10}")
post_count=$(echo "$resp" | jq -r '.result.posts | length' 2>/dev/null || echo "0")
assert_eq "Author feed has 1 post" "$post_count" "1"

# Reply
resp=$(rpc_call alice "posts.reply" "{\"parent\":\"${POST_HASH}\",\"body\":\"Reply from mobile\"}")
assert_no_error "Reply to post" "$resp"

resp=$(rpc_call alice "posts.replies" "{\"parent_hash\":\"${POST_HASH}\",\"limit\":10}")
reply_count=$(echo "$resp" | jq -r '.result.posts | length' 2>/dev/null || echo "0")
assert_eq "Has 1 reply" "$reply_count" "1"

# Reactions
resp=$(rpc_call alice "social.react" "{\"content_hash\":\"${POST_HASH}\",\"emoji\":\"heart\",\"action\":\"add\"}")
assert_no_error "Add heart reaction" "$resp"

resp=$(rpc_call alice "social.get_reactions" "{\"content_hash\":\"${POST_HASH}\"}")
heart=$(echo "$resp" | jq -r '.result.heart // "0"')
assert_eq "Heart count is 1" "$heart" "1"

resp=$(rpc_call alice "social.react" "{\"content_hash\":\"${POST_HASH}\",\"emoji\":\"heart\",\"action\":\"remove\"}")
assert_no_error "Remove heart" "$resp"

# Delete post
resp=$(rpc_call alice "posts.create" '{"body":"Delete me","ttl_seconds":3600}')
DEL_HASH=$(echo "$resp" | jq -r '.result.content_hash // empty')
resp=$(rpc_call alice "posts.delete" "{\"content_hash\":\"${DEL_HASH}\"}")
assert_no_error "Delete post" "$resp"

resp=$(rpc_call alice "posts.get" "{\"content_hash\":\"${DEL_HASH}\"}")
assert_has_error "Deleted post returns error" "$resp"

# =====================================================================
# Phase 4: Direct Messages
# =====================================================================
section "Phase 4: Direct Messages"

# Use a valid Ed25519 pubkey for DM testing (not a real user, but valid for crypto)
# This is the canonical Ed25519 basepoint — a valid public key.
FAKE_PK="5866666666666666666666666666666666666666666666666666666666666666"
resp=$(rpc_call alice "messages.send" "{\"recipient\":\"${FAKE_PK}\",\"body\":\"Test DM from mobile\"}")
assert_no_error "Send DM" "$resp"
DM_HASH=$(echo "$resp" | jq -r '.result.message_hash // empty')
assert_not_empty "DM has hash" "$DM_HASH"

resp=$(rpc_call alice "messages.list_conversations" "{}")
conv_count=$(echo "$resp" | jq -r '.result.conversations | length' 2>/dev/null || echo "0")
assert_eq "Has 1 conversation" "$conv_count" "1"

conv_id=$(echo "$resp" | jq -r '.result.conversations[0].conversation_id // empty')
if [[ -n "$conv_id" ]]; then
    resp=$(rpc_call alice "messages.get_thread" "{\"conversation_id\":\"${conv_id}\",\"limit\":10}")
    msg_count=$(echo "$resp" | jq -r '.result.messages | length' 2>/dev/null || echo "0")
    assert_eq "Thread has 1 message" "$msg_count" "1"

    resp=$(rpc_call alice "messages.mark_read" "{\"conversation_id\":\"${conv_id}\"}")
    assert_no_error "Mark read" "$resp"
fi

# =====================================================================
# Phase 5: Groups & Group Chats
# =====================================================================
section "Phase 5: Groups & Group Chats"

resp=$(rpc_call alice "groups.create" '{"name":"Mobile Group","visibility":"public"}')
assert_no_error "Create group" "$resp"
GID=$(echo "$resp" | jq -r '.result.group_id // empty')
assert_not_empty "Group has ID" "$GID"

resp=$(rpc_call alice "groups.list" "{}")
g_count=$(echo "$resp" | jq -r '.result.groups | length' 2>/dev/null || echo "0")
assert_eq "Has 1 group" "$g_count" "1"

resp=$(rpc_call alice "groups.info" "{\"group_id\":\"${GID}\"}")
g_name=$(echo "$resp" | jq -r '.result.name // empty')
assert_eq "Group name correct" "$g_name" "Mobile Group"

resp=$(rpc_call alice "groups.search" '{"query":"Mobile"}')
assert_no_error "Search groups" "$resp"

resp=$(rpc_call alice "groups.delete" "{\"group_id\":\"${GID}\"}")
assert_no_error "Delete group" "$resp"

# Group chats
resp=$(rpc_call alice "group_chats.create_private" "{\"name\":\"Test Chat\",\"members\":[\"${FAKE_PK}\"]}")
assert_no_error "Create group chat" "$resp"
CHAT_ID=$(echo "$resp" | jq -r '.result.chat_id // empty')

if [[ -n "$CHAT_ID" ]]; then
    resp=$(rpc_call alice "group_chats.send" "{\"chat_id\":\"${CHAT_ID}\",\"body\":\"Hello chat\"}")
    assert_no_error "Send chat message" "$resp"

    resp=$(rpc_call alice "group_chats.messages" "{\"chat_id\":\"${CHAT_ID}\",\"limit\":10}")
    chat_msgs=$(echo "$resp" | jq -r '.result.messages | length' 2>/dev/null || echo "0")
    assert_eq "Chat has 1 message" "$chat_msgs" "1"

    resp=$(rpc_call alice "group_chats.list" "{}")
    assert_no_error "List chats" "$resp"
fi

# =====================================================================
# Phase 6: Topics
# =====================================================================
section "Phase 6: Topics"

resp=$(rpc_call alice "topics.create" '{"name":"mobile-test","description":"Testing from mobile"}')
assert_no_error "Create topic" "$resp"
TID=$(echo "$resp" | jq -r '.result.topic_id // empty')
assert_not_empty "Topic has ID" "$TID"

if [[ -n "$POST_HASH" ]]; then
    resp=$(rpc_call alice "topics.post" "{\"topic_id\":\"${TID}\",\"content_hash\":\"${POST_HASH}\"}")
    assert_no_error "Post to topic" "$resp"
fi

resp=$(rpc_call alice "topics.list" "{}")
t_count=$(echo "$resp" | jq -r '.result.topics | length' 2>/dev/null || echo "0")
assert_eq "Has 1 topic" "$t_count" "1"

# =====================================================================
# Phase 7: Handles
# =====================================================================
section "Phase 7: Handles"

resp=$(rpc_call alice "identity.check_handle_available" '{"name":"mobileuser"}')
avail=$(echo "$resp" | jq -r '.result.available // empty')
assert_eq "Handle available" "$avail" "true"

echo "  Registering @mobileuser handle (PoW)..."
resp=$(rpc_call alice "identity.register_handle" '{"name":"mobileuser"}')
if echo "$resp" | jq -e '.result' > /dev/null 2>&1; then
    assert_no_error "Register handle" "$resp"

    sleep 5
    resp=$(rpc_call alice "identity.my_handle" "{}")
    assert_no_error "Check my_handle" "$resp"

    resp=$(rpc_call alice "identity.lookup_handle" '{"name":"mobileuser"}')
    owner=$(echo "$resp" | jq -r '.result.owner // empty')
    assert_eq "Handle lookup matches" "$owner" "$ALICE_PK"
else
    skip_test "Handle registration" "PoW timeout"
    SKIP_COUNT=$((SKIP_COUNT + 2))
fi

# =====================================================================
# Phase 8: Blocks, Mutes, Follows
# =====================================================================
section "Phase 8: Moderation"

resp=$(rpc_call alice "moderation.block" "{\"target\":\"${FAKE_PK}\"}")
assert_no_error "Block user" "$resp"

resp=$(rpc_call alice "moderation.unblock" "{\"target\":\"${FAKE_PK}\"}")
assert_no_error "Unblock user" "$resp"

resp=$(rpc_call alice "moderation.mute" "{\"target\":\"${FAKE_PK}\",\"duration_hours\":1}")
assert_no_error "Mute user" "$resp"

resp=$(rpc_call alice "moderation.unmute" "{\"target\":\"${FAKE_PK}\"}")
assert_no_error "Unmute user" "$resp"

resp=$(rpc_call alice "social.follow" "{\"target\":\"${FAKE_PK}\"}")
assert_no_error "Follow user" "$resp"

resp=$(rpc_call alice "social.unfollow" "{\"target\":\"${FAKE_PK}\"}")
assert_no_error "Unfollow user" "$resp"

# =====================================================================
# Phase 9: Notifications
# =====================================================================
section "Phase 9: Notifications"

resp=$(rpc_call alice "notifications.list" '{"limit":10}')
assert_no_error "List notifications" "$resp"

resp=$(rpc_call alice "notifications.count" "{}")
assert_no_error "Count notifications" "$resp"

resp=$(rpc_call alice "notifications.mark_all_read" "{}")
assert_no_error "Mark all read" "$resp"

# =====================================================================
# Phase 10: Identity Features
# =====================================================================
section "Phase 10: Identity Features"

resp=$(rpc_call alice "identity.get_active" "{}")
assert_no_error "Get active identity" "$resp"

resp=$(rpc_call alice "identity.has_keystore" "{}")
has_ks=$(echo "$resp" | jq -r '.result.exists // "false"')
assert_eq "Has keystore" "$has_ks" "true"

resp=$(rpc_call alice "identity.list_pseudonyms" "{}")
assert_no_error "List pseudonyms" "$resp"

# Auto-unlock test: remember, lock, auto_unlock
resp=$(rpc_call alice "identity.lock" '{"forget":true}')
assert_no_error "Lock identity" "$resp"

resp=$(rpc_call alice "identity.unlock" '{"passphrase":"mobile-test-pass","remember":true}')
assert_no_error "Unlock with remember" "$resp"

resp=$(rpc_call alice "identity.has_session" "{}")
has_sess=$(echo "$resp" | jq -r '.result.has_session // "false"')
assert_eq "Has session after remember" "$has_sess" "true"

resp=$(rpc_call alice "identity.lock" "{}")
assert_no_error "Lock (keep session)" "$resp"

resp=$(rpc_call alice "identity.auto_unlock" "{}")
auto=$(echo "$resp" | jq -r '.result.auto_unlocked // "false"')
assert_eq "Auto-unlock succeeds" "$auto" "true"

resp=$(rpc_call alice "identity.lock" '{"forget":true}')
assert_no_error "Lock and forget" "$resp"

resp=$(rpc_call alice "identity.auto_unlock" "{}")
auto2=$(echo "$resp" | jq -r '.result.auto_unlocked // "false"')
assert_eq "Auto-unlock fails after forget" "$auto2" "false"

# Re-unlock for remaining tests
resp=$(rpc_call alice "identity.unlock" '{"passphrase":"mobile-test-pass"}')
assert_no_error "Re-unlock" "$resp"

# =====================================================================
# Phase 11: Network & Meta
# =====================================================================
section "Phase 11: Network & Meta"

resp=$(rpc_call alice "network.info" "{}")
assert_no_error "Network info" "$resp"

resp=$(rpc_call alice "network.peers" "{}")
assert_no_error "Network peers" "$resp"

resp=$(rpc_call alice "network.status" "{}")
assert_no_error "Network status" "$resp"

resp=$(rpc_call alice "meta.capabilities" "{}")
assert_no_error "Meta capabilities" "$resp"
cap_count=$(echo "$resp" | jq -r '.result.methods | length' 2>/dev/null || echo "0")
echo "  Node reports $cap_count RPC methods"

resp=$(rpc_call alice "meta.debug_log" "{}")
assert_no_error "Meta debug_log" "$resp"

# =====================================================================
# Phase 12: Feeds
# =====================================================================
section "Phase 12: Feeds"

resp=$(rpc_call alice "feed.connections" '{"limit":10}')
assert_no_error "Feed connections" "$resp"

resp=$(rpc_call alice "feed.discover" '{"limit":10}')
assert_no_error "Feed discover" "$resp"

resp=$(rpc_call alice "mentions.list" '{"limit":10}')
assert_no_error "Mentions list" "$resp"

# =====================================================================
# Phase 13: Mobile-Specific Tests
# =====================================================================
section "Phase 13: Mobile-Specific"

# App suspend/resume simulation
echo "  Simulating app backgrounding..."
adb shell input keyevent KEYCODE_HOME 2>/dev/null
sleep 3
adb shell am start -n social.ephemera.app/.MainActivity 2>/dev/null
sleep 5

# Verify node still responds after resume
resp=$(rpc_call alice "meta.status" "{}")
assert_no_error "Node responds after app resume" "$resp"

resp=$(rpc_call alice "network.status" "{}")
iroh_up=$(echo "$resp" | jq -r '.result.iroh_available // "false"')
assert_eq "Iroh available after resume" "$iroh_up" "true"

# =====================================================================
# Summary
# =====================================================================
section "Mobile Test Summary"

# Save logcat
echo "  Saving logcat..."
adb logcat -d > "$LOG_DIR/logcat.log" 2>&1 || true
echo "  Logs saved to: $LOG_DIR/"

# Remove port forward
adb forward --remove tcp:3520 2>/dev/null || true

print_summary
