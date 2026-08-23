//! Linux UDisks2 D-Bus format provider.
//!
//! `/dev/sdX` is only resolved from the current native mount immediately
//! before the operation. The provider then calls the documented UDisks2
//! `org.freedesktop.UDisks2.Block.Format` D-Bus method directly; it never
//! invokes `udisksctl`, a shell, or an arbitrary formatter command.

use super::{
    ExpectedFormatTarget, FormatProviderError, PlatformFormatProvider, ResolvedFormatTarget,
    ValidatedMount,
};
use crate::format_profiles::{FormatFilesystem, FormatProfile};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};
use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::Value;

const UDISKS_SERVICE: &str = "org.freedesktop.UDisks2";
const UDISKS_ROOT: &str = "/org/freedesktop/UDisks2";
const BLOCK_INTERFACE: &str = "org.freedesktop.UDisks2.Block";
const FORMAT_TIMEOUT: Duration = Duration::from_secs(45);

pub(super) struct LinuxUdisksProvider;

impl PlatformFormatProvider for LinuxUdisksProvider {
    fn resolve_exact_target(
        &self,
        expected: &ExpectedFormatTarget,
    ) -> Result<ResolvedFormatTarget, FormatProviderError> {
        let block = block_for_mount(&expected.current_mount_root)?;
        let observed = block_info(&block)?;
        if observed.capacity_bytes != expected.expected_capacity_bytes {
            return Err(FormatProviderError::TargetChanged);
        }
        if observed.read_only {
            return Err(FormatProviderError::WriteProtected);
        }
        if observed.hint_system || !sysfs_removable(&block) {
            return Err(FormatProviderError::NotRemovable);
        }
        Ok(ResolvedFormatTarget {
            provider_key: block,
            medium_key: expected.medium_key.clone(),
            connection_generation: expected.connection_generation,
            capacity_bytes: observed.capacity_bytes,
        })
    }

    fn quick_format(
        &self,
        target: &ResolvedFormatTarget,
        profile: &FormatProfile,
    ) -> Result<(), FormatProviderError> {
        if !safe_block_name(&target.provider_key) {
            return Err(FormatProviderError::TargetChanged);
        }
        let observed = block_info(&target.provider_key)?;
        if observed.capacity_bytes != target.capacity_bytes || observed.read_only {
            return Err(FormatProviderError::TargetChanged);
        }
        if observed.hint_system || !sysfs_removable(&target.provider_key) {
            return Err(FormatProviderError::NotRemovable);
        }
        let connection = Connection::system().map_err(|_| FormatProviderError::FormatFailed)?;
        let object_path = format!("{UDISKS_ROOT}/block_devices/{}", target.provider_key);
        let proxy = Proxy::new(&connection, UDISKS_SERVICE, object_path, BLOCK_INTERFACE)
            .map_err(|_| FormatProviderError::FormatFailed)?;
        let options = HashMap::from([("erase", Value::from("quick"))]);
        let _: () = proxy
            .call("Format", &(udisks_filesystem(profile.filesystem), options))
            .map_err(|_| FormatProviderError::FormatFailed)?;
        Ok(())
    }

    fn wait_for_validated_mount(
        &self,
        expected: &ExpectedFormatTarget,
        profile: &FormatProfile,
    ) -> Result<ValidatedMount, FormatProviderError> {
        let deadline = Instant::now() + FORMAT_TIMEOUT;
        loop {
            match block_for_mount(&expected.current_mount_root).and_then(|block| {
                let info = block_info(&block)?;
                if info.capacity_bytes != expected.expected_capacity_bytes {
                    return Err(FormatProviderError::TargetChanged);
                }
                if filesystem_matches(&info.filesystem, profile.filesystem) {
                    Ok(info)
                } else {
                    Err(FormatProviderError::ValidationFailed)
                }
            }) {
                Ok(info) => {
                    return Ok(ValidatedMount {
                        root: expected.current_mount_root.clone(),
                        filesystem: info.filesystem,
                        capacity_bytes: info.capacity_bytes,
                    });
                }
                Err(FormatProviderError::TargetChanged) => {
                    return Err(FormatProviderError::TargetChanged)
                }
                Err(_) if Instant::now() >= deadline => {
                    return Err(FormatProviderError::RemountFailed)
                }
                Err(_) => thread::sleep(Duration::from_millis(500)),
            }
        }
    }
}

struct BlockInfo {
    capacity_bytes: u64,
    filesystem: String,
    read_only: bool,
    hint_system: bool,
}

fn block_info(block: &str) -> Result<BlockInfo, FormatProviderError> {
    if !safe_block_name(block) {
        return Err(FormatProviderError::TargetUnavailable);
    }
    let connection = Connection::system().map_err(|_| FormatProviderError::TargetUnavailable)?;
    let object_path = format!("{UDISKS_ROOT}/block_devices/{block}");
    let proxy = Proxy::new(&connection, UDISKS_SERVICE, object_path, BLOCK_INTERFACE)
        .map_err(|_| FormatProviderError::TargetUnavailable)?;
    Ok(BlockInfo {
        capacity_bytes: proxy
            .get_property("Size")
            .map_err(|_| FormatProviderError::TargetUnavailable)?,
        filesystem: proxy
            .get_property("IdType")
            .map_err(|_| FormatProviderError::TargetUnavailable)?,
        read_only: proxy
            .get_property("ReadOnly")
            .map_err(|_| FormatProviderError::TargetUnavailable)?,
        hint_system: proxy
            .get_property("HintSystem")
            .map_err(|_| FormatProviderError::TargetUnavailable)?,
    })
}

fn block_for_mount(root: &Path) -> Result<String, FormatProviderError> {
    let target = root
        .canonicalize()
        .map_err(|_| FormatProviderError::TargetUnavailable)?;
    let mountinfo = fs::read_to_string("/proc/self/mountinfo")
        .map_err(|_| FormatProviderError::TargetUnavailable)?;
    mountinfo
        .lines()
        .filter_map(mountinfo_entry)
        .find(|(mount, _)| *mount == target)
        .and_then(|(_, source)| source.strip_prefix("/dev/"))
        .filter(|block| safe_block_name(block))
        .map(str::to_owned)
        .ok_or(FormatProviderError::TargetUnavailable)
}

fn mountinfo_entry(line: &str) -> Option<(PathBuf, String)> {
    let (before_separator, after_separator) = line.split_once(" - ")?;
    let mount = before_separator.split_whitespace().nth(4)?;
    let source = after_separator.split_whitespace().nth(1)?;
    Some((PathBuf::from(unescape_mountinfo(mount)?), source.into()))
}

fn unescape_mountinfo(value: &str) -> Option<String> {
    let mut result = String::new();
    let mut characters = value.chars();
    while let Some(character) = characters.next() {
        if character != '\\' {
            result.push(character);
            continue;
        }
        let octal = [characters.next()?, characters.next()?, characters.next()?];
        let number = octal.into_iter().collect::<String>();
        let byte = u8::from_str_radix(&number, 8).ok()?;
        result.push(char::from(byte));
    }
    Some(result)
}

fn sysfs_removable(block: &str) -> bool {
    let canonical = match Path::new("/sys/class/block").join(block).canonicalize() {
        Ok(path) => path,
        Err(_) => return false,
    };
    canonical.ancestors().any(|path| {
        fs::read_to_string(path.join("removable"))
            .ok()
            .is_some_and(|value| value.trim() == "1")
    })
}

fn safe_block_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

fn udisks_filesystem(filesystem: FormatFilesystem) -> &'static str {
    match filesystem {
        FormatFilesystem::Fat | FormatFilesystem::Fat32 => "vfat",
        FormatFilesystem::Exfat => "exfat",
    }
}

fn filesystem_matches(observed: &str, expected: FormatFilesystem) -> bool {
    match expected {
        FormatFilesystem::Fat | FormatFilesystem::Fat32 => observed.eq_ignore_ascii_case("vfat"),
        FormatFilesystem::Exfat => observed.eq_ignore_ascii_case("exfat"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mountinfo_parser_keeps_a_dev_source_and_decodes_mount_escapes() {
        let (mount, source) =
            mountinfo_entry("29 23 8:17 / /media/card\\040one rw,nosuid - vfat /dev/sdb1 rw")
                .expect("entry");
        assert_eq!(mount, PathBuf::from("/media/card one"));
        assert_eq!(source, "/dev/sdb1");
        assert!(safe_block_name("mmcblk0p1"));
        assert!(!safe_block_name("../sdb1"));
    }
}
