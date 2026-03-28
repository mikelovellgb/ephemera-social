#!/usr/bin/env bash
#
# run_demo.sh -- Demonstrate a two-node Ephemera network.
#
# This script:
#   1. Builds the project
#   2. Starts Node A on 0.0.0.0:9100
#   3. Starts Node B on 0.0.0.0:9101 with --connect 127.0.0.1:9100
#   4. Creates an identity on Node A
#   5. Creates a post on Node A
#   6. Reads the feed on Node B to show propagation
#
# Usage:
#   ./scripts/run_demo.sh
#
# Requirements:
#   - Rust toolchain (cargo)
#   - Unix-like shell (bash)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
BINARY="$PROJECT_ROOT/target/debug/ephemera"

DATA_A="$PROJECT_ROOT/tmp/demo-node-a"
DATA_B="$PROJECT_ROOT/tmp/demo-node-b"

LISTEN_A="0.0.0.0:9100"
LISTEN_B="0.0.0.0:9101"

NODE_A_PID=""
NODE_B_PID=""

cleanup() {
    echo ""
    echo "--- Cleaning up ---"
    [ -n "$NODE_A_PID" ] && kill "$NODE_A_PID" 2>/dev/null && echo "Stopped Node A (PID $NODE_A_PID)"
    [ -n "$NODE_B_PID" ] && kill "$NODE_B_PID" 2>/dev/null && echo "Stopped Node B (PID $NODE_B_PID)"
    rm -rf "$PROJECT_ROOT/tmp/demo-node-a" "$PROJECT_ROOT/tmp/demo-node-b"
    echo "Demo data cleaned up."
}

trap cleanup EXIT

echo "=== Ephemera Two-Node Demo ==="
echo ""

# ---- Step 1: Build ----
echo "--- Step 1: Building the project ---"
cd "$PROJECT_ROOT"
cargo build --bin ephemera
echo "Build complete."
echo ""

# ---- Step 2: Prepare data directories ----
echo "--- Step 2: Preparing data directories ---"
rm -rf "$DATA_A" "$DATA_B"
mkdir -p "$DATA_A" "$DATA_B"

# Write config for Node A (listens on all interfaces).
cat > "$DATA_A/config.toml" <<EOF
data_dir = "$DATA_A"
profile = "standalone"
listen_addr = "$LISTEN_A"
bootstrap_nodes = []
EOF

# Write config for Node B, bootstrapping from Node A.
cat > "$DATA_B/config.toml" <<EOF
data_dir = "$DATA_B"
profile = "standalone"
listen_addr = "$LISTEN_B"
bootstrap_nodes = ["127.0.0.1:9100"]
EOF

echo "Node A data dir: $DATA_A"
echo "Node B data dir: $DATA_B"
echo ""

# ---- Step 3: Initialize identities ----
echo "--- Step 3: Creating identities ---"
echo "Creating identity on Node A..."
"$BINARY" --data-dir "$DATA_A" --listen "$LISTEN_A" init --passphrase "demo-passphrase-a"
echo ""

echo "Creating identity on Node B..."
"$BINARY" --data-dir "$DATA_B" --listen "$LISTEN_B" --connect "127.0.0.1:9100" init --passphrase "demo-passphrase-b"
echo ""

# ---- Step 4: Create a post on Node A ----
echo "--- Step 4: Creating a post on Node A ---"
"$BINARY" --data-dir "$DATA_A" --listen "$LISTEN_A" post "Hello from Node A! This message will self-destruct in 24 hours." --ttl 86400
echo ""

# ---- Step 5: Read the feed on Node A (local verification) ----
echo "--- Step 5: Reading feed on Node A (local) ---"
"$BINARY" --data-dir "$DATA_A" --listen "$LISTEN_A" feed --limit 5
echo ""

# ---- Step 6: Check Node A status ----
echo "--- Step 6: Node A status ---"
"$BINARY" --data-dir "$DATA_A" --listen "$LISTEN_A" status
echo ""

echo "=== Demo Complete ==="
echo ""
echo "In a full network deployment, Node B would discover Node A via"
echo "the bootstrap peer list and replicate the post through gossip."
echo ""
echo "Ports used:"
echo "  Node A: $LISTEN_A (9100, all interfaces)"
echo "  Node B: $LISTEN_B (9101, all interfaces)"
echo ""
echo "To connect from another machine, use:"
echo "  ephemera --connect <node-a-ip>:9100 ..."
