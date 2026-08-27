//! Deterministic camera identity and capture-time destination projection.

use blake3::Hasher;
use chrono::{DateTime, FixedOffset, Timelike};
use std::path::{Component, Path, PathBuf};
use unicode_casefold::UnicodeCaseFold;
use unicode_normalization::UnicodeNormalization;

const MAX_PORTABLE_COMPONENT_BYTES: usize = 120;
const MAX_CUSTOM_DIRECTORY_FIELDS: usize = 8;

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

/// An operator-selected field which contributes one deterministic directory
/// level before the camera/time layout. For example, `Photographer` + `Ari`
/// becomes `Photographer/Ari`.
#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct CustomDirectoryField {
    label: String,
    value: String,
}

impl CustomDirectoryField {
    pub fn new(label: impl Into<String>, value: impl Into<String>) -> Result<Self, &'static str> {
        let label = label.into();
        let value = value.into();
        validate_custom_directory_component(&label, "custom field label")?;
        validate_custom_directory_component(&value, "custom field value")?;
        Ok(Self { label, value })
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub fn value(&self) -> &str {
        &self.value
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SortMode {
    OriginalTree,
    CameraDay,
    CameraInterval { minutes: u16 },
}

/// One draggable destination-directory segment. The filename is intentionally
/// not represented here: it is always emitted as the final path component.
#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DestinationDepthSegment {
    CustomField { index: u8 },
    CameraModel,
    CaptureDay,
    CaptureInterval,
    OriginalTree,
}

pub fn camera_identity(
    make: &str,
    model: &str,
    body_serial: Option<&str>,
    run_id: &str,
) -> CameraIdentity {
    // The model is the operator-facing camera directory. The short identity
    // suffix remains mandatory, so matching models never merge bodies.
    let display_label = camera_display_label(make, model);
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

pub fn custom_directory_prefix(fields: &[CustomDirectoryField]) -> Result<PathBuf, &'static str> {
    if fields.len() > MAX_CUSTOM_DIRECTORY_FIELDS {
        return Err("at most eight custom directory fields are supported");
    }
    let mut prefix = PathBuf::new();
    for field in fields {
        // The type's constructor validates these, but revalidate here to keep
        // this boundary safe if construction changes in a future persistence
        // adapter.
        validate_custom_directory_component(field.label(), "custom field label")?;
        validate_custom_directory_component(field.value(), "custom field value")?;
        prefix.push(sanitize_destination_component(field.label()));
        prefix.push(sanitize_destination_component(field.value()));
    }
    Ok(prefix)
}

/// Resolves an optional operator order into the exact complete set of
/// directory segments permitted by the selected organization mode. A supplied
/// order must be a permutation of that set, so a drag-and-drop client cannot
/// accidentally omit, repeat, or inject a destination level.
pub fn canonical_destination_depth_order(
    mode: &SortMode,
    custom_field_count: usize,
    requested_order: Option<&[DestinationDepthSegment]>,
) -> Result<Vec<DestinationDepthSegment>, &'static str> {
    if custom_field_count > MAX_CUSTOM_DIRECTORY_FIELDS {
        return Err("at most eight custom directory fields are supported");
    }

    let mut expected = (0..custom_field_count)
        .map(|index| DestinationDepthSegment::CustomField { index: index as u8 })
        .collect::<Vec<_>>();
    match mode {
        SortMode::OriginalTree => expected.push(DestinationDepthSegment::OriginalTree),
        SortMode::CameraDay => expected.extend([
            DestinationDepthSegment::CameraModel,
            DestinationDepthSegment::CaptureDay,
        ]),
        SortMode::CameraInterval { minutes } => {
            validate_interval_minutes(*minutes)?;
            expected.extend([
                DestinationDepthSegment::CameraModel,
                DestinationDepthSegment::CaptureDay,
                DestinationDepthSegment::CaptureInterval,
            ]);
        }
    }

    let Some(requested) = requested_order else {
        return Ok(expected);
    };
    for segment in requested {
        if let DestinationDepthSegment::CustomField { index } = segment {
            if usize::from(*index) >= custom_field_count {
                return Err("destination depth order references an unknown custom field");
            }
        }
        if !expected.contains(segment) {
            return Err("destination depth segment is not supported by this sort mode");
        }
        if requested
            .iter()
            .filter(|candidate| *candidate == segment)
            .count()
            > 1
        {
            return Err("destination depth order contains a duplicate segment");
        }
    }
    if requested.len() != expected.len()
        || expected.iter().any(|segment| !requested.contains(segment))
    {
        return Err("destination depth order is missing a required segment");
    }
    Ok(requested.to_vec())
}

pub fn destination_relative_path(
    original_relative_path: &str,
    camera: &CameraIdentity,
    capture: DateTime<FixedOffset>,
    mode: SortMode,
) -> Result<PathBuf, &'static str> {
    destination_relative_path_with_order(original_relative_path, camera, capture, mode, &[], None)
}

/// Projects a source file into a portable destination relative path using the
/// validated destination-depth order. The source filename always remains
/// final, even when `OriginalTree` is reordered among other directory levels.
pub fn destination_relative_path_with_order(
    original_relative_path: &str,
    camera: &CameraIdentity,
    capture: DateTime<FixedOffset>,
    mode: SortMode,
    custom_fields: &[CustomDirectoryField],
    requested_order: Option<&[DestinationDepthSegment]>,
) -> Result<PathBuf, &'static str> {
    let original = portable_relative_path(original_relative_path)?;
    let filename = original.file_name().ok_or("missing filename")?.to_owned();
    let order = canonical_destination_depth_order(&mode, custom_fields.len(), requested_order)?;
    // Revalidation protects the path boundary when fields came from persisted
    // JSON rather than the constructor.
    custom_directory_prefix(custom_fields)?;

    let mut destination = PathBuf::new();
    for segment in order {
        match segment {
            DestinationDepthSegment::CustomField { index } => {
                let field = custom_fields
                    .get(usize::from(index))
                    .ok_or("destination depth order references an unknown custom field")?;
                destination.push(sanitize_destination_component(field.label()));
                destination.push(sanitize_destination_component(field.value()));
            }
            DestinationDepthSegment::CameraModel => destination.push(camera_folder(camera)),
            DestinationDepthSegment::CaptureDay => {
                destination.push(capture.format("%Y-%m-%d").to_string())
            }
            DestinationDepthSegment::CaptureInterval => {
                let minutes = match mode {
                    SortMode::CameraInterval { minutes } => minutes,
                    _ => {
                        return Err("destination depth segment is not supported by this sort mode")
                    }
                };
                destination.push(capture_interval_folder(capture, minutes)?);
            }
            DestinationDepthSegment::OriginalTree => {
                if let Some(parent) = original.parent() {
                    destination.push(parent);
                }
            }
        }
    }
    destination.push(filename);
    if is_portable_destination_relative_path(&destination) {
        Ok(destination)
    } else {
        Err("unsafe destination projection")
    }
}

fn camera_folder(camera: &CameraIdentity) -> String {
    let key_prefix = camera.key.chars().take(10).collect::<String>();
    format!(
        "{}__{}",
        sanitize_destination_component(&camera.display_label),
        sanitize_destination_component(&key_prefix)
    )
}

fn capture_interval_folder(
    capture: DateTime<FixedOffset>,
    minutes: u16,
) -> Result<String, &'static str> {
    validate_interval_minutes(minutes)?;
    // Each local capture day remains its own namespace. This makes an
    // arbitrary interval deterministic even when it does not divide a day:
    // its final bucket simply ends at the next local midnight.
    let minute = capture.hour() as u16 * 60 + capture.minute() as u16;
    let bucket = minute / minutes * minutes;
    let offset = capture.format("%:z").to_string().replace(':', "");
    Ok(format!("{:02}-{:02}_{offset}", bucket / 60, bucket % 60))
}

fn validate_interval_minutes(minutes: u16) -> Result<(), &'static str> {
    if !(1..=1_440).contains(&minutes) {
        return Err("interval must be from 1 to 1,440 minutes");
    }
    Ok(())
}

fn camera_display_label(make: &str, model: &str) -> String {
    let model = model.trim();
    if !model.is_empty() {
        return sanitize_destination_component(model);
    }
    let make = make.trim();
    if !make.is_empty() {
        return sanitize_destination_component(make);
    }
    "unknown-camera".into()
}

fn validate_custom_directory_component(
    value: &str,
    kind: &'static str,
) -> Result<(), &'static str> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(match kind {
            "custom field label" => "custom field label cannot be empty",
            _ => "custom field value cannot be empty",
        });
    }
    if value != trimmed || value == "." || value == ".." {
        return Err("custom directory fields cannot use whitespace-only or dot components");
    }
    if value
        .chars()
        .any(|character| "<>:\"/\\|?*".contains(character) || character.is_control())
    {
        return Err(
            "custom directory fields cannot contain path separators or reserved characters",
        );
    }
    Ok(())
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
        assert_eq!(first.display_label, "FX3");
        assert_ne!(camera_folder(&first), camera_folder(&second));
    }

    #[test]
    fn camera_folder_falls_back_without_turning_missing_metadata_into_a_shared_identity() {
        let make_only = camera_identity("Sony", "", None, "first-run");
        let unknown = camera_identity("", "", None, "second-run");
        assert_eq!(make_only.display_label, "Sony");
        assert_eq!(unknown.display_label, "unknown-camera");
        assert_ne!(camera_folder(&make_only), camera_folder(&unknown));
    }

    #[test]
    fn custom_directory_fields_create_a_portable_operator_prefix() {
        let fields = vec![
            CustomDirectoryField::new("Photographer", "Ari Tan").expect("photographer"),
            CustomDirectoryField::new("Project", "Night Market").expect("project"),
        ];
        let prefix = custom_directory_prefix(&fields).expect("prefix");
        assert_eq!(
            prefix,
            PathBuf::from("Photographer")
                .join("Ari Tan")
                .join("Project")
                .join("Night Market")
        );
        assert!(is_portable_destination_relative_path(&prefix));
    }

    #[test]
    fn custom_directory_fields_reject_empty_and_path_like_values() {
        assert_eq!(
            CustomDirectoryField::new("Photographer", " "),
            Err("custom field value cannot be empty")
        );
        assert_eq!(
            CustomDirectoryField::new("Photographer/role", "Ari"),
            Err("custom directory fields cannot contain path separators or reserved characters")
        );
        assert_eq!(
            CustomDirectoryField::new("Photographer", ".."),
            Err("custom directory fields cannot use whitespace-only or dot components")
        );
    }

    #[test]
    fn destination_depth_order_defaults_to_the_canonical_complete_layout() {
        let order =
            canonical_destination_depth_order(&SortMode::CameraInterval { minutes: 30 }, 2, None)
                .expect("canonical order");
        assert_eq!(
            order,
            vec![
                DestinationDepthSegment::CustomField { index: 0 },
                DestinationDepthSegment::CustomField { index: 1 },
                DestinationDepthSegment::CameraModel,
                DestinationDepthSegment::CaptureDay,
                DestinationDepthSegment::CaptureInterval,
            ]
        );
    }

    #[test]
    fn destination_depth_order_preserves_a_complete_dragged_permutation() {
        let requested = vec![
            DestinationDepthSegment::CaptureDay,
            DestinationDepthSegment::CustomField { index: 0 },
            DestinationDepthSegment::CameraModel,
        ];
        assert_eq!(
            canonical_destination_depth_order(&SortMode::CameraDay, 1, Some(&requested)),
            Ok(requested)
        );
    }

    #[test]
    fn destination_depth_order_rejects_invalid_missing_and_duplicate_segments() {
        assert_eq!(
            canonical_destination_depth_order(
                &SortMode::CameraDay,
                1,
                Some(&[
                    DestinationDepthSegment::CameraModel,
                    DestinationDepthSegment::CameraModel,
                    DestinationDepthSegment::CustomField { index: 0 },
                ])
            ),
            Err("destination depth order contains a duplicate segment")
        );
        assert_eq!(
            canonical_destination_depth_order(
                &SortMode::CameraDay,
                1,
                Some(&[
                    DestinationDepthSegment::CustomField { index: 0 },
                    DestinationDepthSegment::CameraModel,
                ])
            ),
            Err("destination depth order is missing a required segment")
        );
        assert_eq!(
            canonical_destination_depth_order(
                &SortMode::CameraDay,
                1,
                Some(&[
                    DestinationDepthSegment::CustomField { index: 4 },
                    DestinationDepthSegment::CameraModel,
                    DestinationDepthSegment::CaptureDay,
                ])
            ),
            Err("destination depth order references an unknown custom field")
        );
        assert_eq!(
            canonical_destination_depth_order(
                &SortMode::OriginalTree,
                0,
                Some(&[DestinationDepthSegment::CameraModel])
            ),
            Err("destination depth segment is not supported by this sort mode")
        );
    }

    #[test]
    fn destination_depth_order_reorders_directories_but_keeps_filename_final() {
        let camera = camera_identity("Sony", "FX3", Some("A-001"), "run");
        let fields = vec![CustomDirectoryField::new("Photographer", "Ari").expect("field")];
        let requested = vec![
            DestinationDepthSegment::CaptureDay,
            DestinationDepthSegment::CustomField { index: 0 },
            DestinationDepthSegment::CameraModel,
        ];
        let path = destination_relative_path_with_order(
            "DCIM/PRIVATE/CLIP.MOV",
            &camera,
            Utc.with_ymd_and_hms(2026, 8, 28, 13, 15, 0)
                .unwrap()
                .with_timezone(&FixedOffset::east_opt(0).unwrap()),
            SortMode::CameraDay,
            &fields,
            Some(&requested),
        )
        .expect("path");
        assert_eq!(
            path.file_name().and_then(|name| name.to_str()),
            Some("CLIP.MOV")
        );
        assert_eq!(
            path.components().count(),
            5,
            "day, custom label/value, camera, filename"
        );
        assert!(path
            .to_string_lossy()
            .starts_with("2026-08-28\\Photographer\\Ari\\FX3__"));
        assert!(is_portable_destination_relative_path(&path));
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
