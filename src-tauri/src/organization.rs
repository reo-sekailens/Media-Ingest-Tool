//! Deterministic camera identity and capture-time destination projection.

use blake3::Hasher;
use chrono::{DateTime, FixedOffset, Timelike};
use std::path::{Component, Path, PathBuf};
use unicode_casefold::UnicodeCaseFold;
use unicode_normalization::UnicodeNormalization;

const MAX_PORTABLE_COMPONENT_BYTES: usize = 120;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CameraConfidence {
    EmbeddedSerial,
    UserMapped,
    RunScopedUnknown,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CameraIdentity {
    pub key: String,
    pub display_label: String,
    pub confidence: CameraConfidence,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SortMode {
    OriginalTree,
    CameraDay,
    CameraInterval { minutes: u16 },
}

pub fn camera_identity(
    make: &str,
    model: &str,
    body_serial: Option<&str>,
    run_id: &str,
) -> CameraIdentity {
    let display_label = sanitize_destination_component(&format!("{make} {model}"));
    match body_serial.filter(|value| !value.trim().is_empty()) {
        Some(serial) => CameraIdentity {
            key: keyed_digest(&["camera-id-v1", make, serial]),
            display_label,
            confidence: CameraConfidence::EmbeddedSerial,
        },
        None => CameraIdentity {
            key: keyed_digest(&["unknown-camera-v1", run_id]),
            display_label,
            confidence: CameraConfidence::RunScopedUnknown,
        },
    }
}

pub fn destination_relative_path(
    original_relative_path: &str,
    camera: &CameraIdentity,
    capture: DateTime<FixedOffset>,
    mode: SortMode,
) -> Result<PathBuf, &'static str> {
    let original = portable_relative_path(original_relative_path)?;
    match mode {
        SortMode::OriginalTree => Ok(original),
        SortMode::CameraDay => Ok(PathBuf::from(camera_folder(camera))
            .join(capture.format("%Y-%m-%d").to_string())
            .join(original.file_name().ok_or("missing filename")?)),
        SortMode::CameraInterval { minutes } => {
            if !(1..=1_440).contains(&minutes) {
                return Err("interval must be from 1 to 1,440 minutes");
            }
            // Each local capture day remains its own namespace. This makes an
            // arbitrary interval deterministic even when it does not divide
            // a day: its final bucket simply ends at the next local midnight.
            let minute = capture.hour() as u16 * 60 + capture.minute() as u16;
            let bucket = minute / minutes * minutes;
            let offset = capture.format("%:z").to_string().replace(':', "");
            Ok(PathBuf::from(camera_folder(camera))
                .join(capture.format("%Y-%m-%d").to_string())
                .join(format!("{:02}-{:02}_{offset}", bucket / 60, bucket % 60))
                .join(original.file_name().ok_or("missing filename")?))
        }
    }
}

fn camera_folder(camera: &CameraIdentity) -> String {
    format!("{}__{}", camera.display_label, &camera.key[..10])
}

fn keyed_digest(parts: &[&str]) -> String {
    let mut hasher = Hasher::new();
    for part in parts {
        hasher.update(&(part.len() as u32).to_le_bytes());
        hasher.update(part.trim().as_bytes());
    }
    hasher.finalize().to_hex().to_string()
}

pub fn sanitize_destination_component(value: &str) -> String {
    let mut result = value
        .chars()
        .map(|character| {
            if "<>:\"/\\|?*".contains(character) || character.is_control() {
                '_'
            } else {
                character
            }
        })
        .collect::<String>()
        .trim_end_matches(|character: char| character == '.' || character.is_whitespace())
        .to_string();
    if result.is_empty() {
        result = "unnamed".into();
    }
    if is_reserved_windows_component(&result) {
        result.insert(0, '_');
    }
    shorten_component(&result)
}

pub fn is_portable_destination_relative_path(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && !path.is_absolute()
        && path.components().all(|part| match part {
            Component::Normal(component) => component
                .to_str()
                .is_some_and(is_portable_destination_component),
            _ => false,
        })
}

/// Conservative cross-filesystem collision key. macOS may normalize file
/// names and Windows commonly ignores case, so plans compare the normalized
/// default-case-folded projection even when the current destination happens
/// to be case-sensitive.
pub fn portable_destination_key(path: &Path) -> Option<String> {
    if !is_portable_destination_relative_path(path) {
        return None;
    }
    let display = path.to_str()?.replace('\\', "/");
    let normalized = display.nfc().collect::<String>();
    Some(normalized.chars().case_fold().nfc().collect())
}

fn portable_relative_path(value: &str) -> Result<PathBuf, &'static str> {
    let path = PathBuf::from(value);
    if path.is_absolute()
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err("unsafe source path");
    }
    let components = path
        .components()
        .map(|part| match part {
            Component::Normal(component) => component
                .to_str()
                .map(sanitize_destination_component)
                .ok_or("source path cannot be projected portably"),
            _ => Err("unsafe source path"),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let projected = components
        .iter()
        .fold(PathBuf::new(), |path, component| path.join(component));
    if !is_portable_destination_relative_path(&projected) {
        Err("unsafe destination projection")
    } else {
        Ok(projected)
    }
}

fn is_portable_destination_component(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_PORTABLE_COMPONENT_BYTES
        && !value.ends_with('.')
        && !value.chars().last().is_some_and(char::is_whitespace)
        && !is_reserved_windows_component(value)
        && !value
            .chars()
            .any(|character| "<>:\"/\\|?*".contains(character) || character.is_control())
}

fn is_reserved_windows_component(value: &str) -> bool {
    let stem = value
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end_matches(char::is_whitespace);
    let upper = stem.to_ascii_uppercase();
    matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CLOCK$")
        || (upper.len() == 4
            && (upper.starts_with("COM") || upper.starts_with("LPT"))
            && matches!(upper.as_bytes()[3], b'1'..=b'9'))
}

fn shorten_component(value: &str) -> String {
    if value.len() <= MAX_PORTABLE_COMPONENT_BYTES {
        return value.into();
    }
    let fingerprint = blake3::hash(value.as_bytes()).to_hex().to_string();
    let suffix = format!("__{}", &fingerprint[..10]);
    let max_prefix_bytes = MAX_PORTABLE_COMPONENT_BYTES - suffix.len();
    let prefix = value
        .chars()
        .scan(0_usize, |used, character| {
            let length = character.len_utf8();
            if *used + length > max_prefix_bytes {
                None
            } else {
                *used += length;
                Some(character)
            }
        })
        .collect::<String>();
    format!("{prefix}{suffix}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    #[test]
    fn same_model_distinct_serials_have_distinct_camera_folders() {
        let first = camera_identity("Sony", "FX3", Some("A-001"), "run");
        let second = camera_identity("Sony", "FX3", Some("A-002"), "run");
        assert_ne!(first.key, second.key);
    }

    #[test]
    fn interval_bucket_includes_offset_for_ambiguous_local_hours() {
        let capture = Utc
            .with_ymd_and_hms(2026, 11, 1, 9, 47, 0)
            .unwrap()
            .with_timezone(&FixedOffset::west_opt(8 * 3600).unwrap());
        let camera = camera_identity("Sony", "FX3", Some("A-001"), "run");
        let path = destination_relative_path(
            "PRIVATE/CLIP.MOV",
            &camera,
            capture,
            SortMode::CameraInterval { minutes: 60 },
        )
        .expect("path");
        assert!(path.to_string_lossy().contains("01-00_-0800"));
    }

    #[test]
    fn arbitrary_interval_is_anchored_at_local_midnight() {
        let capture = Utc
            .with_ymd_and_hms(2026, 8, 23, 23, 47, 0)
            .unwrap()
            .with_timezone(&FixedOffset::east_opt(0).unwrap());
        let camera = camera_identity("Sony", "FX3", Some("A-001"), "run");
        let path = destination_relative_path(
            "PRIVATE/CLIP.MOV",
            &camera,
            capture,
            SortMode::CameraInterval { minutes: 37 },
        )
        .expect("path");
        assert!(path.to_string_lossy().contains("2026-08-23\\23-26_+0000"));
    }

    #[test]
    fn interval_rejects_only_values_outside_the_supported_range() {
        let capture = Utc
            .with_ymd_and_hms(2026, 8, 23, 0, 0, 0)
            .unwrap()
            .with_timezone(&FixedOffset::east_opt(0).unwrap());
        let camera = camera_identity("Sony", "FX3", Some("A-001"), "run");
        for minutes in [0, 1_441] {
            assert_eq!(
                destination_relative_path(
                    "PRIVATE/CLIP.MOV",
                    &camera,
                    capture,
                    SortMode::CameraInterval { minutes },
                ),
                Err("interval must be from 1 to 1,440 minutes")
            );
        }
    }

    #[test]
    fn path_projection_makes_windows_reserved_and_trailing_names_portable() {
        let camera = camera_identity("Sony", "FX3", Some("A-001"), "run");
        let path = destination_relative_path(
            "DCIM/CON./AUX .MOV",
            &camera,
            Utc.with_ymd_and_hms(2026, 8, 23, 1, 2, 3)
                .unwrap()
                .with_timezone(&FixedOffset::east_opt(0).unwrap()),
            SortMode::OriginalTree,
        )
        .expect("path");
        assert_eq!(path, PathBuf::from("DCIM/_CON/_AUX .MOV"));
        assert!(is_portable_destination_relative_path(&path));
    }

    #[test]
    fn long_components_are_deterministically_shortened() {
        let component = sanitize_destination_component(&"a".repeat(200));
        assert!(component.len() <= MAX_PORTABLE_COMPONENT_BYTES);
        assert!(component.contains("__"));
        assert!(is_portable_destination_relative_path(&PathBuf::from(
            component
        )));
    }

    #[test]
    fn portable_key_matches_case_and_canonical_unicode_variants() {
        assert_eq!(
            portable_destination_key(Path::new("Camera/CLIP.MOV")),
            portable_destination_key(Path::new("camera/clip.mov"))
        );
        assert_eq!(
            portable_destination_key(Path::new("Camera/café.mov")),
            portable_destination_key(Path::new("camera/cafe\u{301}.mov"))
        );
    }
}
