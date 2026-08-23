//! Best-effort, pure-Rust media metadata extraction.
//!
//! Unsupported or malformed media never blocks an ingest.  They retain an
//! explicit filesystem-time fallback and run-scoped unknown camera identity,
//! rather than inheriting host-local time or another camera's model identity.

use chrono::{DateTime, FixedOffset, Utc};
use nom_exif::{read_metadata, ExifDateTime, ExifTag, Metadata, TrackInfoTag};
use std::path::Path;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CaptureTimeSource {
    ExifOriginalWithOffset,
    ContainerCreateWithOffset,
    FilesystemModifiedUtc,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MediaMetadata {
    pub make: String,
    pub model: String,
    pub body_serial: Option<String>,
    pub capture_time: DateTime<FixedOffset>,
    pub capture_time_source: CaptureTimeSource,
    /// A parse failure/missing field is deliberately retained as a warning,
    /// not promoted into a guessed camera or local timezone.
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
            .and_then(|value| match value {
                ExifDateTime::Aware(value) => Some(value),
                ExifDateTime::Naive(_) => None,
            });
        return capture_time.map_or_else(
            || fallback,
            |capture_time| MediaMetadata {
                make: "Unknown make".into(),
                model: "Unknown model".into(),
                body_serial: None,
                capture_time,
                capture_time_source: CaptureTimeSource::ContainerCreateWithOffset,
                warning: Some("Container creation time has no embedded camera body serial".into()),
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
        .and_then(|value| match value {
            ExifDateTime::Aware(value) => Some(value),
            // The project has no configured camera timezone yet.  Never use
            // host-local time to give a naive timestamp false precision.
            ExifDateTime::Naive(_) => None,
        });
    match capture_time {
        Some(capture_time) => MediaMetadata {
            make,
            model,
            body_serial,
            capture_time,
            capture_time_source: CaptureTimeSource::ExifOriginalWithOffset,
            warning: None,
        },
        None => {
            let mut fallback = fallback;
            fallback.make = make;
            fallback.model = model;
            fallback.body_serial = body_serial;
            fallback.warning = Some(
                "No timezone-aware DateTimeOriginal metadata; using filesystem modified time in UTC"
                    .into(),
            );
            fallback
        }
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
        assert!(metadata.warning.is_some());
        fs::remove_file(path).expect("cleanup");
    }
}
