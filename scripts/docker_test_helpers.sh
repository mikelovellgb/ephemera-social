#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────
# Ephemera Docker integration test helpers
# ─────────────────────────────────────────────────────────────────────
set -euo pipefail

# Prevent Git Bash (MSYS/MINGW) from mangling Unix paths like /data
export MSYS_NO_PATHCONV=1

# Colours
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
CYAN='\033[0;36m'
NC='\033[0m'

# Counters
PASS_COUNT=0
FAIL_COUNT=0
SKIP_COUNT=0

# Container → port mapping
declare -A NODE_PORT
NODE_PORT[alice]=3510
NODE_PORT[bob]=3511
NODE_PORT[charlie]=3512

declare -A NODE_CONTAINER
NODE_CONTAINER[alice]=ephemera-test-alice
NODE_CONTAINER[bob]=ephemera-test-bob
NODE_CONTAINER[charlie]=ephemera-test-charlie

# Token cache (populated by get_token)
declare -A NODE_TOKEN

# Pubkey cache (populated by create_identity / get_pubkey)
declare -A NODE_PUBKEY

# ─────────────────────────────────────────────────────────────────────
# get_token <name>  — read the RPC auth token from a running container
# ─────────────────────────────────────────────────────────────────────
get_token() {
    local name=$1
    local container=${NODE_CONTAINER[$name]}
    local token
    # MSYS_NO_PATHCONV prevents Git Bash from mangling /data to C:/Program Files/Git/data
    token=$(MSYS_NO_PATHCONV=1 docker exec "$container" cat /data/rpc_token 2>/dev/null || echo "")
    if [[ -z "$token" ]]; then
        echo ""
        return 1
    fi
    NODE_TOKEN[$name]="$token"
    echo "$token"
}

# ─────────────────────────────────────────────────────────────────────
# rpc_call <name> <method> [params_json]
# Makes an authenticated JSON-RPC 2.0 call. Returns the full response.
# ─────────────────────────────────────────────────────────────────────
rpc_call() {
    local name=$1
    local method=$2
    local params=${3:-"{}"}
    local port=${NODE_PORT[$name]}
    local token=${NODE_TOKEN[$name]:-$(get_token "$name")}

    curl -s -X POST "http://localhost:${port}/rpc" \
        -H "Content-Type: application/json" \
        -H "Authorization: Bearer ${token}" \
        -d "{\"jsonrpc\":\"2.0\",\"method\":\"${method}\",\"params\":${params},\"id\":1}" \
        2>/dev/null || echo '{"error":{"code":-1,"message":"curl failed"}}'
}

# ─────────────────────────────────────────────────────────────────────
# rpc_result <name> <method> [params_json]
# Like rpc_call but returns only .result (strips envelope).
# ─────────────────────────────────────────────────────────────────────
rpc_result() {
    local resp
    resp=$(rpc_call "$@")
    echo "$resp" | jq -r '.result // empty'
}

# ─────────────────────────────────────────────────────────────────────
# wait_for_node <name> [timeout_secs]
# Poll until the node's RPC endpoint is responding.
# ─────────────────────────────────────────────────────────────────────
wait_for_node() {
    local name=$1
    local timeout=${2:-60}
    local port=${NODE_PORT[$name]}
    local container=${NODE_CONTAINER[$name]}
    local elapsed=0

    printf "  Waiting for %-8s (port %s) ..." "$name" "$port"
    while (( elapsed < timeout )); do
        # Try to read the RPC token directly — if it exists, the node is up
        local tok
        tok=$(docker exec "$container" cat /data/rpc_token 2>/dev/null) || tok=""
        if [[ -n "$tok" && ${#tok} -ge 32 ]]; then
            NODE_TOKEN[$name]="$tok"
            printf " ${GREEN}ready${NC} (%ds)\n" "$elapsed"
            return 0
        fi
        sleep 2
        elapsed=$((elapsed + 2))
    done
    printf " ${RED}TIMEOUT${NC}\n"
    return 1
}

# ─────────────────────────────────────────────────────────────────────
# create_identity <name> <passphrase>
# Creates an identity and caches the pubkey.
# ─────────────────────────────────────────────────────────────────────
create_identity() {
    local name=$1
    local pass=$2
    local resp

    # Check if keystore already exists
    resp=$(rpc_call "$name" "identity.has_keystore" "{}")
    local exists
    exists=$(echo "$resp" | jq -r '.result.exists // "false"')

    if [[ "$exists" == "true" ]]; then
        # Unlock existing identity
        resp=$(rpc_call "$name" "identity.unlock" "{\"passphrase\":\"${pass}\"}")
        local err
        err=$(echo "$resp" | jq -r '.error.message // empty')
        if [[ -n "$err" ]]; then
            echo "FAILED: $err"
            return 1
        fi
    else
        # Create new identity
        resp=$(rpc_call "$name" "identity.create" "{\"passphrase\":\"${pass}\"}")
        local err
        err=$(echo "$resp" | jq -r '.error.message // empty')
        if [[ -n "$err" ]]; then
            echo "FAILED: $err"
            return 1
        fi
    fi

    # Wait briefly for network to start, then get the active pubkey
    sleep 2
    resp=$(rpc_call "$name" "identity.get_active" "{}")
    local pubkey
    pubkey=$(echo "$resp" | jq -r '.result.pubkey // empty')
    if [[ -n "$pubkey" ]]; then
        NODE_PUBKEY[$name]="$pubkey"
        echo "$pubkey"
        return 0
    fi

    echo "FAILED: could not get pubkey after identity creation"
    return 1
}

# ─────────────────────────────────────────────────────────────────────
# get_pubkey <name>
# Returns cached pubkey or fetches via identity.get_active.
# ─────────────────────────────────────────────────────────────────────
get_pubkey() {
    local name=$1
    if [[ -n "${NODE_PUBKEY[$name]:-}" ]]; then
        echo "${NODE_PUBKEY[$name]}"
        return 0
    fi
    local resp
    resp=$(rpc_call "$name" "identity.get_active" "{}")
    local pubkey
    pubkey=$(echo "$resp" | jq -r '.result.pubkey // empty')
    if [[ -n "$pubkey" ]]; then
        NODE_PUBKEY[$name]="$pubkey"
        echo "$pubkey"
        return 0
    fi
    return 1
}

# ─────────────────────────────────────────────────────────────────────
# poll_until <name> <method> <params> <jq_expr> <expected> [timeout]
# Repeatedly calls an RPC method until the jq expression on .result
# matches the expected value, or times out.
# ─────────────────────────────────────────────────────────────────────
poll_until() {
    local name=$1
    local method=$2
    local params=$3
    local jq_expr=$4
    local expected=$5
    local timeout=${6:-90}
    local elapsed=0
    local interval=3

    while (( elapsed < timeout )); do
        local resp
        resp=$(rpc_call "$name" "$method" "$params")
        local actual
        actual=$(echo "$resp" | jq -r ".result | $jq_expr // empty" 2>/dev/null || echo "")
        if [[ "$actual" == "$expected" ]]; then
            return 0
        fi
        sleep "$interval"
        elapsed=$((elapsed + interval))
        # Exponential-ish backoff, cap at 10s
        if (( interval < 10 )); then
            interval=$((interval + 2))
        fi
    done
    return 1
}

# ─────────────────────────────────────────────────────────────────────
# assert_eq <test_name> <actual> <expected>
# ─────────────────────────────────────────────────────────────────────
assert_eq() {
    local test_name=$1
    local actual=$2
    local expected=$3

    if [[ "$actual" == "$expected" ]]; then
        printf "  ${GREEN}[PASS]${NC} %s\n" "$test_name"
        PASS_COUNT=$((PASS_COUNT + 1))
    else
        printf "  ${RED}[FAIL]${NC} %s — expected '%s', got '%s'\n" "$test_name" "$expected" "$actual"
        FAIL_COUNT=$((FAIL_COUNT + 1))
    fi
}

# ─────────────────────────────────────────────────────────────────────
# assert_not_empty <test_name> <value>
# ─────────────────────────────────────────────────────────────────────
assert_not_empty() {
    local test_name=$1
    local value=$2

    if [[ -n "$value" && "$value" != "null" ]]; then
        printf "  ${GREEN}[PASS]${NC} %s\n" "$test_name"
        PASS_COUNT=$((PASS_COUNT + 1))
    else
        printf "  ${RED}[FAIL]${NC} %s — value was empty/null\n" "$test_name"
        FAIL_COUNT=$((FAIL_COUNT + 1))
    fi
}

# ─────────────────────────────────────────────────────────────────────
# assert_contains <test_name> <haystack> <needle>
# ─────────────────────────────────────────────────────────────────────
assert_contains() {
    local test_name=$1
    local haystack=$2
    local needle=$3

    if echo "$haystack" | grep -q "$needle"; then
        printf "  ${GREEN}[PASS]${NC} %s\n" "$test_name"
        PASS_COUNT=$((PASS_COUNT + 1))
    else
        printf "  ${RED}[FAIL]${NC} %s — '%s' not found in response\n" "$test_name" "$needle"
        FAIL_COUNT=$((FAIL_COUNT + 1))
    fi
}

# ─────────────────────────────────────────────────────────────────────
# assert_json_field <test_name> <json> <jq_expr> <expected>
# ─────────────────────────────────────────────────────────────────────
assert_json_field() {
    local test_name=$1
    local json=$2
    local jq_expr=$3
    local expected=$4

    local actual
    actual=$(echo "$json" | jq -r "$jq_expr" 2>/dev/null || echo "JQ_ERROR")
    assert_eq "$test_name" "$actual" "$expected"
}

# ─────────────────────────────────────────────────────────────────────
# assert_no_error <test_name> <rpc_response_json>
# Checks that the RPC response has no error field.
# ─────────────────────────────────────────────────────────────────────
assert_no_error() {
    local test_name=$1
    local json=$2
    local err
    err=$(echo "$json" | jq -r '.error // empty' 2>/dev/null || echo "")
    if [[ -z "$err" ]]; then
        printf "  ${GREEN}[PASS]${NC} %s\n" "$test_name"
        PASS_COUNT=$((PASS_COUNT + 1))
    else
        local msg
        msg=$(echo "$json" | jq -r '.error.message // "unknown"' 2>/dev/null)
        printf "  ${RED}[FAIL]${NC} %s — RPC error: %s\n" "$test_name" "$msg"
        FAIL_COUNT=$((FAIL_COUNT + 1))
    fi
}

# ─────────────────────────────────────────────────────────────────────
# assert_has_error <test_name> <rpc_response_json>
# Checks that the RPC response DOES have an error (negative test).
# ─────────────────────────────────────────────────────────────────────
assert_has_error() {
    local test_name=$1
    local json=$2
    local err
    err=$(echo "$json" | jq -r '.error // empty' 2>/dev/null || echo "")
    if [[ -n "$err" ]]; then
        printf "  ${GREEN}[PASS]${NC} %s\n" "$test_name"
        PASS_COUNT=$((PASS_COUNT + 1))
    else
        printf "  ${RED}[FAIL]${NC} %s — expected an error, got success\n" "$test_name"
        FAIL_COUNT=$((FAIL_COUNT + 1))
    fi
}

# ─────────────────────────────────────────────────────────────────────
# skip_test <test_name> <reason>
# ─────────────────────────────────────────────────────────────────────
skip_test() {
    local test_name=$1
    local reason=$2
    printf "  ${YELLOW}[SKIP]${NC} %s — %s\n" "$test_name" "$reason"
    SKIP_COUNT=$((SKIP_COUNT + 1))
}

# ─────────────────────────────────────────────────────────────────────
# section <title>
# ─────────────────────────────────────────────────────────────────────
section() {
    printf "\n${CYAN}═══ %s ═══${NC}\n" "$1"
}

# ─────────────────────────────────────────────────────────────────────
# print_summary
# ─────────────────────────────────────────────────────────────────────
print_summary() {
    local total=$((PASS_COUNT + FAIL_COUNT + SKIP_COUNT))
    printf "\n${CYAN}═══════════════════════════════════════${NC}\n"
    printf "  Total: %d  |  " "$total"
    printf "${GREEN}Pass: %d${NC}  |  " "$PASS_COUNT"
    printf "${RED}Fail: %d${NC}  |  " "$FAIL_COUNT"
    printf "${YELLOW}Skip: %d${NC}\n" "$SKIP_COUNT"
    printf "${CYAN}═══════════════════════════════════════${NC}\n"
    if (( FAIL_COUNT > 0 )); then
        return 1
    fi
    return 0
}
