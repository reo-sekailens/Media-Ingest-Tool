//! Best-effort, pure-Rust media metadata extraction.
//!
//! Unsupported or malformed media never blocks an ingest.  They retain an
//! explicit filesystem-time fallback and run-scoped unknown camera identity,
//! rather than inheriting another camera's model identity.

use chrono::{DateTime, FixedOffset, Utc};
use nom_exif::{read_metadata, ExifDateTime, ExifTag, Metadata, TrackInfoTag};
use std::path::Path;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CaptureTimeSource {
    ExifOriginalWithOffset,
    ExifOriginalWithoutOffset,
    ContainerCreateWithOffset,
    ContainerCreateWithoutOffset,
    FilesystemModifiedUtc,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MediaMetadata {
    pub make: String,
    pub model: String,
    pub body_serial: Option<String>,
    pub capture_time: DateTime<FixedOffset>,
    /// Whether `capture_time` carries a real embedded UTC offset. A false
    /// value means this is a camera wall clock with no claimed UTC offset.
    pub capture_offset_known: bool,
    pub capture_time_source: CaptureTimeSource,
    /// A parse failure/missing field is deliberately retained as a warning,
    /// not promoted into another camera's identity.
    pub warning: Option<String>,
}

pub fn inspect(path: &Path) -> MediaMetadata {
    let fallback = filesystem_time(path);
    let Ok(metadata) = read_metadata(path) else {
        return fallback;
    };
    let Metadata::Exif(exif) = metadata else {
        let Metadata::Track(track) = metadata else {
            return fallback;
        };
        let capture_time = track
            .get(TrackInfoTag::CreateDate)
            .and_then(|value| value.as_datetime())
            .map(capture_time_from_exif);
        return capture_time.map_or_else(
            || fallback,
            |(capture_time, capture_offset_known)| MediaMetadata {
                make: "Unknown make".into(),
                model: "Unknown model".into(),
                body_serial: None,
                capture_time,
                capture_offset_known,
                capture_time_source: if capture_offset_known {
                    CaptureTimeSource::ContainerCreateWithOffset
                } else {
                    CaptureTimeSource::ContainerCreateWithoutOffset
                },
                warning: Some(if capture_offset_known {
                    "Container creation time has no embedded camera body serial".into()
                } else {
                    "Container creation time has no UTC offset; using its recorded wall clock and an offset-unknown folder label".into()
                }),
            },
        );
    };
    let make = exif
        .get(ExifTag::Make)
        .and_then(|value| value.as_str())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| "Unknown make".into());
    let model = exif
        .get(ExifTag::Model)
        .and_then(|value| value.as_str())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| "Unknown model".into());
    let body_serial = exif
        .get(ExifTag::CameraSerialNumber)
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let capture_time = exif
        .get(ExifTag::DateTimeOriginal)
        .and_then(|value| value.as_datetime())
        .map(capture_time_from_exif);
    match capture_time {
        Some((capture_time, true)) => MediaMetadata {
            make,
            model,
            body_serial,
            capture_time,
            capture_offset_known: true,
            capture_time_source: CaptureTimeSource::ExifOriginalWithOffset,
            warning: None,
        },
        Some((capture_time, false)) => MediaMetadata {
            make,
            model,
            body_serial,
            capture_time,
            capture_offset_known: false,
            capture_time_source: CaptureTimeSource::ExifOriginalWithoutOffset,
            warning: Some(
                "DateTimeOriginal has no UTC offset; using its recorded camera wall clock and an offset-unknown folder label".into(),
            ),
        },
        None => {
            let mut fallback = fallback;
            fallback.make = make;
            fallback.model = model;
            fallback.body_serial = body_serial;
            fallback.warning = Some(
                "No readable DateTimeOriginal metadata; using filesystem modified time in UTC".into(),
            );
            fallback
        }
    }
}

/// EXIF's `OffsetTimeOriginal` is preferred when present. A naive
/// `DateTimeOriginal` remains a camera wall clock when it has no embedded
/// offset. Retain its components, but never convert it through this host or
/// label it as a known numeric offset.
fn capture_time_from_exif(value: ExifDateTime) -> (DateTime<FixedOffset>, bool) {
    match value {
        ExifDateTime::Aware(value) => (value, true),
        ExifDateTime::Naive(value) => (value.and_utc().fixed_offset(), false),
    }
}

fn filesystem_time(path: &Path) -> MediaMetadata {
    let capture_time = std::fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .map(DateTime::<Utc>::from)
        .unwrap_or_else(|_| Utc::now())
        .with_timezone(&FixedOffset::east_opt(0).expect("UTC offset"));
    MediaMetadata {
        make: "Unknown make".into(),
        model: "Unknown model".into(),
        body_serial: None,
        capture_time,
        capture_offset_known: true,
        capture_time_source: CaptureTimeSource::FilesystemModifiedUtc,
        warning: Some(
            "No readable embedded media metadata; using filesystem modified time in UTC".into(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use uuid::Uuid;

    #[test]
    fn unsupported_file_keeps_explicit_utc_fallback() {
        let path = std::env::temp_dir().join(format!("metadata-{}.bin", Uuid::new_v4()));
        fs::write(&path, b"not media").expect("fixture");
        let metadata = inspect(&path);
        assert_eq!(metadata.capture_time.offset().local_minus_utc(), 0);
        assert_eq!(
            metadata.capture_time_source,
            CaptureTimeSource::FilesystemModifiedUtc
        );
        assert!(metadata.capture_offset_known);
        assert!(metadata.warning.is_some());
        fs::remove_file(path).expect("cleanup");
    }

    #[test]
    fn naive_exif_time_preserves_camera_wall_clock_without_a_host_offset() {
        let naive = chrono::NaiveDate::from_ymd_opt(2026, 8, 23)
            .expect("date")
            .and_hms_opt(5, 30, 0)
            .expect("time");
        let (capture, had_exif_offset) = capture_time_from_exif(ExifDateTime::Naive(naive));
        assert!(!had_exif_offset);
        assert_eq!(capture.naive_local(), naive);
        assert_eq!(capture.offset().local_minus_utc(), 0);
    }
}
