#!/usr/bin/env bash
# =============================================================================
# Ephemera Android Build Script
#
# Builds the Ephemera APK using Tauri 2.x for Android.
#
# Usage:
#   ./scripts/build_android.sh [--debug|--release]
#
# Prerequisites:
#   - Rust toolchain (stable, via rustup)
#   - Android SDK with NDK r26+ installed
#   - JDK 17+ (Android Studio bundled or standalone)
#   - Tauri CLI 2.x: cargo install tauri-cli@^2
#
# Environment variables (auto-detected if Android Studio is installed):
#   ANDROID_HOME    Path to Android SDK
#   ANDROID_NDK_HOME  Path to NDK (inside SDK)
#   JAVA_HOME       Path to JDK 17+
# =============================================================================

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
TAURI_DIR="$PROJECT_ROOT/crates/ephemera-client/src-tauri"

BUILD_MODE="${1:---release}"

# ---------------------------------------------------------------------------
# Colors for output
# ---------------------------------------------------------------------------
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No color

info()  { echo -e "${GREEN}[INFO]${NC} $*"; }
warn()  { echo -e "${YELLOW}[WARN]${NC} $*"; }
error() { echo -e "${RED}[ERROR]${NC} $*" >&2; }
fatal() { error "$@"; exit 1; }

# ---------------------------------------------------------------------------
# Step 1: Detect / validate prerequisites
# ---------------------------------------------------------------------------
info "Checking prerequisites..."

# Rust
if ! command -v rustup &>/dev/null; then
    fatal "rustup not found. Install from https://rustup.rs/"
fi
info "Rust: $(rustc --version)"

# Tauri CLI
if ! cargo tauri --version &>/dev/null; then
    warn "Tauri CLI not found. Installing..."
    cargo install tauri-cli@^2
fi
info "Tauri CLI: $(cargo tauri --version)"

# ---------------------------------------------------------------------------
# Step 2: Detect Android SDK
# ---------------------------------------------------------------------------

# Try common locations on Windows and Linux/macOS
detect_android_sdk() {
    if [[ -n "${ANDROID_HOME:-}" ]] && [[ -d "$ANDROID_HOME" ]]; then
        echo "$ANDROID_HOME"
        return
    fi
    if [[ -n "${ANDROID_SDK_ROOT:-}" ]] && [[ -d "$ANDROID_SDK_ROOT" ]]; then
        echo "$ANDROID_SDK_ROOT"
        return
    fi
    # Windows default
    local win_default="$HOME/AppData/Local/Android/Sdk"
    if [[ -d "$win_default" ]]; then
        echo "$win_default"
        return
    fi
    # Linux default
    local linux_default="$HOME/Android/Sdk"
    if [[ -d "$linux_default" ]]; then
        echo "$linux_default"
        return
    fi
    # macOS default
    local mac_default="$HOME/Library/Android/sdk"
    if [[ -d "$mac_default" ]]; then
        echo "$mac_default"
        return
    fi
    echo ""
}

ANDROID_HOME="$(detect_android_sdk)"
if [[ -z "$ANDROID_HOME" ]]; then
    fatal "Android SDK not found. Install Android Studio or set ANDROID_HOME."
fi
export ANDROID_HOME
info "Android SDK: $ANDROID_HOME"

# ---------------------------------------------------------------------------
# Step 3: Detect Android NDK
# ---------------------------------------------------------------------------

detect_ndk() {
    if [[ -n "${ANDROID_NDK_HOME:-}" ]] && [[ -d "$ANDROID_NDK_HOME" ]]; then
        echo "$ANDROID_NDK_HOME"
        return
    fi
    if [[ -n "${NDK_HOME:-}" ]] && [[ -d "$NDK_HOME" ]]; then
        echo "$NDK_HOME"
        return
    fi
    # Find newest NDK in SDK directory
    local ndk_dir="$ANDROID_HOME/ndk"
    if [[ -d "$ndk_dir" ]]; then
        local newest
        newest=$(ls -1 "$ndk_dir" 2>/dev/null | sort -V | tail -1)
        if [[ -n "$newest" ]]; then
            echo "$ndk_dir/$newest"
            return
        fi
    fi
    echo ""
}

ANDROID_NDK_HOME="$(detect_ndk)"
if [[ -z "$ANDROID_NDK_HOME" ]]; then
    warn "Android NDK not found. Attempting to install via sdkmanager..."
    if [[ -x "$ANDROID_HOME/cmdline-tools/latest/bin/sdkmanager" ]]; then
        "$ANDROID_HOME/cmdline-tools/latest/bin/sdkmanager" "ndk;27.1.12297006"
        ANDROID_NDK_HOME="$ANDROID_HOME/ndk/27.1.12297006"
    else
        fatal "NDK not found and sdkmanager not available. Install NDK r26+ via Android Studio SDK Manager."
    fi
fi
export ANDROID_NDK_HOME
export NDK_HOME="$ANDROID_NDK_HOME"
info "Android NDK: $ANDROID_NDK_HOME"

# ---------------------------------------------------------------------------
# Step 4: Detect JDK
# ---------------------------------------------------------------------------

detect_java_home() {
    if [[ -n "${JAVA_HOME:-}" ]] && [[ -d "$JAVA_HOME" ]]; then
        echo "$JAVA_HOME"
        return
    fi
    # Android Studio bundled JBR (Windows)
    local as_jbr="C:/Program Files/Android/Android Studio/jbr"
    if [[ -d "$as_jbr" ]]; then
        echo "$as_jbr"
        return
    fi
    # System JDK (Windows)
    for jdk_dir in "C:/Program Files/Java"/jdk-*; do
        if [[ -d "$jdk_dir" ]]; then
            echo "$jdk_dir"
            return
        fi
    done
    # macOS
    if [[ -x "/usr/libexec/java_home" ]]; then
        local mac_java
        mac_java=$(/usr/libexec/java_home 2>/dev/null || true)
        if [[ -n "$mac_java" ]]; then
            echo "$mac_java"
            return
        fi
    fi
    echo ""
}

JAVA_HOME="$(detect_java_home)"
if [[ -z "$JAVA_HOME" ]]; then
    fatal "JDK not found. Install JDK 17+ or Android Studio (which bundles one)."
fi
export JAVA_HOME
export PATH="$JAVA_HOME/bin:$PATH"
info "JDK: $JAVA_HOME"
info "Java: $("$JAVA_HOME/bin/java" -version 2>&1 | head -1)"

# ---------------------------------------------------------------------------
# Step 5: Add Rust Android targets
# ---------------------------------------------------------------------------
info "Adding Rust Android cross-compilation targets..."

TARGETS=(
    "aarch64-linux-android"
    "armv7-linux-androideabi"
    "x86_64-linux-android"
    "i686-linux-android"
)

for target in "${TARGETS[@]}"; do
    if ! rustup target list --installed | grep -q "$target"; then
        info "  Adding target: $target"
        rustup target add "$target"
    else
        info "  Target already installed: $target"
    fi
done

# ---------------------------------------------------------------------------
# Step 6: Configure Cargo for NDK cross-compilation
# ---------------------------------------------------------------------------

# Determine NDK toolchain path for the linker binaries.
# NDK r23+ uses a unified toolchain under toolchains/llvm/prebuilt/<host>/bin/
detect_ndk_host_tag() {
    case "$(uname -s)" in
        Linux*)   echo "linux-x86_64" ;;
        Darwin*)  echo "darwin-x86_64" ;;
        MINGW*|MSYS*|CYGWIN*) echo "windows-x86_64" ;;
        *)        echo "linux-x86_64" ;;
    esac
}

HOST_TAG="$(detect_ndk_host_tag)"
NDK_TOOLCHAIN="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/$HOST_TAG"

if [[ ! -d "$NDK_TOOLCHAIN" ]]; then
    fatal "NDK toolchain not found at $NDK_TOOLCHAIN"
fi

# Write .cargo/config.toml for cross-compilation
CARGO_CONFIG_DIR="$PROJECT_ROOT/.cargo"
CARGO_CONFIG_FILE="$CARGO_CONFIG_DIR/config.toml"

mkdir -p "$CARGO_CONFIG_DIR"

# Use the NDK API level matching our minSdkVersion (26 = Android 8.0)
API_LEVEL=26

# Normalize path separators for Windows
NDK_BIN="${NDK_TOOLCHAIN}/bin"

info "Writing Cargo cross-compilation config to $CARGO_CONFIG_FILE"

# Determine file extension for linker binary
if [[ "$(uname -s)" == MINGW* ]] || [[ "$(uname -s)" == MSYS* ]] || [[ "$(uname -s)" == CYGWIN* ]]; then
    EXE_EXT=".cmd"
else
    EXE_EXT=""
fi

cat > "$CARGO_CONFIG_FILE" <<TOML
# Auto-generated by scripts/build_android.sh for Android cross-compilation.
# Do not edit manually -- re-run the build script to regenerate.

[target.aarch64-linux-android]
linker = "${NDK_BIN}/aarch64-linux-android${API_LEVEL}-clang${EXE_EXT}"

[target.armv7-linux-androideabi]
linker = "${NDK_BIN}/armv7a-linux-androideabi${API_LEVEL}-clang${EXE_EXT}"

[target.x86_64-linux-android]
linker = "${NDK_BIN}/x86_64-linux-android${API_LEVEL}-clang${EXE_EXT}"

[target.i686-linux-android]
linker = "${NDK_BIN}/i686-linux-android${API_LEVEL}-clang${EXE_EXT}"
TOML

# ---------------------------------------------------------------------------
# Step 7: Initialize Tauri Android project (if not already done)
# ---------------------------------------------------------------------------

if [[ ! -d "$TAURI_DIR/gen/android" ]]; then
    info "Initializing Tauri Android project..."
    cd "$TAURI_DIR"
    cargo tauri android init
else
    info "Tauri Android project already initialized."
fi

# ---------------------------------------------------------------------------
# Step 8: Build
# ---------------------------------------------------------------------------

cd "$TAURI_DIR"

if [[ "$BUILD_MODE" == "--debug" ]]; then
    info "Building debug APK..."
    cargo tauri android build --debug
else
    info "Building release APK..."
    cargo tauri android build
fi

# ---------------------------------------------------------------------------
# Step 9: Output locations
# ---------------------------------------------------------------------------

APK_DIR="$TAURI_DIR/gen/android/app/build/outputs/apk"

info ""
info "Build complete!"
info ""

if [[ -d "$APK_DIR/release" ]]; then
    info "Release APK: $APK_DIR/release/"
    ls -la "$APK_DIR/release/"*.apk 2>/dev/null || true
fi

if [[ -d "$APK_DIR/debug" ]]; then
    info "Debug APK: $APK_DIR/debug/"
    ls -la "$APK_DIR/debug/"*.apk 2>/dev/null || true
fi

if [[ -d "$APK_DIR/universal/release" ]]; then
    info "Universal Release APK: $APK_DIR/universal/release/"
    ls -la "$APK_DIR/universal/release/"*.apk 2>/dev/null || true
fi

info ""
info "To install on a connected device:"
info "  adb install <path-to-apk>"
