//! Best-effort, pure-Rust media metadata extraction.
//!
//! Unsupported or malformed media retains an explicit filesystem-time
//! fallback when readable and a run-scoped unknown camera identity. Missing
//! clocks are reported rather than substituting the time of this ingest.

use chrono::{DateTime, FixedOffset, Utc};
use nom_exif::{read_metadata, ExifDateTime, ExifTag, Metadata, TrackInfoTag};
use std::io;
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

pub fn inspect(path: &Path) -> io::Result<MediaMetadata> {
    inspect_with_fallback(path, || filesystem_time(path))
}

fn inspect_with_fallback(
    path: &Path,
    filesystem_fallback: impl FnOnce() -> io::Result<MediaMetadata>,
) -> io::Result<MediaMetadata> {
    let Ok(metadata) = read_metadata(path) else {
        return filesystem_fallback();
    };
    let Metadata::Exif(exif) = metadata else {
        let Metadata::Track(track) = metadata else {
            return filesystem_fallback();
        };
        let capture_time = track
            .get(TrackInfoTag::CreateDate)
            .and_then(|value| value.as_datetime())
            .map(capture_time_from_exif);
        return capture_time.map_or_else(
            filesystem_fallback,
            |(capture_time, capture_offset_known)| Ok(MediaMetadata {
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
            }),
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
        .map(|value| {
            capture_time_from_original_exif(
                value,
                exif.get(ExifTag::OffsetTimeOriginal)
                    .and_then(|offset| offset.as_str()),
            )
        });
    match capture_time {
        Some((capture_time, true)) => Ok(MediaMetadata {
            make,
            model,
            body_serial,
            capture_time,
            capture_offset_known: true,
            capture_time_source: CaptureTimeSource::ExifOriginalWithOffset,
            warning: None,
        }),
        Some((capture_time, false)) => Ok(MediaMetadata {
            make,
            model,
            body_serial,
            capture_time,
            capture_offset_known: false,
            capture_time_source: CaptureTimeSource::ExifOriginalWithoutOffset,
            warning: Some(
                "DateTimeOriginal has no usable OffsetTimeOriginal; using its recorded camera wall clock and an offset-unknown folder label".into(),
            ),
        }),
        None => {
            let mut fallback = filesystem_fallback()?;
            fallback.make = make;
            fallback.model = model;
            fallback.body_serial = body_serial;
            fallback.warning = Some(
                "No readable DateTimeOriginal metadata; using filesystem modified time in UTC".into(),
            );
            Ok(fallback)
        }
    }
}

/// The pinned reader can apply a digitization/modification offset to every
/// EXIF timestamp. Recover the recorded wall clock and accept only the offset
/// tag belonging to DateTimeOriginal before claiming a known capture instant.
fn capture_time_from_original_exif(
    value: ExifDateTime,
    original_offset: Option<&str>,
) -> (DateTime<FixedOffset>, bool) {
    let wall_clock = value.into_naive();
    if let Some(capture) = original_offset
        .and_then(parse_original_offset)
        .and_then(|offset| wall_clock.and_local_timezone(offset).single())
    {
        (capture, true)
    } else {
        (wall_clock.and_utc().fixed_offset(), false)
    }
}

fn parse_original_offset(value: &str) -> Option<FixedOffset> {
    let value = value.trim().as_bytes();
    if value.len() != 6
        || !matches!(value[0], b'+' | b'-')
        || value[3] != b':'
        || ![value[1], value[2], value[4], value[5]]
            .iter()
            .all(u8::is_ascii_digit)
    {
        return None;
    }
    let hours = i32::from(value[1] - b'0') * 10 + i32::from(value[2] - b'0');
    let minutes = i32::from(value[4] - b'0') * 10 + i32::from(value[5] - b'0');
    if minutes >= 60 {
        return None;
    }
    let seconds = hours * 3600 + minutes * 60;
    FixedOffset::east_opt(if value[0] == b'-' { -seconds } else { seconds })
}

/// Container timestamps without an embedded offset retain their wall-clock
/// components without claiming a known numeric offset or host-local instant.
fn capture_time_from_exif(value: ExifDateTime) -> (DateTime<FixedOffset>, bool) {
    match value {
        ExifDateTime::Aware(value) => (value, true),
        ExifDateTime::Naive(value) => (value.and_utc().fixed_offset(), false),
    }
}

fn filesystem_time(path: &Path) -> io::Result<MediaMetadata> {
    let capture_time = std::fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .map(DateTime::<Utc>::from)?
        .with_timezone(&FixedOffset::east_opt(0).expect("UTC offset"));
    Ok(MediaMetadata {
        make: "Unknown make".into(),
        model: "Unknown model".into(),
        body_serial: None,
        capture_time,
        capture_offset_known: true,
        capture_time_source: CaptureTimeSource::FilesystemModifiedUtc,
        warning: Some(
            "No readable embedded media metadata; using filesystem modified time in UTC".into(),
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{NaiveDate, Timelike};
    use std::fs;
    use uuid::Uuid;

    /// A TIFF EXIF block with DateTimeOriginal and independently selected
    /// offset tags. The production nom-exif reader parses these actual bytes.
    fn original_time_tiff(capture_time: &str, offsets: &[(u16, &str)]) -> Vec<u8> {
        fn write_ifd(bytes: &mut [u8], offset: usize, entries: &[(u16, u16, u32, u32)]) {
            bytes[offset..offset + 2].copy_from_slice(&(entries.len() as u16).to_le_bytes());
            for (index, (tag, field_type, count, value)) in entries.iter().enumerate() {
                let start = offset + 2 + index * 12;
                bytes[start..start + 2].copy_from_slice(&tag.to_le_bytes());
                bytes[start + 2..start + 4].copy_from_slice(&field_type.to_le_bytes());
                bytes[start + 4..start + 8].copy_from_slice(&count.to_le_bytes());
                bytes[start + 8..start + 12].copy_from_slice(&value.to_le_bytes());
            }
        }

        fn append_ascii(bytes: &mut Vec<u8>, value: &str) -> (u32, u32) {
            if value.len() < 4 {
                let mut inline = [0_u8; 4];
                inline[..value.len()].copy_from_slice(value.as_bytes());
                return ((value.len() + 1) as u32, u32::from_le_bytes(inline));
            }
            if !bytes.len().is_multiple_of(2) {
                bytes.push(0);
            }
            let offset = bytes.len() as u32;
            bytes.extend_from_slice(value.as_bytes());
            bytes.push(0);
            ((value.len() + 1) as u32, offset)
        }

        // A complete one-pixel baseline image is required for the production
        // reader to recognize TIFF; an EXIF-pointer-only IFD is not an image.
        let image_offset = 8;
        let exif_offset = image_offset + 2 + 15 * 12 + 4;
        let entry_count = 4 + offsets.len();
        let mut bytes = vec![0_u8; exif_offset + 2 + entry_count * 12 + 4];
        bytes[..8].copy_from_slice(b"II*\0\x08\0\0\0");
        let (make_count, make_offset) = append_ascii(&mut bytes, "Sony");
        let (count, value) = append_ascii(&mut bytes, capture_time);
        let mut exif_entries = vec![
            (0x9000, 7, 4, u32::from_le_bytes(*b"0231")),
            (0x9003, 2, count, value),
        ];
        for (tag, text) in offsets {
            let (count, value) = append_ascii(&mut bytes, text);
            exif_entries.push((*tag, 2, count, value));
        }
        exif_entries.extend([
            (0xa000, 7, 4, u32::from_le_bytes(*b"0100")),
            (0xa001, 3, 1, 1),
        ]);
        exif_entries.sort_unstable_by_key(|entry| entry.0);
        if !bytes.len().is_multiple_of(2) {
            bytes.push(0);
        }
        let resolution_offset = bytes.len() as u32;
        bytes.extend_from_slice(&72_u32.to_le_bytes());
        bytes.extend_from_slice(&1_u32.to_le_bytes());
        let pixel_offset = bytes.len() as u32;
        bytes.push(127);
        write_ifd(
            &mut bytes,
            image_offset,
            &[
                (0x0100, 4, 1, 1),
                (0x0101, 4, 1, 1),
                (0x0102, 3, 1, 8),
                (0x0103, 3, 1, 1),
                (0x0106, 3, 1, 1),
                (0x010f, 2, make_count, make_offset),
                (0x0110, 2, 4, u32::from_le_bytes(*b"FX3\0")),
                (0x0111, 4, 1, pixel_offset),
                (0x0115, 3, 1, 1),
                (0x0116, 4, 1, 1),
                (0x0117, 4, 1, 1),
                (0x011a, 5, 1, resolution_offset),
                (0x011b, 5, 1, resolution_offset),
                (0x0128, 3, 1, 2),
                (0x8769, 4, 1, exif_offset as u32),
            ],
        );
        write_ifd(&mut bytes, exif_offset, &exif_entries);
        bytes
    }

    #[test]
    fn original_offset_tag_preserves_every_numeric_minute_offset_without_host_conversion() {
        let path = std::env::temp_dir().join(format!("metadata-offsets-{}.tif", Uuid::new_v4()));
        let wall_clock = NaiveDate::from_ymd_opt(2026, 1, 1)
            .expect("date")
            .and_hms_opt(0, 15, 59)
            .expect("time");
        for offset_minutes in -1_439_i32..=1_439 {
            let magnitude = offset_minutes.abs();
            let offset = format!(
                "{}{:02}:{:02}",
                if offset_minutes < 0 { '-' } else { '+' },
                magnitude / 60,
                magnitude % 60,
            );
            fs::write(
                &path,
                original_time_tiff("2026:01:01 00:15:59", &[(0x9011, &offset)]),
            )
            .expect("EXIF fixture");
            let metadata = inspect(&path).expect("EXIF metadata");
            assert_eq!(metadata.capture_time.naive_local(), wall_clock);
            assert_eq!(
                metadata.capture_time.offset().local_minus_utc(),
                offset_minutes * 60
            );
            assert_eq!(
                metadata.capture_time_source,
                CaptureTimeSource::ExifOriginalWithOffset
            );
            assert!(metadata.capture_offset_known);
            assert!(metadata.warning.is_none());
        }
        fs::remove_file(path).expect("cleanup");
    }

    #[test]
    fn original_capture_never_borrows_digitization_or_modification_offsets() {
        let path =
            std::env::temp_dir().join(format!("metadata-offset-tags-{}.tif", Uuid::new_v4()));
        let wall_clock = NaiveDate::from_ymd_opt(2025, 12, 31)
            .expect("date")
            .and_hms_opt(23, 42, 59)
            .expect("time");
        for offsets in [
            vec![],
            vec![(0x9012, "-07:00")],
            vec![(0x9010, "+09:30")],
            vec![(0x9011, "broken"), (0x9012, "-07:00")],
            vec![(0x9011, "+24:00"), (0x9012, "-07:00")],
            vec![(0x9011, "+05:60")],
            vec![(0x9011, "-07:0")],
        ] {
            fs::write(&path, original_time_tiff("2025:12:31 23:42:59", &offsets))
                .expect("EXIF fixture");
            let metadata = inspect(&path).expect("EXIF metadata");
            assert_eq!(metadata.capture_time.naive_local(), wall_clock);
            assert_eq!(
                metadata.capture_time_source,
                CaptureTimeSource::ExifOriginalWithoutOffset
            );
            assert!(!metadata.capture_offset_known);
            assert_eq!(metadata.capture_time.offset().local_minus_utc(), 0);
            assert!(metadata.warning.is_some());
        }
        fs::write(
            &path,
            original_time_tiff(
                "2025:12:31 23:42:59",
                &[(0x9010, "+09:30"), (0x9011, "+05:45"), (0x9012, "-07:00")],
            ),
        )
        .expect("EXIF fixture with distinct offsets");
        let metadata = inspect(&path).expect("EXIF metadata");
        assert_eq!(metadata.capture_time.naive_local(), wall_clock);
        assert_eq!(metadata.capture_time.offset().local_minus_utc(), 345 * 60);
        assert_eq!(
            metadata.capture_time_source,
            CaptureTimeSource::ExifOriginalWithOffset
        );
        assert!(metadata.capture_offset_known);
        fs::remove_file(path).expect("cleanup");
    }

    #[test]
    fn embedded_capture_does_not_require_a_readable_filesystem_timestamp() {
        let path =
            std::env::temp_dir().join(format!("metadata-lazy-fallback-{}.tif", Uuid::new_v4()));
        fs::write(
            &path,
            original_time_tiff("2026:01:01 13:42:59", &[(0x9011, "+05:45")]),
        )
        .expect("EXIF fixture");
        let metadata = inspect_with_fallback(&path, || {
            Err(io::Error::from(io::ErrorKind::PermissionDenied))
        })
        .expect("embedded capture must succeed without filesystem time");
        assert_eq!(
            metadata.capture_time_source,
            CaptureTimeSource::ExifOriginalWithOffset
        );
        assert_eq!(metadata.capture_time.hour(), 13);
        assert_eq!(metadata.capture_time.offset().local_minus_utc(), 345 * 60);
        fs::remove_file(path).expect("cleanup");
    }

    #[test]
    fn unavailable_capture_and_filesystem_time_return_an_error_instead_of_ingest_time() {
        let missing = std::env::temp_dir().join(format!("metadata-missing-{}.bin", Uuid::new_v4()));
        assert_eq!(
            inspect(&missing).expect_err("no capture time").kind(),
            io::ErrorKind::NotFound
        );
        let path = std::env::temp_dir().join(format!("metadata-time-error-{}.bin", Uuid::new_v4()));
        fs::write(&path, b"no embedded metadata").expect("unsupported fixture");
        assert_eq!(
            inspect_with_fallback(&path, || Err(io::Error::from(
                io::ErrorKind::PermissionDenied
            )))
            .expect_err("no fabricated clock")
            .kind(),
            io::ErrorKind::PermissionDenied
        );
        fs::remove_file(path).expect("cleanup");
    }

    #[test]
    fn filesystem_fallback_preserves_a_known_utc_instant_for_computer_conversion() {
        use crate::organization::{resolve_capture_time_basis_with_offset, CaptureTimeBasis};
        let path =
            std::env::temp_dir().join(format!("metadata-filesystem-time-{}.bin", Uuid::new_v4()));
        fs::write(&path, b"no embedded metadata").expect("unsupported fixture");
        let metadata = inspect(&path).expect("filesystem timestamp");
        let actual = DateTime::<Utc>::from(
            fs::metadata(&path)
                .expect("metadata")
                .modified()
                .expect("modified"),
        );
        assert_eq!(metadata.capture_time.timestamp(), actual.timestamp());
        let offset = FixedOffset::east_opt(345 * 60).expect("computer offset");
        let converted = resolve_capture_time_basis_with_offset(
            metadata.capture_time,
            metadata.capture_offset_known,
            CaptureTimeBasis::ComputerLocal,
            |_| offset,
        );
        assert_eq!(converted.timestamp(), actual.timestamp());
        assert_eq!(*converted.offset(), offset);
        fs::remove_file(path).expect("cleanup");
    }

    #[test]
    fn unsupported_file_keeps_explicit_utc_fallback() {
        let path = std::env::temp_dir().join(format!("metadata-{}.bin", Uuid::new_v4()));
        fs::write(&path, b"not media").expect("fixture");
        let metadata = inspect(&path).expect("filesystem fallback");
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
