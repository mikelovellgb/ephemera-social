//! Platform abstraction layer for Ephemera.
//!
//! Defines a trait for platform-specific capabilities (camera, share sheet,
//! biometrics, etc.) and provides a desktop stub implementation. Mobile
//! platforms will supply their own implementations via Tauri plugins or
//! direct FFI bindings.

use std::path::PathBuf;

/// Platform-specific capabilities.
///
/// Each target platform (Desktop, Android, iOS) provides an implementation
/// of this trait. The desktop implementation returns sensible no-ops for
/// mobile-only features (camera, haptics, biometrics).
pub trait PlatformApi: Send + Sync {
    /// Persistent data directory for the application.
    fn data_directory(&self) -> PathBuf;

    /// Capture an image from the device camera.
    ///
    /// Returns `None` on platforms without a camera or if the user cancels.
    fn camera_capture(&self) -> Option<Vec<u8>>;

    /// Open the system photo/gallery picker.
    ///
    /// Returns `None` on platforms without a gallery or if the user cancels.
    fn photo_picker(&self) -> Option<Vec<u8>>;

    /// Invoke the OS share sheet with the given text and URL.
    fn share_intent(&self, text: &str, url: &str);

    /// Trigger a short haptic feedback pulse.
    fn haptic_feedback(&self);

    /// Attempt biometric authentication (FaceID, fingerprint, Windows Hello).
    ///
    /// Returns `true` if authentication succeeded.
    fn biometric_auth(&self) -> bool;

    /// Whether the device currently has network connectivity.
    fn is_network_available(&self) -> bool;

    /// Estimated battery level as a percentage (0-100), if available.
    fn battery_level(&self) -> Option<u8>;
}

// ---------------------------------------------------------------------------
// Desktop implementation
// ---------------------------------------------------------------------------

/// Desktop stub implementing [`PlatformApi`].
///
/// Camera, haptics, and biometrics are no-ops on desktop. The data
/// directory delegates to [`ephemera_config::NodeConfig::default_data_dir`].
pub struct DesktopPlatform {
    data_dir: PathBuf,
}

impl DesktopPlatform {
    /// Create a desktop platform with the given data directory.
    #[must_use]
    pub fn new(data_dir: PathBuf) -> Self {
        Self { data_dir }
    }
}

impl PlatformApi for DesktopPlatform {
    fn data_directory(&self) -> PathBuf {
        self.data_dir.clone()
    }

    fn camera_capture(&self) -> Option<Vec<u8>> {
        // Desktop: no built-in camera API.
        None
    }

    fn photo_picker(&self) -> Option<Vec<u8>> {
        // Desktop: could use native file dialogs in the future.
        None
    }

    fn share_intent(&self, text: &str, url: &str) {
        // Desktop: copy to clipboard as a fallback.
        tracing::info!(text, url, "share_intent: desktop has no native share sheet");
    }

    fn haptic_feedback(&self) {
        // No-op on desktop.
    }

    fn biometric_auth(&self) -> bool {
        // Desktop: no biometric hardware by default.
        // Could integrate Windows Hello / TouchID in the future.
        false
    }

    fn is_network_available(&self) -> bool {
        // Assume available on desktop.
        true
    }

    fn battery_level(&self) -> Option<u8> {
        // Desktop: battery info not readily available without platform crate.
        None
    }
}

// ---------------------------------------------------------------------------
// Android stub
// ---------------------------------------------------------------------------

/// Android implementation of [`PlatformApi`].
///
/// In a real Tauri mobile build these methods would call into JNI or
/// Tauri plugin APIs. For now they are stubs that log and return defaults.
pub struct AndroidPlatform {
    data_dir: PathBuf,
}

impl AndroidPlatform {
    /// Create an Android platform with the given app-internal data directory.
    #[must_use]
    pub fn new(data_dir: PathBuf) -> Self {
        Self { data_dir }
    }
}

impl PlatformApi for AndroidPlatform {
    fn data_directory(&self) -> PathBuf {
        self.data_dir.clone()
    }

    fn camera_capture(&self) -> Option<Vec<u8>> {
        tracing::debug!("camera_capture: stub on Android");
        None
    }

    fn photo_picker(&self) -> Option<Vec<u8>> {
        tracing::debug!("photo_picker: stub on Android");
        None
    }

    fn share_intent(&self, text: &str, url: &str) {
        tracing::info!(text, url, "share_intent: would invoke Android Intent.ACTION_SEND");
    }

    fn haptic_feedback(&self) {
        tracing::debug!("haptic_feedback: stub on Android");
    }

    fn biometric_auth(&self) -> bool {
        tracing::debug!("biometric_auth: stub on Android");
        false
    }

    fn is_network_available(&self) -> bool {
        true
    }

    fn battery_level(&self) -> Option<u8> {
        None
    }
}

// ---------------------------------------------------------------------------
// iOS stub
// ---------------------------------------------------------------------------

/// iOS implementation of [`PlatformApi`].
///
/// In a real Tauri mobile build these methods would call into Swift/ObjC
/// bridges or Tauri plugin APIs. For now they are stubs.
pub struct IosPlatform {
    data_dir: PathBuf,
}

impl IosPlatform {
    /// Create an iOS platform with the given app container data directory.
    #[must_use]
    pub fn new(data_dir: PathBuf) -> Self {
        Self { data_dir }
    }
}

impl PlatformApi for IosPlatform {
    fn data_directory(&self) -> PathBuf {
        self.data_dir.clone()
    }

    fn camera_capture(&self) -> Option<Vec<u8>> {
        tracing::debug!("camera_capture: stub on iOS");
        None
    }

    fn photo_picker(&self) -> Option<Vec<u8>> {
        tracing::debug!("photo_picker: stub on iOS");
        None
    }

    fn share_intent(&self, text: &str, url: &str) {
        tracing::info!(text, url, "share_intent: would invoke UIActivityViewController");
    }

    fn haptic_feedback(&self) {
        tracing::debug!("haptic_feedback: stub on iOS");
    }

    fn biometric_auth(&self) -> bool {
        tracing::debug!("biometric_auth: stub on iOS (would use LAContext)");
        false
    }

    fn is_network_available(&self) -> bool {
        true
    }

    fn battery_level(&self) -> Option<u8> {
        None
    }
}

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

/// Create the appropriate [`PlatformApi`] implementation for the current
/// target platform.
///
/// Uses compile-time `#[cfg]` to pick the right implementation.
pub fn create_platform(data_dir: PathBuf) -> Box<dyn PlatformApi> {
    #[cfg(target_os = "android")]
    {
        Box::new(AndroidPlatform::new(data_dir))
    }
    #[cfg(target_os = "ios")]
    {
        Box::new(IosPlatform::new(data_dir))
    }
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
        Box::new(DesktopPlatform::new(data_dir))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_desktop_platform() {
        let dir = PathBuf::from("/tmp/test-ephemera");
        let platform = DesktopPlatform::new(dir.clone());
        assert_eq!(platform.data_directory(), dir);
        assert!(platform.camera_capture().is_none());
        assert!(platform.photo_picker().is_none());
        assert!(!platform.biometric_auth());
        assert!(platform.is_network_available());
        assert_eq!(platform.battery_level(), None);
    }

    #[test]
    fn test_create_platform_returns_desktop() {
        let p = create_platform(PathBuf::from("/tmp"));
        assert!(p.is_network_available());
        assert!(p.camera_capture().is_none());
    }

    #[test]
    fn test_android_platform_stubs() {
        let platform = AndroidPlatform::new(PathBuf::from("/data/ephemera"));
        assert_eq!(platform.data_directory(), PathBuf::from("/data/ephemera"));
        assert!(platform.camera_capture().is_none());
        assert!(!platform.biometric_auth());
    }

    #[test]
    fn test_ios_platform_stubs() {
        let platform = IosPlatform::new(PathBuf::from("/var/mobile/ephemera"));
        assert_eq!(platform.data_directory(), PathBuf::from("/var/mobile/ephemera"));
        assert!(platform.photo_picker().is_none());
        assert!(!platform.biometric_auth());
    }
}
