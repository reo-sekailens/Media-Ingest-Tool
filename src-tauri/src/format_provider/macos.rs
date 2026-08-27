//! macOS provider using the system-owned `diskutil` executable.
//!
//! macOS does not expose a stable `/dev/diskN` name across reinsertions. This
//! adapter receives only a freshly discovered native mount root, resolves it
//! immediately to a current diskutil volume identifier, and never accepts an
//! identifier or command arguments from the webview. All commands have a
//! fixed absolute executable and fixed argument grammar.

use super::{
    ExpectedFormatTarget, FormatProviderError, PlatformFormatProvider, ResolvedFormatTarget,
    ValidatedMount,
};
use crate::format_profiles::{FormatFilesystem, FormatProfile};
use std::path::Path;
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

const DISKUTIL: &str = "/usr/sbin/diskutil";
const FORMAT_TIMEOUT: Duration = Duration::from_secs(45);
const FORMAT_LABEL: &str = "MEDIA_INGEST";

pub(super) struct MacOsDiskutilProvider;

impl PlatformFormatProvider for MacOsDiskutilProvider {
    fn resolve_exact_target(
        &self,
        expected: &ExpectedFormatTarget,
    ) -> Result<ResolvedFormatTarget, FormatProviderError> {
        let info = volume_info(&expected.current_mount_root)?;
        if info.capacity_bytes != expected.expected_capacity_bytes {
            return Err(FormatProviderError::TargetChanged);
        }
        if !info.removable || info.read_only || !is_volume_identifier(&info.identifier) {
            return Err(FormatProviderError::NotRemovable);
        }
        Ok(ResolvedFormatTarget {
            provider_key: info.identifier,
            current_mount_root: expected.current_mount_root.clone(),
            medium_key: expected.medium_key.clone(),
            connection_generation: expected.connection_generation,
            capacity_bytes: info.capacity_bytes,
        })
    }

    fn quick_format(
        &self,
        target: &ResolvedFormatTarget,
        profile: &FormatProfile,
    ) -> Result<(), FormatProviderError> {
        if !is_volume_identifier(&target.provider_key) {
            return Err(FormatProviderError::TargetChanged);
        }
        let info = volume_info(Path::new(&format!("/dev/{}", target.provider_key)))?;
        if !info.removable || info.read_only || info.capacity_bytes != target.capacity_bytes {
            return Err(FormatProviderError::TargetChanged);
        }
        let output = Command::new(DISKUTIL)
            .args([
                "eraseVolume",
                diskutil_format(profile.filesystem),
                FORMAT_LABEL,
            ])
            .arg(format!("/dev/{}", target.provider_key))
            .output()
            .map_err(|_| FormatProviderError::FormatFailed)?;
        if output.status.success() {
            Ok(())
        } else {
            Err(FormatProviderError::FormatFailed)
        }
    }

    fn wait_for_validated_mount(
        &self,
        expected: &ExpectedFormatTarget,
        profile: &FormatProfile,
    ) -> Result<ValidatedMount, FormatProviderError> {
        let deadline = Instant::now() + FORMAT_TIMEOUT;
        loop {
            match volume_info(&expected.current_mount_root) {
                Ok(info)
                    if info.capacity_bytes == expected.expected_capacity_bytes
                        && filesystem_matches(&info.filesystem, profile.filesystem) =>
                {
                    return Ok(ValidatedMount {
                        root: expected.current_mount_root.clone(),
                        filesystem: info.filesystem,
                        capacity_bytes: info.capacity_bytes,
                    });
                }
                Ok(info) if info.capacity_bytes != expected.expected_capacity_bytes => {
                    return Err(FormatProviderError::TargetChanged);
                }
                _ if Instant::now() >= deadline => return Err(FormatProviderError::RemountFailed),
                _ => thread::sleep(Duration::from_millis(500)),
            }
        }
    }
}

struct VolumeInfo {
    identifier: String,
    capacity_bytes: u64,
    filesystem: String,
    removable: bool,
    read_only: bool,
}

fn volume_info(locator: &Path) -> Result<VolumeInfo, FormatProviderError> {
    let output = Command::new(DISKUTIL)
        .args(["info", "-plist"])
        .arg(locator)
        .output()
        .map_err(|_| FormatProviderError::TargetUnavailable)?;
    if !output.status.success() {
        return Err(FormatProviderError::TargetUnavailable);
    }
    let xml =
        std::str::from_utf8(&output.stdout).map_err(|_| FormatProviderError::TargetUnavailable)?;
    Ok(VolumeInfo {
        identifier: plist_string(xml, "DeviceIdentifier")
            .ok_or(FormatProviderError::TargetUnavailable)?,
        capacity_bytes: plist_u64(xml, "TotalSize")
            .ok_or(FormatProviderError::TargetUnavailable)?,
        filesystem: plist_string(xml, "FilesystemType").unwrap_or_default(),
        removable: plist_bool(xml, "RemovableMediaOrExternal").unwrap_or(false),
        read_only: plist_bool(xml, "ReadOnlyVolume").unwrap_or(true),
    })
}

fn diskutil_format(filesystem: FormatFilesystem) -> &'static str {
    match filesystem {
        FormatFilesystem::Fat => "MS-DOS FAT16",
        FormatFilesystem::Fat32 => "MS-DOS FAT32",
        FormatFilesystem::Exfat => "ExFAT",
    }
}

fn filesystem_matches(observed: &str, expected: FormatFilesystem) -> bool {
    match expected {
        FormatFilesystem::Fat => observed.eq_ignore_ascii_case("msdos"),
        FormatFilesystem::Fat32 => observed.eq_ignore_ascii_case("msdos"),
        FormatFilesystem::Exfat => observed.eq_ignore_ascii_case("exfat"),
    }
}

fn is_volume_identifier(value: &str) -> bool {
    value.starts_with("disk")
        && value[4..].bytes().any(|byte| byte == b's')
        && value.bytes().all(|byte| byte.is_ascii_alphanumeric())
}

fn plist_string(xml: &str, key: &str) -> Option<String> {
    let after_key = xml.split_once(&format!("<key>{key}</key>"))?.1;
    let value = after_key
        .split_once("<string>")?
        .1
        .split_once("</string>")?
        .0;
    Some(value.into())
}

fn plist_u64(xml: &str, key: &str) -> Option<u64> {
    let after_key = xml.split_once(&format!("<key>{key}</key>"))?.1;
    after_key
        .split_once("<integer>")?
        .1
        .split_once("</integer>")?
        .0
        .trim()
        .parse()
        .ok()
}

fn plist_bool(xml: &str, key: &str) -> Option<bool> {
    let after_key = xml.split_once(&format!("<key>{key}</key>"))?.1.trim_start();
    if after_key.starts_with("<true/>") {
        Some(true)
    } else if after_key.starts_with("<false/>") {
        Some(false)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diskutil_plist_parser_is_limited_to_expected_typed_fields() {
        let xml = "<key>DeviceIdentifier</key><string>disk4s1</string><key>TotalSize</key><integer>123</integer><key>RemovableMediaOrExternal</key><true/><key>ReadOnlyVolume</key><false/>";
        assert_eq!(
            plist_string(xml, "DeviceIdentifier"),
            Some("disk4s1".into())
        );
        assert_eq!(plist_u64(xml, "TotalSize"), Some(123));
        assert_eq!(plist_bool(xml, "RemovableMediaOrExternal"), Some(true));
        assert_eq!(plist_bool(xml, "ReadOnlyVolume"), Some(false));
        assert!(is_volume_identifier("disk4s1"));
        assert!(!is_volume_identifier("disk4"));
    }
}
