//! Platform-owned destructive format boundary.
//!
//! No caller may pass a mount path, drive letter, disk number, or arbitrary
//! formatter arguments through IPC. A provider resolves the expected card
//! itself immediately before formatting and again after the operation.

use crate::format_profiles::FormatProfile;
use std::path::PathBuf;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExpectedFormatTarget {
    pub medium_key: String,
    pub connection_generation: u64,
    pub expected_capacity_bytes: u64,
    /// Native-only current mount locator, never accepted from IPC.
    pub current_mount_root: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedFormatTarget {
    /// Native-only opaque binding owned by a platform provider.
    pub provider_key: String,
    /// Native-only mount locator carried from the immediately preceding exact
    /// platform resolution. It is revalidated before any formatter runs.
    pub current_mount_root: PathBuf,
    pub medium_key: String,
    pub connection_generation: u64,
    pub capacity_bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidatedMount {
    pub root: PathBuf,
    pub filesystem: String,
    pub capacity_bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FormatProviderError {
    UnsupportedPlatform,
    TargetUnavailable,
    TargetChanged,
    TargetReopenFailed,
    TargetCapacityMismatch,
    NotRemovable,
    WriteProtected,
    Busy,
    FormatInputFailed,
    FormatOutputMissing,
    FormatResultUnreadable,
    FormatFailed,
    FormatFailedWithCode(u64),
    RemountFailed,
    ValidationFailed,
}

pub trait PlatformFormatProvider: Send + Sync {
    fn resolve_exact_target(
        &self,
        expected: &ExpectedFormatTarget,
    ) -> Result<ResolvedFormatTarget, FormatProviderError>;
    fn quick_format(
        &self,
        target: &ResolvedFormatTarget,
        profile: &FormatProfile,
    ) -> Result<(), FormatProviderError>;
    fn wait_for_validated_mount(
        &self,
        expected: &ExpectedFormatTarget,
        profile: &FormatProfile,
    ) -> Result<ValidatedMount, FormatProviderError>;
}

/// Concrete platform providers must use their native storage API and repeat
/// target resolution after elevation. Until one is installed, fail closed.
#[cfg(windows)]
pub fn current_platform_provider() -> Box<dyn PlatformFormatProvider> {
    Box::new(windows::WindowsStorageProvider)
}

#[cfg(target_os = "macos")]
pub fn current_platform_provider() -> Box<dyn PlatformFormatProvider> {
    Box::new(macos::MacOsAuthorizedHelperProvider)
}

#[cfg(target_os = "linux")]
pub fn current_platform_provider() -> Box<dyn PlatformFormatProvider> {
    Box::new(linux::LinuxUdisksProvider)
}

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
pub fn current_platform_provider() -> Box<dyn PlatformFormatProvider> {
    Box::new(UnsupportedProvider)
}

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
struct UnsupportedProvider;

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
impl PlatformFormatProvider for UnsupportedProvider {
    fn resolve_exact_target(
        &self,
        _expected: &ExpectedFormatTarget,
    ) -> Result<ResolvedFormatTarget, FormatProviderError> {
        Err(FormatProviderError::UnsupportedPlatform)
    }
    fn quick_format(
        &self,
        _target: &ResolvedFormatTarget,
        _profile: &FormatProfile,
    ) -> Result<(), FormatProviderError> {
        Err(FormatProviderError::UnsupportedPlatform)
    }
    fn wait_for_validated_mount(
        &self,
        _expected: &ExpectedFormatTarget,
        _profile: &FormatProfile,
    ) -> Result<ValidatedMount, FormatProviderError> {
        Err(FormatProviderError::UnsupportedPlatform)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_platform_can_fall_back_to_a_path_based_formatter() {
        let expected = ExpectedFormatTarget {
            medium_key: "v1:card".into(),
            connection_generation: 1,
            expected_capacity_bytes: 64,
            current_mount_root: PathBuf::from("F:/"),
        };
        assert!(current_platform_provider()
            .resolve_exact_target(&expected)
            .is_err());
    }
}
