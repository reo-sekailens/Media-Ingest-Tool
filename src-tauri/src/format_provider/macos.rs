//! macOS destructive-format boundary.
//!
//! The former direct `diskutil eraseVolume` implementation was deliberately
//! removed. It could erase a volume but could not prove the whole-medium
//! binding, obtain an application-owned authorization, or reliably locate a
//! renamed remount. Apple documents Disk Arbitration and Authorization
//! Services/SMAppService as the appropriate boundaries for these operations.
//!
//! A future provider must communicate with a signed, least-privilege helper
//! over authenticated IPC. Until that helper can bind the current
//! `DADisk`/IOMedia object, unmount through Disk Arbitration, format the exact
//! approved medium, and return the new mount location, every format request is
//! refused before it can mutate media.

use super::{
    ExpectedFormatTarget, FormatProviderError, PlatformFormatProvider, ResolvedFormatTarget,
    ValidatedMount,
};
use crate::format_profiles::FormatProfile;

pub(super) struct MacOsAuthorizedHelperProvider;

impl PlatformFormatProvider for MacOsAuthorizedHelperProvider {
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
    use crate::format_profiles::{FormatFilesystem, FormatProfile};
    use std::path::PathBuf;

    #[test]
    fn refuses_direct_formatting_without_the_authorized_helper() {
        let provider = MacOsAuthorizedHelperProvider;
        let expected = ExpectedFormatTarget {
            medium_key: "v1:test".into(),
            connection_generation: 1,
            expected_capacity_bytes: 1024,
            current_mount_root: PathBuf::from("/Volumes/CARD"),
        };
        assert_eq!(
            provider.resolve_exact_target(&expected),
            Err(FormatProviderError::UnsupportedPlatform)
        );
        let profile = FormatProfile {
            id: "test",
            filesystem: FormatFilesystem::Exfat,
            inferred_from_capacity: true,
        };
        let target = ResolvedFormatTarget {
            provider_key: "disk4s1".into(),
            current_mount_root: expected.current_mount_root,
            medium_key: expected.medium_key,
            connection_generation: expected.connection_generation,
            capacity_bytes: expected.expected_capacity_bytes,
        };
        assert_eq!(
            provider.quick_format(&target, &profile),
            Err(FormatProviderError::UnsupportedPlatform)
        );
    }
}
