# Ephemera Mobile Build Guide

## Quick Start

### Option A: Local build (Windows/Linux/macOS with Android Studio)

```bash
./scripts/build_android.sh           # Release APK
./scripts/build_android.sh --debug   # Debug APK (faster, unsigned)
```

The script auto-detects your Android SDK, NDK, and JDK installations. It
will add the required Rust cross-compilation targets and configure cargo
linker paths automatically.

### Option B: Docker build (no local Android tooling needed)

```bash
docker build -f Dockerfile.android -t ephemera-android-builder .
docker run --rm -v "$(pwd)/output:/output" ephemera-android-builder
```

The APK appears in `./output/` on the host.

---

## Prerequisites

### Android SDK & NDK

| Component | Version | How to install |
|-----------|---------|----------------|
| Android SDK | API 34 | Android Studio SDK Manager, or `sdkmanager "platforms;android-34"` |
| Android NDK | r27.1+ | SDK Manager > SDK Tools > NDK, or `sdkmanager "ndk;27.1.12297006"` |
| Build Tools | 34.0.0 | SDK Manager, or `sdkmanager "build-tools;34.0.0"` |
| JDK | 17+ | Bundled with Android Studio at `<AS>/jbr/`, or install OpenJDK 17 |

The build script checks these common locations automatically:

- **Windows**: `%LOCALAPPDATA%\Android\Sdk` (SDK), `C:\Program Files\Android\Android Studio\jbr` (JDK), `C:\Program Files\Java\jdk-21` (standalone JDK)
- **Linux**: `$HOME/Android/Sdk`
- **macOS**: `$HOME/Library/Android/sdk`

### Rust Toolchain

```bash
rustup target add aarch64-linux-android    # ARM64 devices (most modern phones)
rustup target add armv7-linux-androideabi  # ARMv7 (older 32-bit devices)
rustup target add x86_64-linux-android    # x86_64 emulator
rustup target add i686-linux-android      # x86 emulator (32-bit)
```

### Tauri CLI 2.x

```bash
cargo install tauri-cli@^2
```

### Environment Variables

Set these if auto-detection fails:

```bash
export ANDROID_HOME="$HOME/AppData/Local/Android/Sdk"  # or your SDK path
export ANDROID_NDK_HOME="$ANDROID_HOME/ndk/27.1.12297006"
export JAVA_HOME="C:/Program Files/Android/Android Studio/jbr"
```

---

## What the Build Script Does

1. **Validates prerequisites**: Checks for Rust, Tauri CLI, Android SDK, NDK, and JDK.
2. **Installs NDK if missing**: Uses `sdkmanager` to download NDK r27.
3. **Adds Rust Android targets**: `aarch64-linux-android`, `armv7-linux-androideabi`, `x86_64-linux-android`, `i686-linux-android`.
4. **Writes `.cargo/config.toml`**: Sets NDK linker paths for each Android target. API level 26 (Android 8.0 Oreo) as minimum.
5. **Initializes Tauri Android project**: Runs `cargo tauri android init` which generates `src-tauri/gen/android/` with Gradle project files and Kotlin Activity.
6. **Builds the APK**: Runs `cargo tauri android build` (release) or `cargo tauri android build --debug`.

---

## Output Location

After a successful build, the APK is at:

```
crates/ephemera-client/src-tauri/gen/android/app/build/outputs/apk/
  universal/release/app-universal-release.apk    # All architectures
  arm64-v8a/release/app-arm64-v8a-release.apk   # ARM64 only
```

---

## Installing on a Device

```bash
# Connect a device with USB debugging enabled, then:
adb install crates/ephemera-client/src-tauri/gen/android/app/build/outputs/apk/universal/release/app-universal-release.apk
```

Or use the debug APK for development:

```bash
adb install crates/ephemera-client/src-tauri/gen/android/app/build/outputs/apk/debug/app-debug.apk
```

---

## Android-Specific Features

### Camera / QR Scanner

The Android app uses a WebView-based QR scanner (no native Tauri plugin):

- `navigator.mediaDevices.getUserMedia()` for camera access
- Canvas-based frame analysis with luminance thresholding
- Manual hex-entry fallback if camera is unavailable or decoding fails
- Requires `CAMERA` permission in AndroidManifest.xml

The CSP in `tauri.conf.json` includes `media-src 'self' mediastream:` to
allow getUserMedia in the WebView.

### Identity Import via QR

1. On the source device: Settings > Security > Export Recovery QR
2. On the target device: Settings > Security > Import Identity > Scan QR Code
3. The hex payload is sent to the `identity.import_qr` RPC method
4. A new keystore is created with the imported master secret

### Resource Constraints

The mobile node runs in constrained mode:
- Storage: 200-256 MB cap
- Peer connections: 3-4 max
- No relay duties (light node)
- Bandwidth-limited when on battery

### Permissions

Required in `AndroidManifest.xml` (added by `cargo tauri android init`):

| Permission | Purpose |
|-----------|---------|
| `INTERNET` | P2P networking |
| `ACCESS_NETWORK_STATE` | Connectivity detection |
| `CAMERA` | QR code scanning |
| `VIBRATE` | Haptic feedback |

---

## Troubleshooting

### "NDK not found"

Ensure `ANDROID_NDK_HOME` points to a valid NDK directory containing
`toolchains/llvm/prebuilt/<host>/bin/`. Install via SDK Manager:

```bash
$ANDROID_HOME/cmdline-tools/latest/bin/sdkmanager "ndk;27.1.12297006"
```

### "JDK not found"

The build script checks `JAVA_HOME`, Android Studio's bundled JBR, and
`C:\Program Files\Java\jdk-*`. Set `JAVA_HOME` explicitly:

```bash
export JAVA_HOME="C:/Program Files/Android/Android Studio/jbr"
```

### Cross-compilation linker errors

Verify the Rust targets are installed:

```bash
rustup target list --installed | grep android
```

And that `.cargo/config.toml` has correct linker paths. Re-run the build
script to regenerate it.

### Large APK size

Release builds use LTO and symbol stripping via the workspace
`[profile.release]` settings. The universal APK includes all
architectures; use architecture-specific APKs for smaller size.

### Camera not working in WebView

- Check that the CSP includes `media-src 'self' mediastream:`
- Verify `CAMERA` permission is in AndroidManifest.xml
- Some WebView versions require HTTPS or `--allow-file-access-from-files`
- Fall back to manual hex entry if camera scanning fails

---

## iOS (macOS Only)

iOS builds require macOS with Xcode 15+. The process is similar:

```bash
rustup target add aarch64-apple-ios aarch64-apple-ios-sim
cargo tauri ios init
cargo tauri ios dev      # Simulator
cargo tauri ios build    # Device
```

See the Apple Developer documentation for provisioning and code signing.
