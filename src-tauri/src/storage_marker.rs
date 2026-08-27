//! Compact app-owned continuity record for removable media.
//!
//! The record pairs a random app identifier with a bounded content witness.
//! It is mutable filesystem evidence: copying it copies the identity, so it
//! must never be promoted to hardware identity or format authorization.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use uuid::Uuid;

pub const MARKER_FILE_NAME: &str = ".media-ingest-device-id";
const LEGACY_PREFIX: &str = "MIT1:";
const PREFIX: &str = "MIT2:";
const FINGERPRINT_PREFIX: &str = "F:";
const MARKER_MAX_BYTES: u64 = 128;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedCardRecord {
    pub token: String,
    /// BLAKE3 content witness from the completed verified ingest, or absent
    /// for a legacy/registration-only record.
    pub content_fingerprint: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MarkerState {
    Existing,
    Created,
}

pub fn marker_path(root: &Path) -> PathBuf {
    root.join(MARKER_FILE_NAME)
}

pub fn read_record(root: &Path) -> io::Result<Option<ManagedCardRecord>> {
    let path = marker_path(root);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if !metadata.file_type().is_file() || metadata.len() > MARKER_MAX_BYTES {
        return Ok(None);
    }
    let mut content = String::new();
    File::open(path)?
        .take(MARKER_MAX_BYTES)
        .read_to_string(&mut content)?;
    Ok(parse_record(&content))
}

/// Compatibility projection used by existing profile storage and discovery.
pub fn read_marker(root: &Path) -> io::Result<Option<String>> {
    Ok(read_record(root)?.map(|record| record.token))
}

/// Creates the compact record only after a completed verified ingest. The
/// caller supplies the sealed content-manifest root, so the on-card file stays
/// below 128 bytes and never exposes media names or paths.
pub fn ensure_marker_with_fingerprint(
    root: &Path,
    content_fingerprint: &str,
) -> io::Result<MarkerState> {
    if !is_hex_digest(content_fingerprint) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid content fingerprint",
        ));
    }
    if let Some(existing) = read_record(root)? {
        let fingerprint = content_fingerprint.to_ascii_lowercase();
        if existing.content_fingerprint.as_deref() != Some(fingerprint.as_str()) {
            return rewrite_known_record(
                &marker_path(root),
                &ManagedCardRecord {
                    token: existing.token,
                    content_fingerprint: Some(fingerprint),
                },
            );
        }
        return Ok(MarkerState::Existing);
    }
    let path = marker_path(root);
    let record = ManagedCardRecord {
        token: format!("{PREFIX}{}", Uuid::new_v4()),
        content_fingerprint: Some(content_fingerprint.to_ascii_lowercase()),
    };
    write_new_record(&path, &record)
}

/// Registration may happen before the first completed ingest, so retain a
/// compact token-only record until the next verified input creates a witness.
pub fn ensure_marker(root: &Path) -> io::Result<MarkerState> {
    if read_record(root)?.is_some() {
        return Ok(MarkerState::Existing);
    }
    let path = marker_path(root);
    let record = ManagedCardRecord {
        token: format!("{PREFIX}{}", Uuid::new_v4()),
        content_fingerprint: None,
    };
    // An empty reserved file can only be an interrupted earlier marker write;
    // it contains no user data and cannot identify a card. Repair it instead
    // of permanently blocking the card from registration.
    if fs::symlink_metadata(&path)
        .map(|metadata| metadata.file_type().is_file() && metadata.len() == 0)
        .unwrap_or(false)
    {
        replace_record(&path, &record)?;
        return Ok(MarkerState::Created);
    }
    write_new_record(&path, &record)
}

pub fn restore_marker(root: &Path, token: &str) -> io::Result<MarkerState> {
    if !valid_token(token) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid app marker token",
        ));
    }
    match read_marker(root)? {
        Some(existing) if existing == token => return Ok(MarkerState::Existing),
        Some(_) => {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "a different app marker already exists",
            ))
        }
        None => {}
    }
    write_new_record(
        &marker_path(root),
        &ManagedCardRecord {
            token: token.into(),
            content_fingerprint: None,
        },
    )
}

fn write_new_record(path: &Path, record: &ManagedCardRecord) -> io::Result<MarkerState> {
    let content = encode_record(record)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid marker record"))?;
    match OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(mut marker) => {
            marker.write_all(content.as_bytes())?;
            marker.sync_all()?;
            Ok(MarkerState::Created)
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            if read_record(path.parent().unwrap_or_else(|| Path::new(".")))?.is_some() {
                Ok(MarkerState::Existing)
            } else {
                Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "the app marker path is occupied by an unknown file",
                ))
            }
        }
        Err(error) => Err(error),
    }
}

/// Only a successfully parsed app record is upgraded. Its token is retained,
/// so existing local registrations keep their continuity key.
fn rewrite_known_record(path: &Path, record: &ManagedCardRecord) -> io::Result<MarkerState> {
    if let Err(replace_error) = replace_record(path, record) {
        // A full source card cannot allocate the durable sibling used by the
        // normal atomic replacement. Both MIT2 forms are fixed-size (109
        // bytes), so update an already validated record in place as the safe
        // fallback. An interrupted overwrite is malformed and therefore
        // fails closed; it can never authorize formatting.
        overwrite_record_in_place(path, record).map_err(|_| replace_error)?;
    }
    Ok(MarkerState::Existing)
}

fn overwrite_record_in_place(path: &Path, record: &ManagedCardRecord) -> io::Result<()> {
    let content = encode_record(record)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid marker record"))?;
    let metadata = fs::metadata(path)?;
    if metadata.len() != content.len() as u64 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "marker length cannot be refreshed in place",
        ));
    }
    let mut marker = OpenOptions::new().write(true).open(path)?;
    marker.seek(SeekFrom::Start(0))?;
    marker.write_all(content.as_bytes())?;
    marker.sync_all()
}

/// Writes a durable sibling first, then swaps it into place. In particular,
/// never truncate the live marker before its replacement is complete: a
/// transient write failure must retain the last valid registration record.
fn replace_record(path: &Path, record: &ManagedCardRecord) -> io::Result<()> {
    let content = encode_record(record)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid marker record"))?;
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let temporary = parent.join(format!(
        ".{}-{}.tmp",
        MARKER_FILE_NAME.trim_start_matches('.'),
        Uuid::new_v4()
    ));
    let mut marker = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    if let Err(error) = marker
        .write_all(content.as_bytes())
        .and_then(|()| marker.sync_all())
    {
        drop(marker);
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    drop(marker);
    match replace_path(&temporary, path) {
        Ok(()) => Ok(()),
        Err(error) => {
            let _ = fs::remove_file(&temporary);
            Err(error)
        }
    }
}

#[cfg(windows)]
fn replace_path(from: &Path, to: &Path) -> io::Result<()> {
    use windows::core::{HSTRING, PCWSTR};
    use windows::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    let from = HSTRING::from(from.as_os_str().to_string_lossy().as_ref());
    let to = HSTRING::from(to.as_os_str().to_string_lossy().as_ref());
    unsafe {
        MoveFileExW(
            PCWSTR(from.as_ptr()),
            PCWSTR(to.as_ptr()),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
        .map_err(|error| io::Error::from_raw_os_error(error.code().0))
    }
}

#[cfg(not(windows))]
fn replace_path(from: &Path, to: &Path) -> io::Result<()> {
    fs::rename(from, to)
}

fn encode_record(record: &ManagedCardRecord) -> Option<String> {
    if !valid_token(&record.token) {
        return None;
    }
    match &record.content_fingerprint {
        Some(fingerprint) if is_hex_digest(fingerprint) => Some(format!(
            "{}\n{FINGERPRINT_PREFIX}{}\n",
            record.token,
            fingerprint.to_ascii_lowercase()
        )),
        Some(_) => None,
        None => Some(format!("{}\n", record.token)),
    }
}

fn parse_record(content: &str) -> Option<ManagedCardRecord> {
    let mut lines = content.strip_suffix('\n')?.split('\n');
    let token = lines.next()?.to_owned();
    if !valid_token(&token) {
        return None;
    }
    let content_fingerprint = match lines.next() {
        Some(line) => {
            let fingerprint = line.strip_prefix(FINGERPRINT_PREFIX)?;
            is_hex_digest(fingerprint).then(|| fingerprint.to_ascii_lowercase())
        }
        None => None,
    };
    lines.next().is_none().then_some(ManagedCardRecord {
        token,
        content_fingerprint,
    })
}

fn valid_token(token: &str) -> bool {
    let uuid = token
        .strip_prefix(PREFIX)
        .or_else(|| token.strip_prefix(LEGACY_PREFIX));
    uuid.and_then(|value| Uuid::parse_str(value).ok())
        .is_some_and(|parsed| parsed.to_string() == token[5..].to_ascii_lowercase())
}

fn is_hex_digest(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_record_round_trips_with_a_content_witness() {
        let root = std::env::temp_dir().join(format!("marker-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).expect("root");
        let fingerprint = "a".repeat(64);
        assert_eq!(
            ensure_marker_with_fingerprint(&root, &fingerprint).expect("create"),
            MarkerState::Created
        );
        let record = read_record(&root).expect("read").expect("record");
        assert_eq!(
            record.content_fingerprint.as_deref(),
            Some(fingerprint.as_str())
        );
        assert!(fs::metadata(marker_path(&root)).expect("metadata").len() <= MARKER_MAX_BYTES);
        let token = record.token;
        let refreshed = "b".repeat(64);
        assert_eq!(
            ensure_marker_with_fingerprint(&root, &refreshed).expect("refresh"),
            MarkerState::Existing
        );
        let refreshed_record = read_record(&root).expect("read refreshed").expect("record");
        assert_eq!(refreshed_record.token, token);
        assert_eq!(
            refreshed_record.content_fingerprint.as_deref(),
            Some(refreshed.as_str())
        );
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn legacy_and_corrupt_records_are_handled_without_promotion() {
        let root = std::env::temp_dir().join(format!("marker-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).expect("root");
        let token = format!("{LEGACY_PREFIX}{}", Uuid::new_v4());
        fs::write(marker_path(&root), format!("{token}\n")).expect("legacy");
        assert_eq!(read_marker(&root).expect("read"), Some(token));
        fs::write(marker_path(&root), b"MIT2:not-a-uuid\n").expect("corrupt");
        assert!(read_record(&root).expect("read").is_none());
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn empty_interrupted_marker_is_repaired_without_reusing_a_token() {
        let root = std::env::temp_dir().join(format!("marker-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).expect("root");
        fs::write(marker_path(&root), []).expect("interrupted marker");

        assert_eq!(ensure_marker(&root).expect("repair"), MarkerState::Created);
        let record = read_record(&root).expect("read").expect("record");
        assert!(record.token.starts_with(PREFIX));
        assert!(record.content_fingerprint.is_none());
        assert!(fs::metadata(marker_path(&root)).expect("metadata").len() > 0);
        fs::remove_dir_all(root).expect("cleanup");
    }

    /// Restores a known managed-card token on a sacrificial mounted volume.
    /// This is intentionally opt-in because it writes only the compact marker,
    /// never media or an inferred content witness.
    #[test]
    #[ignore = "set MEDIA_INGEST_HW_ROOT and MEDIA_INGEST_HW_MARKER_TOKEN"]
    fn hardware_restore_registered_marker() {
        let root = std::env::var("MEDIA_INGEST_HW_ROOT").expect("set mounted card root");
        let token =
            std::env::var("MEDIA_INGEST_HW_MARKER_TOKEN").expect("set known managed-card token");
        restore_marker(Path::new(&root), &token).expect("restore registered marker");
        assert_eq!(
            read_marker(Path::new(&root)).expect("read restored marker"),
            Some(token)
        );
    }
}
