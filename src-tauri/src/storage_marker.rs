//! App-owned removable-media continuity marker.
//!
//! This marker makes a card easier to recognize after a mount-name change, but
//! it is ordinary mutable filesystem content. It is never immutable hardware
//! identity and cannot authorize automatic destination recall or formatting.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use uuid::Uuid;

pub const MARKER_FILE_NAME: &str = ".media-ingest-device-id";
const MARKER_PREFIX: &str = "MIT1:";
const MARKER_MAX_BYTES: u64 = 64;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MarkerState {
    Existing,
    Created,
}

pub fn marker_path(root: &Path) -> PathBuf {
    root.join(MARKER_FILE_NAME)
}

/// Returns a normalized marker token only when the root-level app file has the
/// exact compact format. A copied marker remains filesystem evidence, not a
/// physical-medium identifier.
pub fn read_marker(root: &Path) -> io::Result<Option<String>> {
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
    Ok(parse_marker(&content))
}

/// Creates the marker only after a completed verified ingest. `create_new`
/// prevents replacement of an existing file, including one created by another
/// ingest racing on the same card.
pub fn ensure_marker(root: &Path) -> io::Result<MarkerState> {
    if read_marker(root)?.is_some() {
        return Ok(MarkerState::Existing);
    }
    let path = marker_path(root);
    let token = format!("{MARKER_PREFIX}{}\n", Uuid::new_v4());
    match OpenOptions::new().write(true).create_new(true).open(&path) {
        Ok(mut marker) => {
            marker.write_all(token.as_bytes())?;
            marker.sync_all()?;
            Ok(MarkerState::Created)
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            // A concurrent creator may have won; only recognize a valid app
            // marker and never overwrite an unknown user file.
            if read_marker(root)?.is_some() {
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

/// Restores a previously registered marker after a formatter has recreated
/// and validated the filesystem. The caller supplies only a token previously
/// read from this app's local store; an unrelated file is never overwritten.
pub fn restore_marker(root: &Path, token: &str) -> io::Result<MarkerState> {
    if parse_marker(&(token.to_owned() + "\n")).is_none() {
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
    let path = marker_path(root);
    let mut marker = OpenOptions::new().write(true).create_new(true).open(path)?;
    marker.write_all(format!("{token}\n").as_bytes())?;
    marker.sync_all()?;
    Ok(MarkerState::Created)
}

fn parse_marker(content: &str) -> Option<String> {
    let token = content.strip_suffix('\n')?;
    let uuid = token.strip_prefix(MARKER_PREFIX)?;
    let parsed = Uuid::parse_str(uuid).ok()?;
    (parsed.to_string() == uuid.to_ascii_lowercase()).then(|| token.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marker_is_compact_and_idempotent_without_overwriting() {
        let root = std::env::temp_dir().join(format!("marker-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).expect("root");
        assert_eq!(ensure_marker(&root).expect("create"), MarkerState::Created);
        let token = read_marker(&root).expect("read").expect("marker");
        assert!(token.starts_with(MARKER_PREFIX));
        assert!(fs::metadata(marker_path(&root)).expect("metadata").len() <= MARKER_MAX_BYTES);
        assert_eq!(
            ensure_marker(&root).expect("existing"),
            MarkerState::Existing
        );
        assert_eq!(read_marker(&root).expect("read again"), Some(token));
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn unknown_occupied_marker_path_is_never_replaced() {
        let root = std::env::temp_dir().join(format!("marker-occupied-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).expect("root");
        fs::write(marker_path(&root), b"not an app marker\n").expect("occupy");
        assert!(ensure_marker(&root).is_err());
        assert_eq!(
            fs::read(marker_path(&root)).expect("unchanged"),
            b"not an app marker\n"
        );
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn restore_requires_a_valid_token_and_never_replaces_another_marker() {
        let root = std::env::temp_dir().join(format!("marker-restore-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).expect("root");
        assert!(restore_marker(&root, "not-a-marker").is_err());
        let token = format!("{MARKER_PREFIX}{}", Uuid::new_v4());
        assert_eq!(
            restore_marker(&root, &token).expect("restore"),
            MarkerState::Created
        );
        assert_eq!(
            restore_marker(&root, &token).expect("idempotent"),
            MarkerState::Existing
        );
        assert!(restore_marker(&root, &format!("{MARKER_PREFIX}{}", Uuid::new_v4())).is_err());
        fs::remove_dir_all(root).expect("cleanup");
    }
}
