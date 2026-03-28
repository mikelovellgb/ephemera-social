#!/bin/bash
# Ephemera Code Quality Gate
# Run this before any commit. All checks must pass.

set -e
cd "$(dirname "$0")/.."
echo "=== Ephemera Quality Gate ==="

# 1. Compilation
echo "[1/8] Checking compilation..."
cargo check --workspace 2>&1

# 2. Tests
echo "[2/8] Running all tests..."
cargo test --workspace 2>&1

# 3. Clippy lints
echo "[3/8] Running clippy..."
cargo clippy --workspace -- -D warnings 2>&1

# 4. Format check
echo "[4/8] Checking formatting..."
cargo fmt --all -- --check 2>&1

# 5. File size check (max 300 lines)
echo "[5/8] Checking file sizes..."
# Find any .rs file over 300 lines, report it
oversized=$(find crates/ -name "*.rs" -exec awk 'END{if(NR>300) print FILENAME": "NR" lines"}' {} \;)
if [ -n "$oversized" ]; then
    echo "FAIL: Files over 300 lines:"
    echo "$oversized"
    exit 1
fi

# 6. No todo!() or unimplemented!() in non-test code
echo "[6/8] Checking for stubs..."
stubs=$(grep -rn 'todo!\|unimplemented!\|panic!("not yet")' crates/ --include="*.rs" | grep -v "#\[cfg(test)\]" | grep -v "mod tests" | grep -v "_test.rs" || true)
if [ -n "$stubs" ]; then
    echo "FAIL: Found stubs in production code:"
    echo "$stubs"
    exit 1
fi

# 7. No unwrap() in library code (main.rs is ok)
echo "[7/8] Checking for unwrap()..."
unwraps=$(grep -rn '\.unwrap()' crates/ --include="*.rs" | grep -v "test" | grep -v "main.rs" | grep -v "// SAFETY:" | grep -v "#\[cfg(test)\]" || true)
if [ -n "$unwraps" ]; then
    echo "WARNING: Found unwrap() in non-test code (use ? or expect() instead):"
    echo "$unwraps"
    # Warning only, not a blocker for now
fi

# 8. Security check — no secrets in code
echo "[8/8] Checking for secrets..."
secrets=$(grep -rn 'password\s*=\s*"\|secret\s*=\s*"\|api_key\s*=\s*"' crates/ --include="*.rs" | grep -v "test" || true)
if [ -n "$secrets" ]; then
    echo "FAIL: Possible hardcoded secrets:"
    echo "$secrets"
    exit 1
fi

echo ""
echo "=== All quality checks PASSED ==="
