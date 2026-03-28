#!/bin/bash
# Run tests and show per-crate pass/fail summary
cd "$(dirname "$0")/.."
echo "=== Ephemera Test Coverage ==="
for crate in crates/*/; do
    name=$(basename "$crate")
    result=$(cargo test -p "$name" 2>&1)
    passed=$(echo "$result" | grep "test result:" | grep -oP '\d+ passed' | head -1)
    failed=$(echo "$result" | grep "test result:" | grep -oP '\d+ failed' | head -1)
    if echo "$result" | grep -q "FAILED"; then
        echo "FAIL  $name: $passed, $failed"
    else
        echo "  OK  $name: $passed"
    fi
done
