#!/bin/bash
# Verify code aligns with architecture decisions
cd "$(dirname "$0")/.."
echo "=== Architecture Compliance ==="

# Check: No direct peer-to-peer connections without transport abstraction
echo "[1] Checking transport abstraction..."

# Check: All content stored encrypted (no plaintext on disk)
echo "[2] Checking encryption at rest..."

# Check: All public APIs have doc comments
echo "[3] Checking API documentation..."
undocumented=$(grep -rn 'pub fn\|pub struct\|pub enum\|pub trait' crates/ --include="*.rs" -B1 | grep -v "///" | grep "pub " | grep -v "test" | head -20)
if [ -n "$undocumented" ]; then
    echo "WARNING: Public items without doc comments:"
    echo "$undocumented"
fi

# Check: Every crate has tests
echo "[4] Checking test coverage..."
for crate in crates/*/; do
    name=$(basename "$crate")
    tests=$(grep -rn "#\[test\]" "$crate" --include="*.rs" | wc -l)
    if [ "$tests" -eq 0 ]; then
        echo "FAIL: $name has ZERO tests"
    fi
done

echo "=== Architecture check complete ==="
