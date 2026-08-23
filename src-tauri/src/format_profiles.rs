//! Allowlisted generic filesystem profiles for a future OS-native formatter.
//!
//! These recommendations are deliberately capacity-based and therefore
//! inferred. They are not camera certification and do not carry arbitrary
//! filesystem, label, partition, or allocation-unit settings across IPC.

use serde::Serialize;

const SD_MAX_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const SDHC_MAX_BYTES: u64 = 32 * 1024 * 1024 * 1024;
const SDXC_MAX_BYTES: u64 = 2 * 1024 * 1024 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FormatFilesystem {
    Fat,
    Fat32,
    Exfat,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FormatProfile {
    pub id: &'static str,
    pub filesystem: FormatFilesystem,
    /// Capacity can suggest an SD family but cannot prove card/controller
    /// characteristics through a generic reader, so this must remain visible.
    pub inferred_from_capacity: bool,
}

/// Returns only the generic profiles the future provider may accept. SDUC is
/// intentionally withheld until an OS/reader/camera combination is certified.
pub fn recommended_profile(total_bytes: Option<u64>) -> Option<FormatProfile> {
    let capacity = total_bytes?;
    let (id, filesystem) = if capacity <= SD_MAX_BYTES {
        ("sd-default", FormatFilesystem::Fat)
    } else if capacity <= SDHC_MAX_BYTES {
        ("sdhc-default", FormatFilesystem::Fat32)
    } else if capacity <= SDXC_MAX_BYTES {
        ("sdxc-default", FormatFilesystem::Exfat)
    } else {
        return None;
    };
    Some(FormatProfile {
        id,
        filesystem,
        inferred_from_capacity: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generic_profiles_cover_sd_sdhc_and_sdxc_capacity_boundaries() {
        assert_eq!(
            recommended_profile(Some(SD_MAX_BYTES)).expect("sd").id,
            "sd-default"
        );
        assert_eq!(
            recommended_profile(Some(SD_MAX_BYTES + 1))
                .expect("sdhc")
                .filesystem,
            FormatFilesystem::Fat32
        );
        assert_eq!(
            recommended_profile(Some(SDHC_MAX_BYTES + 1))
                .expect("sdxc")
                .filesystem,
            FormatFilesystem::Exfat
        );
        assert!(recommended_profile(Some(SDXC_MAX_BYTES + 1)).is_none());
        assert!(recommended_profile(None).is_none());
    }
}
