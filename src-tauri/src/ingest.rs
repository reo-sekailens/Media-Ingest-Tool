//! Portable, bounded, verified streaming ingest primitive.

use crate::storage_marker::MARKER_FILE_NAME;
use blake3::Hasher;
use fs2::available_space;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Condvar, Mutex};
use std::thread;
use uuid::Uuid;

#[cfg(unix)]
use std::os::unix::fs::MetadataExt as UnixMetadataExt;
#[cfg(windows)]
use std::os::windows::fs::MetadataExt;

pub const COPY_BUFFER_BYTES: usize = 1024 * 1024;
pub const MAX_SOURCE_DIRECTORY_DEPTH: usize = 64;
pub const MAX_SOURCE_FILE_COUNT: usize = 1_000_000;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CopyProgressStage {
    Copying,
    Verifying,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CopyProgress {
    pub stage: CopyProgressStage,
    pub bytes: u64,
    /// One-based ordinal only; raw source paths never leave the native core.
    pub file_index: usize,
    pub total_files: usize,
}

pub type CopyProgressCallback = Arc<dyn Fn(CopyProgress) + Send + Sync>;
type FileByteProgressCallback = Arc<dyn Fn(CopyProgressStage, u64) + Send + Sync>;

#[derive(Clone)]
pub struct VerificationProgress {
    pub callback: CopyProgressCallback,
    pub file_index: usize,
    pub total_files: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SourceEnumerationLimits {
    pub max_directory_depth: usize,
    pub max_file_count: usize,
}

impl Default for SourceEnumerationLimits {
    fn default() -> Self {
        Self {
            max_directory_depth: MAX_SOURCE_DIRECTORY_DEPTH,
            max_file_count: MAX_SOURCE_FILE_COUNT,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileState {
    Planned,
    Copying,
    CopiedUnverified,
    Verifying,
    ByteVerified,
    Committing,
    Committed,
    Cancelled,
    RetryableError,
    SourceChanged,
    VerificationFailed,
}

pub fn can_transition(from: FileState, to: FileState) -> bool {
    matches!(
        (from, to),
        (FileState::Planned, FileState::Copying)
            | (
                FileState::Copying,
                FileState::CopiedUnverified
                    | FileState::Cancelled
                    | FileState::RetryableError
                    | FileState::SourceChanged
            )
            | (FileState::CopiedUnverified, FileState::Verifying)
            | (
                FileState::Verifying,
                FileState::ByteVerified | FileState::VerificationFailed | FileState::RetryableError
            )
            | (FileState::ByteVerified, FileState::Committing)
            | (
                FileState::Committing,
                FileState::Committed | FileState::RetryableError
            )
            | (
                FileState::RetryableError,
                FileState::Copying | FileState::Verifying
            )
    )
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlannedSourceFile {
    pub relative_path: PathBuf,
    pub byte_length: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlannedCopyFile {
    pub entry_id: String,
    pub source: PlannedSourceFile,
    pub destination_relative_path: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedCopy {
    pub bytes: u64,
    pub digest_hex: String,
    pub entry_id: String,
    pub source_relative_path: PathBuf,
    pub destination_relative_path: PathBuf,
    pub final_path: PathBuf,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IngestReceipt {
    pub schema_version: u8,
    pub manifest_algorithm: String,
    pub manifest_root_blake3: String,
    pub source_medium_key: String,
    pub source_identity_confidence: String,
    pub source_generation: u64,
    pub files: Vec<ReceiptFile>,
}

pub const MANIFEST_ALGORITHM: &str = "blake3:media-ingest-manifest-v1";

/// Domain-separated root over the complete, deterministically ordered set of
/// independently verified copy records. It is a receipt seal, not a shortcut
/// for a future full destination re-verification.
pub fn manifest_root(copies: &[VerifiedCopy]) -> String {
    let mut records = copies
        .iter()
        .map(|copy| {
            (
                copy.source_relative_path
                    .to_string_lossy()
                    .replace('\\', "/"),
                copy.destination_relative_path
                    .to_string_lossy()
                    .replace('\\', "/"),
                copy.bytes,
                copy.digest_hex.clone(),
            )
        })
        .collect::<Vec<_>>();
    records.sort();
    let mut hasher = Hasher::new();
    hasher.update(b"media-ingest-manifest-v1\0");
    for (source, destination, bytes, digest) in records {
        for value in [source.as_bytes(), destination.as_bytes(), digest.as_bytes()] {
            hasher.update(&(value.len() as u64).to_le_bytes());
            hasher.update(value);
        }
        hasher.update(&bytes.to_le_bytes());
    }
    hasher.finalize().to_hex().to_string()
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReceiptFile {
    pub relative_path: String,
    pub bytes: u64,
    pub blake3: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WorkerLimits {
    pub max_workers: usize,
}

impl Default for WorkerLimits {
    fn default() -> Self {
        Self { max_workers: 2 }
    }
}

/// Serializes complete ingest operations targeting the same resolved
/// destination root while leaving independent destinations concurrent. This
/// deliberately prevents combined copy and verification readers from
/// thrashing one volume until per-volume benchmark evidence supports a higher
/// shared concurrency limit.
pub struct DestinationLeaseRegistry {
    active: Mutex<HashSet<String>>,
    available: Condvar,
}

impl Default for DestinationLeaseRegistry {
    fn default() -> Self {
        Self {
            active: Mutex::new(HashSet::new()),
            available: Condvar::new(),
        }
    }
}

pub struct DestinationLease {
    registry: Arc<DestinationLeaseRegistry>,
    key: String,
}

impl DestinationLeaseRegistry {
    pub fn acquire(self: &Arc<Self>, key: String) -> DestinationLease {
        let mut active = self
            .active
            .lock()
            .expect("destination lease registry poisoned");
        while active.contains(&key) {
            active = self
                .available
                .wait(active)
                .expect("destination lease registry poisoned");
        }
        active.insert(key.clone());
        DestinationLease {
            registry: Arc::clone(self),
            key,
        }
    }

    #[cfg(test)]
    fn active_count(&self) -> usize {
        self.active
            .lock()
            .expect("destination lease registry poisoned")
            .len()
    }
}

impl Drop for DestinationLease {
    fn drop(&mut self) {
        if let Ok(mut active) = self.registry.active.lock() {
            active.remove(&self.key);
            self.registry.available.notify_all();
        }
    }
}

#[derive(Debug)]
pub enum IngestError {
    Cancelled,
    InvalidPath,
    DestinationExists,
    SourceChanged,
    UnsafeSourceEntry,
    VerificationFailed,
    InsufficientDestinationSpace,
    SourceLimitExceeded,
    Io(io::Error),
}

impl From<io::Error> for IngestError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

pub fn enumerate_regular_files(root: &Path) -> Result<Vec<PlannedSourceFile>, IngestError> {
    enumerate_regular_files_with_limits(root, SourceEnumerationLimits::default())
}

fn enumerate_regular_files_with_limits(
    root: &Path,
    limits: SourceEnumerationLimits,
) -> Result<Vec<PlannedSourceFile>, IngestError> {
    let canonical_root = root.canonicalize()?;
    let root_metadata = fs::metadata(&canonical_root)?;
    let mut files = Vec::new();
    enumerate_directory(
        &canonical_root,
        &root_metadata,
        &canonical_root,
        0,
        limits,
        &mut files,
    )?;
    files.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    Ok(files)
}

pub fn validate_ingest_roots(
    source_root: &Path,
    destination_root: &Path,
) -> Result<(), IngestError> {
    let source = source_root.canonicalize()?;
    if !source.is_dir() {
        return Err(IngestError::InvalidPath);
    }
    if destination_root.exists() && !destination_root.is_dir() {
        return Err(IngestError::InvalidPath);
    }
    let destination = if destination_root.exists() {
        destination_root.canonicalize()?
    } else {
        let existing = nearest_existing_ancestor(destination_root)?;
        if !existing.is_dir() {
            return Err(IngestError::InvalidPath);
        }
        let suffix = destination_root
            .strip_prefix(existing)
            .map_err(|_| IngestError::InvalidPath)?;
        if suffix
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
        {
            return Err(IngestError::InvalidPath);
        }
        existing.canonicalize()?.join(suffix)
    };
    if destination == source || destination.starts_with(&source) || source.starts_with(&destination)
    {
        return Err(IngestError::InvalidPath);
    }
    Ok(())
}

/// Checks an existing destination (or its nearest existing parent) before a
/// copy plan begins. This is a preflight, not a reservation: concurrent writes
/// can still consume space after the check, so the streaming write path remains
/// authoritative for I/O errors.
pub fn has_destination_space(
    destination_root: &Path,
    required_bytes: u64,
) -> Result<bool, IngestError> {
    let existing = nearest_existing_ancestor(destination_root)?;
    Ok(available_space(existing)? >= required_bytes)
}

/// Creates an app-local scheduling key from the nearest existing destination
/// ancestor without creating any directory. It is a concurrency key, never a
/// device identity or a value exposed to the webview.
pub fn destination_lease_key(destination_root: &Path) -> Result<String, IngestError> {
    let existing = nearest_existing_ancestor(destination_root)?;
    let suffix = destination_root
        .strip_prefix(existing)
        .map_err(|_| IngestError::InvalidPath)?;
    let resolved = existing.canonicalize()?.join(suffix);
    #[cfg(windows)]
    let key = resolved.to_string_lossy().to_lowercase();
    #[cfg(not(windows))]
    let key = resolved.to_string_lossy().into_owned();
    Ok(key)
}

fn nearest_existing_ancestor(path: &Path) -> Result<&Path, IngestError> {
    let mut candidate = path;
    while !candidate.exists() {
        candidate = candidate.parent().ok_or(IngestError::InvalidPath)?;
    }
    Ok(candidate)
}

fn enumerate_directory(
    root: &Path,
    root_metadata: &fs::Metadata,
    directory: &Path,
    directory_depth: usize,
    limits: SourceEnumerationLimits,
    files: &mut Vec<PlannedSourceFile>,
) -> Result<(), IngestError> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let relative_path = entry
            .path()
            .strip_prefix(root)
            .map_err(|_| IngestError::InvalidPath)?
            .to_path_buf();
        // OS-managed root directories are not camera media. They can be
        // inaccessible or change while the card is mounted. Match only exact
        // root names; ordinary dotfiles and identically named nested media
        // remain part of the inventory and managed-content witness.
        if matches!(
            relative_path.to_str(),
            Some("System Volume Information" | ".fseventsd" | ".Spotlight-V100" | ".Trashes")
        ) {
            continue;
        }
        let metadata = fs::symlink_metadata(entry.path())?;
        if is_prohibited_source_entry(&metadata) {
            continue;
        }
        if metadata.is_dir() {
            if source_entry_crosses_mount(root_metadata, &metadata) {
                return Err(IngestError::UnsafeSourceEntry);
            }
            if directory_depth >= limits.max_directory_depth {
                return Err(IngestError::SourceLimitExceeded);
            }
            enumerate_directory(
                root,
                root_metadata,
                &entry.path(),
                directory_depth + 1,
                limits,
                files,
            )?;
        } else if metadata.is_file() {
            // The root marker identifies this source card locally and is not
            // camera media. Never reproduce it in an ingest destination.
            if relative_path == Path::new(MARKER_FILE_NAME) {
                continue;
            }
            if files.len() >= limits.max_file_count {
                return Err(IngestError::SourceLimitExceeded);
            }
            files.push(PlannedSourceFile {
                relative_path,
                byte_length: metadata.len(),
            });
        }
    }
    Ok(())
}

pub fn verified_copy(
    source_root: &Path,
    source: &PlannedSourceFile,
    destination_root: &Path,
    cancel: &AtomicBool,
) -> Result<VerifiedCopy, IngestError> {
    verified_copy_to(
        source_root,
        source,
        &source.relative_path,
        &Uuid::new_v4().to_string(),
        destination_root,
        cancel,
    )
}

pub fn verified_copy_to(
    source_root: &Path,
    source: &PlannedSourceFile,
    destination_relative_path: &Path,
    entry_id: &str,
    destination_root: &Path,
    cancel: &AtomicBool,
) -> Result<VerifiedCopy, IngestError> {
    verified_copy_to_with_progress(
        source_root,
        source,
        destination_relative_path,
        entry_id,
        destination_root,
        cancel,
        None,
    )
}

/// Re-establishes byte-verification evidence for a file that may have been
/// published before an application crash. It never accepts name or size alone:
/// both source and final destination are freshly and independently hashed.
/// Callers use this only after the exact run/device-generation gate succeeds.
pub fn verify_existing_copy(
    source_root: &Path,
    source: &PlannedSourceFile,
    destination_relative_path: &Path,
    entry_id: &str,
    destination_root: &Path,
    cancel: &AtomicBool,
) -> Result<VerifiedCopy, IngestError> {
    verify_existing_copy_with_progress(
        source_root,
        source,
        destination_relative_path,
        entry_id,
        destination_root,
        cancel,
        None,
    )
}

/// Emits only the independent final-file readback. The source rehash remains
/// unreported so verification progress stays bounded by the frozen plan size.
pub fn verify_existing_copy_with_progress(
    source_root: &Path,
    source: &PlannedSourceFile,
    destination_relative_path: &Path,
    entry_id: &str,
    destination_root: &Path,
    cancel: &AtomicBool,
    progress: Option<VerificationProgress>,
) -> Result<VerifiedCopy, IngestError> {
    validate_ingest_roots(source_root, destination_root)?;
    validate_source_relative_path(&source.relative_path)?;
    validate_destination_relative_path(destination_relative_path)?;
    let source_path = source_root.join(&source.relative_path);
    let final_path = destination_root.join(destination_relative_path);
    let root_metadata = fs::metadata(source_root)?;
    let before = fs::symlink_metadata(&source_path)?;
    if is_prohibited_source_entry(&before) || source_entry_crosses_mount(&root_metadata, &before) {
        return Err(IngestError::UnsafeSourceEntry);
    }
    if !before.is_file() || before.len() != source.byte_length {
        return Err(IngestError::SourceChanged);
    }
    let destination_before = fs::symlink_metadata(&final_path)?;
    if !destination_before.is_file()
        || is_prohibited_source_entry(&destination_before)
        || destination_before.len() != source.byte_length
    {
        return Err(IngestError::VerificationFailed);
    }
    let source_digest = hash_file(&source_path, cancel)?;
    let destination_progress = progress.map(|progress| {
        let callback = progress.callback;
        let file_index = progress.file_index;
        let total_files = progress.total_files;
        Arc::new(move |stage, bytes| {
            callback(CopyProgress {
                stage,
                bytes,
                file_index,
                total_files,
            });
        }) as FileByteProgressCallback
    });
    let destination_digest =
        hash_file_with_progress(&final_path, cancel, destination_progress.as_ref())?;
    let after = fs::symlink_metadata(&source_path)?;
    if is_prohibited_source_entry(&after) || source_metadata_changed(&before, &after) {
        return Err(IngestError::SourceChanged);
    }
    if source_digest != destination_digest {
        return Err(IngestError::VerificationFailed);
    }
    // The final path can change after its last readback chunk. Recovery must
    // reject that changed file rather than seal the digest of earlier bytes.
    let destination_after = fs::symlink_metadata(&final_path)?;
    if !destination_after.is_file()
        || is_prohibited_source_entry(&destination_after)
        || source_metadata_changed(&destination_before, &destination_after)
    {
        return Err(IngestError::VerificationFailed);
    }
    Ok(VerifiedCopy {
        bytes: source.byte_length,
        digest_hex: source_digest,
        entry_id: entry_id.into(),
        source_relative_path: source.relative_path.clone(),
        destination_relative_path: destination_relative_path.to_path_buf(),
        final_path,
    })
}

fn verified_copy_to_with_progress(
    source_root: &Path,
    source: &PlannedSourceFile,
    destination_relative_path: &Path,
    entry_id: &str,
    destination_root: &Path,
    cancel: &AtomicBool,
    progress: Option<&FileByteProgressCallback>,
) -> Result<VerifiedCopy, IngestError> {
    // This guard must live at the primitive boundary, not only in a planner:
    // callers that bypass a UI flow must never recursively ingest into the
    // source tree (or overwrite it through a parent destination).
    validate_ingest_roots(source_root, destination_root)?;
    validate_source_relative_path(&source.relative_path)?;
    validate_destination_relative_path(destination_relative_path)?;
    let source_path = source_root.join(&source.relative_path);
    let root_metadata = fs::metadata(source_root)?;
    let before = fs::symlink_metadata(&source_path)?;
    if is_prohibited_source_entry(&before) || source_entry_crosses_mount(&root_metadata, &before) {
        return Err(IngestError::UnsafeSourceEntry);
    }
    if !before.is_file() || before.len() != source.byte_length {
        return Err(IngestError::SourceChanged);
    }
    let final_path = destination_root.join(destination_relative_path);
    if final_path.exists() {
        return Err(IngestError::DestinationExists);
    }
    let parent = final_path.parent().ok_or(IngestError::InvalidPath)?;
    fs::create_dir_all(parent)?;
    // A stable, entry-scoped checkpoint name lets an explicit recovery own
    // the interrupted partial without trusting a path supplied by the UI.
    // Hashing the opaque entry ID keeps arbitrary caller text out of the
    // filesystem namespace and prevents separator/path traversal tricks.
    let temporary_path = partial_path(parent, entry_id);
    discard_recoverable_partial(&temporary_path)?;
    let result = copy_and_verify(
        &source_path,
        &temporary_path,
        source.byte_length,
        cancel,
        progress,
    );
    match result {
        Ok((bytes, digest_hex)) => {
            let after = fs::symlink_metadata(&source_path)?;
            if is_prohibited_source_entry(&after) {
                let _ = fs::remove_file(&temporary_path);
                return Err(IngestError::UnsafeSourceEntry);
            }
            if source_metadata_changed(&before, &after) {
                let _ = fs::remove_file(&temporary_path);
                return Err(IngestError::SourceChanged);
            }
            publish_verified_file(&temporary_path, &final_path)?;
            // The independent digest was computed on the closed temporary
            // file. Reopen and flush the published path before checking it:
            // the same-directory rename is atomic on the supported local
            // filesystem boundary, but a fresh final-path stat still catches
            // a failed/replaced publication before this file can be admitted
            // to the manifest.
            sync_final_file(&final_path)?;
            if !destination_metadata_matches(&final_path, bytes)? {
                return Err(IngestError::VerificationFailed);
            }
            Ok(VerifiedCopy {
                bytes,
                digest_hex,
                entry_id: entry_id.into(),
                source_relative_path: source.relative_path.clone(),
                destination_relative_path: destination_relative_path.to_path_buf(),
                final_path,
            })
        }
        // Keep the flushed partial for a durable, operator-visible recovery
        // checkpoint. It is never considered transferred, and the exact
        // frozen-plan recovery path discards it before a fresh verified copy.
        Err(error) => Err(error),
    }
}

fn partial_path(parent: &Path, entry_id: &str) -> PathBuf {
    parent.join(format!(
        ".ingest-{}.partial",
        blake3::hash(entry_id.as_bytes()).to_hex()
    ))
}

/// macOS rename() replaces an existing destination. The existence check at
/// planning time cannot prevent another process (or a case/Unicode alias) from
/// creating that destination before publication. RENAME_EXCL makes the refusal
/// atomic and preserves the verified partial on a collision. Unsupported
/// filesystems return an error; never fall back to a replacing rename.
#[cfg(target_os = "macos")]
fn publish_verified_file(temporary: &Path, destination: &Path) -> Result<(), IngestError> {
    use std::ffi::CString;
    use std::os::raw::{c_char, c_int, c_uint};
    use std::os::unix::ffi::OsStrExt;

    extern "C" {
        fn renamex_np(from: *const c_char, to: *const c_char, flags: c_uint) -> c_int;
    }
    // Apple's <sys/stdio.h>; available since macOS 10.12.
    const RENAME_EXCL: c_uint = 0x0000_0004;
    let from =
        CString::new(temporary.as_os_str().as_bytes()).map_err(|_| IngestError::InvalidPath)?;
    let to =
        CString::new(destination.as_os_str().as_bytes()).map_err(|_| IngestError::InvalidPath)?;
    // SAFETY: both NUL-terminated path buffers outlive this synchronous call;
    // the signature and flag match Apple's header on Intel and ARM64.
    if unsafe { renamex_np(from.as_ptr(), to.as_ptr(), RENAME_EXCL) } == 0 {
        return Ok(());
    }
    let error = io::Error::last_os_error();
    if error.kind() == io::ErrorKind::AlreadyExists {
        Err(IngestError::DestinationExists)
    } else {
        Err(IngestError::Io(error))
    }
}

#[cfg(not(target_os = "macos"))]
fn publish_verified_file(temporary: &Path, destination: &Path) -> Result<(), IngestError> {
    fs::rename(temporary, destination)?;
    Ok(())
}

/// Removes only the deterministic checkpoint owned by the frozen plan entry.
/// A symlink, directory, or other replacement is never followed or removed.
fn discard_recoverable_partial(path: &Path) -> Result<(), IngestError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() && !metadata.file_type().is_symlink() => {
            fs::remove_file(path)?;
            Ok(())
        }
        Ok(_) => Err(IngestError::InvalidPath),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(IngestError::Io(error)),
    }
}

/// Length is always mandatory. Modification time is used when the filesystem
/// exposes it; an unavailable timestamp is intentionally not synthesized as a
/// stability guarantee.
fn source_metadata_changed(before: &fs::Metadata, after: &fs::Metadata) -> bool {
    before.len() != after.len()
        || matches!(
            (before.modified(), after.modified()),
            (Ok(before_modified), Ok(after_modified)) if before_modified != after_modified
        )
}

/// On Unix, a different `st_dev` means an in-tree mount point and must not be
/// followed out of the selected source filesystem. Windows mount points are
/// reparse points and are rejected by `is_prohibited_source_entry` instead.
#[cfg(unix)]
fn source_entry_crosses_mount(root: &fs::Metadata, entry: &fs::Metadata) -> bool {
    !source_filesystem_matches(root.dev(), entry.dev())
}

#[cfg(not(unix))]
fn source_entry_crosses_mount(_root: &fs::Metadata, _entry: &fs::Metadata) -> bool {
    false
}

#[cfg(any(unix, test))]
fn source_filesystem_matches(root_device: u64, entry_device: u64) -> bool {
    root_device == entry_device
}

fn destination_metadata_matches(path: &Path, expected_bytes: u64) -> Result<bool, IngestError> {
    let metadata = fs::symlink_metadata(path)?;
    Ok(metadata.is_file()
        && !is_prohibited_source_entry(&metadata)
        && metadata.len() == expected_bytes)
}

fn sync_final_file(path: &Path) -> Result<(), IngestError> {
    // Windows requires a writable handle for FlushFileBuffers; the ingest
    // already owns this newly published file and must treat an inability to
    // reopen it that way as a failed verified ingest.
    OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)?
        .sync_all()?;
    Ok(())
}

/// Junctions and other Windows reparse points are not ordinary directories or
/// files even when they are not reported as POSIX-like symlinks. They are
/// excluded from both inventory and post-copy source validation.
#[cfg(windows)]
fn is_prohibited_source_entry(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_symlink() || metadata.file_attributes() & 0x0400 != 0
}

#[cfg(not(windows))]
fn is_prohibited_source_entry(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

/// Bounded parallelism for independent files. Each worker owns a blocking I/O
/// loop; no task is created per chunk and no worker count exceeds the caller's
/// policy. Higher-level scheduling supplies per-card/destination limits.
pub fn verified_copy_batch(
    source_root: &Path,
    files: Vec<PlannedSourceFile>,
    destination_root: &Path,
    limits: WorkerLimits,
    cancel: Arc<AtomicBool>,
) -> Vec<Result<VerifiedCopy, IngestError>> {
    verified_copy_batch_planned(
        source_root,
        files
            .into_iter()
            .map(|source| PlannedCopyFile {
                entry_id: Uuid::new_v4().to_string(),
                destination_relative_path: source.relative_path.clone(),
                source,
            })
            .collect(),
        destination_root,
        limits,
        cancel,
    )
}

pub fn verified_copy_batch_planned(
    source_root: &Path,
    files: Vec<PlannedCopyFile>,
    destination_root: &Path,
    limits: WorkerLimits,
    cancel: Arc<AtomicBool>,
) -> Vec<Result<VerifiedCopy, IngestError>> {
    verified_copy_batch_planned_with_progress(
        source_root,
        files,
        destination_root,
        limits,
        cancel,
        None,
    )
}

pub fn verified_copy_batch_planned_with_progress(
    source_root: &Path,
    files: Vec<PlannedCopyFile>,
    destination_root: &Path,
    limits: WorkerLimits,
    cancel: Arc<AtomicBool>,
    progress: Option<CopyProgressCallback>,
) -> Vec<Result<VerifiedCopy, IngestError>> {
    let total_files = files.len();
    verified_copy_batch_planned_with_progress_positions(
        source_root,
        files
            .into_iter()
            .enumerate()
            .map(|(index, file)| (index + 1, file))
            .collect(),
        total_files,
        destination_root,
        limits,
        cancel,
        progress,
    )
}

/// Like [`verified_copy_batch_planned_with_progress`], but preserves the
/// original frozen-plan ordinal when a recovery copies only a subset of files.
pub fn verified_copy_batch_planned_with_progress_positions(
    source_root: &Path,
    files: Vec<(usize, PlannedCopyFile)>,
    total_files: usize,
    destination_root: &Path,
    limits: WorkerLimits,
    cancel: Arc<AtomicBool>,
    progress: Option<CopyProgressCallback>,
) -> Vec<Result<VerifiedCopy, IngestError>> {
    if files.is_empty() {
        return Vec::new();
    }
    let job_total = files.len();
    let worker_count = limits.max_workers.clamp(1, job_total);
    let (job_tx, job_rx) = mpsc::channel::<(usize, usize, PlannedCopyFile)>();
    let (result_tx, result_rx) = mpsc::channel();
    let job_rx = Arc::new(Mutex::new(job_rx));
    let source_root = Arc::new(source_root.to_path_buf());
    let destination_root = Arc::new(destination_root.to_path_buf());
    let mut workers = Vec::with_capacity(worker_count);
    for _ in 0..worker_count {
        let job_rx = Arc::clone(&job_rx);
        let result_tx = result_tx.clone();
        let source_root = Arc::clone(&source_root);
        let destination_root = Arc::clone(&destination_root);
        let cancel = Arc::clone(&cancel);
        let progress = progress.clone();
        workers.push(thread::spawn(move || loop {
            let job = job_rx.lock().expect("job queue poisoned").recv();
            let Ok((result_index, file_index, file)) = job else {
                break;
            };
            let file_progress = progress.as_ref().map(|progress| {
                let progress = Arc::clone(progress);
                Arc::new(move |stage, bytes| {
                    progress(CopyProgress {
                        stage,
                        bytes,
                        file_index,
                        total_files,
                    });
                }) as FileByteProgressCallback
            });
            let result = verified_copy_to_with_progress(
                &source_root,
                &file.source,
                &file.destination_relative_path,
                &file.entry_id,
                &destination_root,
                &cancel,
                file_progress.as_ref(),
            );
            if result_tx.send((result_index, result)).is_err() {
                break;
            }
        }));
    }
    for (result_index, (file_index, file)) in files.into_iter().enumerate() {
        let _ = job_tx.send((result_index, file_index, file));
    }
    drop(job_tx);
    drop(result_tx);
    let mut outcomes = (0..job_total)
        .map(|_| None)
        .collect::<Vec<Option<Result<VerifiedCopy, IngestError>>>>();
    for _ in 0..job_total {
        if let Ok((index, outcome)) = result_rx.recv() {
            outcomes[index] = Some(outcome);
        }
    }
    for worker in workers {
        let _ = worker.join();
    }
    outcomes
        .into_iter()
        .map(|outcome| outcome.unwrap_or(Err(IngestError::Cancelled)))
        .collect()
}

/// Writes an immutable JSON projection after byte verification. The SQLite
/// manifest remains authoritative; this is an operator-readable handoff.
pub fn write_receipt(path: &Path, receipt: &IngestReceipt) -> Result<(), IngestError> {
    // Receipts are an immutable projection of a completed run.  A collision is
    // therefore an operator-visible error rather than a platform-dependent
    // replacement (rename replaces on some Unix filesystems).
    if path.exists() {
        return Err(IngestError::DestinationExists);
    }
    let parent = path.parent().ok_or(IngestError::InvalidPath)?;
    fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(".receipt-{}.partial", Uuid::new_v4()));
    let bytes = serde_json::to_vec_pretty(receipt)
        .map_err(|error| IngestError::Io(io::Error::other(error)))?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    drop(file);
    publish_verified_file(&temporary, path)?;
    Ok(())
}

fn copy_and_verify(
    source_path: &Path,
    temporary_path: &Path,
    expected_bytes: u64,
    cancel: &AtomicBool,
    progress: Option<&FileByteProgressCallback>,
) -> Result<(u64, String), IngestError> {
    let mut source = File::open(source_path)?;
    let mut temporary = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(temporary_path)?;
    let mut buffer = vec![0_u8; COPY_BUFFER_BYTES];
    let mut source_hasher = Hasher::new();
    let mut copied = 0_u64;
    loop {
        if cancel.load(Ordering::Relaxed) {
            // The partial may be reused only as durable diagnostic evidence;
            // recovery will delete it and copy the frozen entry from scratch.
            temporary.sync_all()?;
            return Err(IngestError::Cancelled);
        }
        let read = source.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        source_hasher.update(&buffer[..read]);
        temporary.write_all(&buffer[..read])?;
        copied += read as u64;
        if let Some(progress) = progress {
            progress(CopyProgressStage::Copying, read as u64);
        }
    }
    if copied != expected_bytes {
        return Err(IngestError::SourceChanged);
    }
    temporary.sync_all()?;
    drop(temporary);
    let destination_digest = hash_file_with_progress(temporary_path, cancel, progress)?;
    let source_digest = source_hasher.finalize().to_hex().to_string();
    if destination_digest != source_digest {
        return Err(IngestError::VerificationFailed);
    }
    Ok((copied, source_digest))
}

fn hash_file(path: &Path, cancel: &AtomicBool) -> Result<String, IngestError> {
    hash_file_with_progress(path, cancel, None)
}

fn hash_file_with_progress(
    path: &Path,
    cancel: &AtomicBool,
    progress: Option<&FileByteProgressCallback>,
) -> Result<String, IngestError> {
    let mut file = File::open(path)?;
    let mut buffer = vec![0_u8; COPY_BUFFER_BYTES];
    let mut hasher = Hasher::new();
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(IngestError::Cancelled);
        }
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        if let Some(progress) = progress {
            progress(CopyProgressStage::Verifying, read as u64);
        }
    }
    Ok(hasher.finalize().to_hex().to_string())
}

fn validate_source_relative_path(path: &Path) -> Result<(), IngestError> {
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(IngestError::InvalidPath);
    }
    Ok(())
}

fn validate_destination_relative_path(path: &Path) -> Result<(), IngestError> {
    if !crate::organization::is_portable_destination_relative_path(path) {
        return Err(IngestError::InvalidPath);
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicU64};

    /// One uncompressed grayscale pixel in a baseline little-endian TIFF,
    /// with a real Exif sub-IFD rather than mocked metadata-reader results.
    pub(crate) fn exif_hour_range_tiff(capture_time: &str, pixel: u8) -> Vec<u8> {
        exif_hour_range_tiff_with_offset(capture_time, Some("+08:00"), pixel)
    }

    pub(crate) fn exif_hour_range_tiff_with_offset(
        capture_time: &str,
        embedded_offset: Option<&str>,
        pixel: u8,
    ) -> Vec<u8> {
        fn append_ascii(bytes: &mut Vec<u8>, value: &str) -> (u32, u32) {
            if !bytes.len().is_multiple_of(2) {
                bytes.push(0);
            }
            let offset = bytes.len() as u32;
            bytes.extend_from_slice(value.as_bytes());
            bytes.push(0);
            ((value.len() + 1) as u32, offset)
        }

        fn write_ifd(bytes: &mut [u8], offset: usize, entries: &[(u16, u16, u32, u32)]) {
            bytes[offset..offset + 2].copy_from_slice(&(entries.len() as u16).to_le_bytes());
            for (index, (tag, field_type, count, value)) in entries.iter().enumerate() {
                let start = offset + 2 + index * 12;
                bytes[start..start + 2].copy_from_slice(&tag.to_le_bytes());
                bytes[start + 2..start + 4].copy_from_slice(&field_type.to_le_bytes());
                bytes[start + 4..start + 8].copy_from_slice(&count.to_le_bytes());
                bytes[start + 8..start + 12].copy_from_slice(&value.to_le_bytes());
            }
            // The zero-filled final four bytes terminate the IFD chain.
        }

        let image_ifd_offset = 8;
        let exif_ifd_offset = image_ifd_offset + 2 + 15 * 12 + 4;
        let data_offset = exif_ifd_offset + 2 + 6 * 12 + 4;
        let mut bytes = vec![0; data_offset];
        bytes[..8].copy_from_slice(b"II*\0\x08\0\0\0");
        let (make_length, make_offset) = append_ascii(&mut bytes, "Sony");
        let (capture_length, capture_offset) = append_ascii(&mut bytes, capture_time);
        let original_offset = embedded_offset.map(|offset| append_ascii(&mut bytes, offset));
        let (serial_length, serial_offset) = append_ascii(&mut bytes, "hour-range-camera-01");
        if !bytes.len().is_multiple_of(2) {
            bytes.push(0);
        }
        let resolution_offset = bytes.len() as u32;
        bytes.extend_from_slice(&72_u32.to_le_bytes());
        bytes.extend_from_slice(&1_u32.to_le_bytes());
        let pixel_offset = bytes.len() as u32;
        bytes.push(pixel);
        write_ifd(
            &mut bytes,
            image_ifd_offset,
            &[
                (0x0100, 4, 1, 1), // ImageWidth
                (0x0101, 4, 1, 1), // ImageLength
                (0x0102, 3, 1, 8), // BitsPerSample
                (0x0103, 3, 1, 1), // Compression: uncompressed
                (0x0106, 3, 1, 1), // PhotometricInterpretation: black is zero
                (0x010f, 2, make_length, make_offset),
                (0x0110, 2, 4, u32::from_le_bytes(*b"FX3\0")),
                (0x0111, 4, 1, pixel_offset), // StripOffsets
                (0x0115, 3, 1, 1),            // SamplesPerPixel
                (0x0116, 4, 1, 1),            // RowsPerStrip
                (0x0117, 4, 1, 1),            // StripByteCounts
                (0x011a, 5, 1, resolution_offset),
                (0x011b, 5, 1, resolution_offset),
                (0x0128, 3, 1, 2), // ResolutionUnit: inch
                (0x8769, 4, 1, exif_ifd_offset as u32),
            ],
        );
        let mut exif_entries = vec![
            (0x9000, 7, 4, u32::from_le_bytes(*b"0231")),
            (0x9003, 2, capture_length, capture_offset),
        ];
        if let Some((length, offset)) = original_offset {
            exif_entries.push((0x9011, 2, length, offset));
        }
        exif_entries.extend([
            (0xa000, 7, 4, u32::from_le_bytes(*b"0100")),
            (0xa001, 3, 1, 1), // ColorSpace: sRGB
            (0xa431, 2, serial_length, serial_offset),
        ]);
        write_ifd(&mut bytes, exif_ifd_offset, &exif_entries);
        bytes
    }

    #[test]
    fn exif_capture_hour_ranges_copy_real_media_and_seal_matching_receipt() {
        use crate::metadata::{inspect, CaptureTimeSource};
        use crate::{
            execute_prepared_ingest, prepare_verified_ingest, IdentityConfidence, IngestSortMode,
            VerifiedIngestRequest,
        };
        use std::collections::BTreeMap;
        use std::time::{Duration, UNIX_EPOCH};

        let root = std::env::temp_dir().join(format!("ingest-exif-hour-ranges-{}", Uuid::new_v4()));
        let source_root = root.join("source");
        let destination_root = root.join("destination");
        fs::create_dir_all(source_root.join("DCIM")).expect("fixture source");
        let fixtures = [
            (
                "A001.TIF",
                "2026:10:13 12:59:59",
                "20261013T000000+0800",
                "1200-1300+0800",
            ),
            (
                "A002.TIF",
                "2026:10:13 13:00:00",
                "20261013T000000+0800",
                "1300-1400+0800",
            ),
            (
                "A003.TIF",
                "2026:10:13 13:42:00",
                "20261013T000000+0800",
                "1300-1400+0800",
            ),
            (
                "A004.TIF",
                "2026:10:13 13:59:59",
                "20261013T000000+0800",
                "1300-1400+0800",
            ),
            (
                "A005.TIF",
                "2026:10:13 14:00:00",
                "20261013T000000+0800",
                "1400-1500+0800",
            ),
            (
                "A006.TIF",
                "2026:10:13 23:59:00",
                "20261013T000000+0800",
                "2300-2400+0800",
            ),
            (
                "A007.TIF",
                "2026:10:14 00:00:00",
                "20261014T000000+0800",
                "0000-0100+0800",
            ),
        ];
        // A different year/day/hour ensures filesystem fallback cannot satisfy
        // the bucket assertions accidentally, regardless of the test host date.
        let modified_time = UNIX_EPOCH + Duration::from_secs(1_577_923_200);
        let mut source_bytes = BTreeMap::new();
        for (index, (name, capture, _, _)) in fixtures.iter().enumerate() {
            let relative = PathBuf::from("DCIM").join(name);
            let path = source_root.join(&relative);
            let bytes = exif_hour_range_tiff(capture, index as u8);
            fs::write(&path, &bytes).expect("valid TIFF fixture");
            OpenOptions::new()
                .write(true)
                .open(&path)
                .expect("fixture modification handle")
                .set_times(fs::FileTimes::new().set_modified(modified_time))
                .expect("deliberately different modification time");
            let metadata = inspect(&path).expect("read capture metadata");
            assert_eq!(metadata.make, "Sony");
            assert_eq!(metadata.model, "FX3");
            assert_eq!(
                metadata.body_serial.as_deref(),
                Some("hour-range-camera-01")
            );
            assert_eq!(
                metadata.capture_time_source,
                CaptureTimeSource::ExifOriginalWithOffset
            );
            assert!(metadata.capture_offset_known);
            assert!(metadata.warning.is_none());
            assert_eq!(
                metadata.capture_time.offset().local_minus_utc(),
                8 * 60 * 60
            );
            assert_eq!(
                metadata
                    .capture_time
                    .format("%Y:%m:%d %H:%M:%S")
                    .to_string(),
                *capture
            );
            assert_ne!(
                metadata.capture_time.timestamp(),
                chrono::DateTime::<chrono::Utc>::from(modified_time).timestamp(),
            );
            source_bytes.insert(relative, bytes);
        }

        let operation_id = Uuid::new_v4().to_string();
        let request = VerifiedIngestRequest {
            operation_id: Some(operation_id.clone()),
            source_root: source_root.to_string_lossy().into(),
            destination_root: destination_root.to_string_lossy().into(),
            source_medium_key: "session:exif-hour-range-fixture".into(),
            source_identity_confidence: IdentityConfidence::Unresolved,
            source_generation: 1,
            max_workers: 2,
            sort_mode: IngestSortMode::CameraInterval,
            interval_minutes: Some(60),
            capture_time_basis: crate::organization::CaptureTimeBasis::CameraRecorded,
            custom_directory_fields: Vec::new(),
            destination_depth_order: None,
            auto_ingest_triggered: false,
        };
        // The read-only UI preview and ingest both use this production planner.
        // Re-plan the same request rather than fabricating destination paths.
        let preview =
            prepare_verified_ingest(request.clone(), operation_id.clone()).expect("preview plan");
        let preview_paths = preview
            .files
            .iter()
            .map(|file| file.destination_relative_path.clone())
            .collect::<Vec<_>>();
        assert!(
            !destination_root.exists(),
            "planning must not copy any media"
        );
        let prepared = prepare_verified_ingest(request, operation_id).expect("ingest plan");
        assert_eq!(prepared.files.len(), fixtures.len());
        assert_eq!(
            prepared
                .files
                .iter()
                .map(|file| file.destination_relative_path.clone())
                .collect::<Vec<_>>(),
            preview_paths,
        );
        let expected_bytes = source_bytes
            .values()
            .map(|bytes| bytes.len() as u64)
            .sum::<u64>();
        let progress_updates = Arc::new(Mutex::new(Vec::new()));
        let observed_progress = Arc::clone(&progress_updates);
        let progress: CopyProgressCallback = Arc::new(move |update| {
            observed_progress
                .lock()
                .expect("progress updates")
                .push(update);
        });
        let completed =
            execute_prepared_ingest(prepared, Arc::new(AtomicBool::new(false)), Some(progress))
                .expect("production verified copy and receipt");
        assert_eq!(completed.summary.copied_files, fixtures.len());
        assert_eq!(completed.summary.copied_bytes, expected_bytes);
        assert_eq!(
            completed
                .copies
                .iter()
                .map(|copy| copy.destination_relative_path.clone())
                .collect::<Vec<_>>(),
            preview_paths
        );
        let camera_folder = completed.copies[0]
            .destination_relative_path
            .components()
            .next()
            .expect("camera folder")
            .as_os_str()
            .to_owned();
        assert!(camera_folder.to_string_lossy().starts_with("FX3__"));
        let mut expected_groups = BTreeMap::<PathBuf, Vec<String>>::new();
        for (name, _, day, hour) in fixtures {
            let relative_source = PathBuf::from("DCIM").join(name);
            let expected_relative = PathBuf::from(&camera_folder)
                .join(day)
                .join(hour)
                .join(name);
            let copy = completed
                .copies
                .iter()
                .find(|copy| copy.source_relative_path == relative_source)
                .expect("copied fixture");
            assert_eq!(copy.destination_relative_path, expected_relative);
            assert_eq!(copy.final_path, destination_root.join(&expected_relative));
            let original = source_bytes.get(&relative_source).expect("fixture bytes");
            assert_eq!(
                fs::read(&copy.final_path).expect("fresh destination readback"),
                *original
            );
            assert_eq!(
                fs::read(source_root.join(&relative_source)).expect("unchanged source"),
                *original
            );
            assert_eq!(
                fs::metadata(source_root.join(&relative_source))
                    .expect("source metadata")
                    .modified()
                    .expect("source modification time"),
                modified_time
            );
            assert_eq!(copy.digest_hex, blake3::hash(original).to_hex().to_string());
            expected_groups
                .entry(
                    expected_relative
                        .parent()
                        .expect("hour bucket")
                        .to_path_buf(),
                )
                .or_default()
                .push(name.into());
        }
        for (group, mut expected_names) in expected_groups {
            let mut actual_names = fs::read_dir(destination_root.join(group))
                .expect("actual hour directory")
                .map(|entry| {
                    entry
                        .expect("media entry")
                        .file_name()
                        .to_string_lossy()
                        .into_owned()
                })
                .collect::<Vec<_>>();
            actual_names.sort();
            expected_names.sort();
            assert_eq!(
                actual_names, expected_names,
                "same-hour media must share one bucket"
            );
        }
        let updates = progress_updates.lock().expect("copy/readback progress");
        for stage in [CopyProgressStage::Copying, CopyProgressStage::Verifying] {
            assert_eq!(
                updates
                    .iter()
                    .filter(|update| update.stage == stage)
                    .map(|update| update.bytes)
                    .sum::<u64>(),
                expected_bytes
            );
        }
        drop(updates);

        let receipt_path = destination_root
            .join(".media-ingest-receipts")
            .join(&completed.summary.receipt_name);
        let receipt: IngestReceipt =
            serde_json::from_slice(&fs::read(receipt_path).expect("published receipt"))
                .expect("receipt JSON");
        assert_eq!(receipt.schema_version, 1);
        assert_eq!(receipt.manifest_algorithm, MANIFEST_ALGORITHM);
        assert_eq!(
            receipt.manifest_root_blake3,
            manifest_root(&completed.copies)
        );
        assert_eq!(receipt.manifest_root_blake3, completed.manifest_root_blake3);
        assert_eq!(receipt.source_medium_key, "session:exif-hour-range-fixture");
        assert_eq!(receipt.source_identity_confidence, "unresolved");
        assert_eq!(receipt.source_generation, 1);
        assert_eq!(receipt.files.len(), fixtures.len());
        assert_eq!(
            receipt.files.iter().map(|file| file.bytes).sum::<u64>(),
            expected_bytes
        );
        for copy in &completed.copies {
            let relative = copy
                .destination_relative_path
                .to_string_lossy()
                .replace('\\', "/");
            let entry = receipt
                .files
                .iter()
                .find(|file| file.relative_path == relative)
                .expect("receipt entry");
            assert_eq!(entry.bytes, copy.bytes);
            assert_eq!(
                entry.blake3,
                blake3::hash(&fs::read(&copy.final_path).expect("receipt independent readback"))
                    .to_hex()
                    .to_string()
            );
        }
        assert_eq!(
            enumerate_regular_files(&source_root)
                .expect("source inventory after ingest")
                .len(),
            fixtures.len()
        );
        assert!(!source_root.join(MARKER_FILE_NAME).exists());
        assert_eq!(
            enumerate_regular_files(&destination_root)
                .expect("destination inventory")
                .len(),
            fixtures.len() + 1
        );
        fs::remove_dir_all(root).expect("temporary fixture cleanup");
    }

    #[test]
    fn selected_time_basis_copies_exif_media_across_offsets_and_calendar_boundaries() {
        use crate::metadata::{inspect, CaptureTimeSource};
        use crate::organization::CaptureTimeBasis;
        use crate::{
            execute_prepared_ingest, prepare_verified_ingest, IdentityConfidence, IngestSortMode,
            VerifiedIngestRequest,
        };
        use chrono::{FixedOffset, NaiveDateTime, TimeZone, Timelike};
        use std::collections::BTreeMap;
        use std::time::{Duration, UNIX_EPOCH};

        let captures = [
            ("2026:12:31 23:30:00", Some("+14:00")),
            ("2026:12:31 23:30:00", Some("-12:00")),
            ("2026:12:31 13:42:00", Some("+05:45")),
            ("2026:12:31 13:42:00", Some("+08:45")),
            ("2026:12:31 13:42:00", Some("-03:30")),
            ("2026:12:31 13:42:00", Some("-09:30")),
            ("2026:12:31 00:30:00", Some("+12:45")),
            ("2026:12:31 13:42:00", None),
        ];
        for basis in [
            CaptureTimeBasis::CameraRecorded,
            CaptureTimeBasis::ComputerLocal,
        ] {
            let root = std::env::temp_dir().join(format!("ingest-time-basis-{}", Uuid::new_v4()));
            let source = root.join("source");
            let destination = root.join("destination");
            fs::create_dir_all(source.join("DCIM")).expect("source fixture");
            let modified = UNIX_EPOCH + Duration::from_secs(1_577_923_200);
            let mut expected = BTreeMap::new();
            for (index, (recorded, offset)) in captures.iter().enumerate() {
                let relative = PathBuf::from("DCIM").join(format!("ZONE{index:02}.TIF"));
                let bytes = exif_hour_range_tiff_with_offset(recorded, *offset, index as u8);
                let path = source.join(&relative);
                fs::write(&path, &bytes).expect("EXIF fixture");
                OpenOptions::new()
                    .write(true)
                    .open(&path)
                    .expect("fixture handle")
                    .set_times(fs::FileTimes::new().set_modified(modified))
                    .expect("source clock fixture");
                let wall_clock = NaiveDateTime::parse_from_str(recorded, "%Y:%m:%d %H:%M:%S")
                    .expect("recorded wall clock");
                let camera_offset = offset.map_or(FixedOffset::east_opt(0).unwrap(), |value| {
                    let seconds = (value[1..3].parse::<i32>().unwrap() * 60
                        + value[4..6].parse::<i32>().unwrap())
                        * 60;
                    FixedOffset::east_opt(if value.starts_with('-') {
                        -seconds
                    } else {
                        seconds
                    })
                    .unwrap()
                });
                let capture = camera_offset
                    .from_local_datetime(&wall_clock)
                    .single()
                    .expect("fixed-offset capture");
                let projected = if basis == CaptureTimeBasis::ComputerLocal && offset.is_some() {
                    capture.with_timezone(&chrono::Local).fixed_offset()
                } else {
                    capture
                };
                let suffix = if offset.is_some() {
                    projected.format("%z").to_string()
                } else {
                    "-offset-unknown".into()
                };
                let day = format!("{}T000000{suffix}", projected.format("%Y%m%d"));
                let hour = format!(
                    "{:02}00-{:02}00{suffix}",
                    projected.hour(),
                    projected.hour() + 1
                );
                let metadata = inspect(&path).expect("real metadata read");
                assert_eq!(metadata.capture_offset_known, offset.is_some());
                assert_eq!(metadata.capture_time.naive_local(), wall_clock);
                assert_eq!(
                    metadata.capture_time_source,
                    if offset.is_some() {
                        CaptureTimeSource::ExifOriginalWithOffset
                    } else {
                        CaptureTimeSource::ExifOriginalWithoutOffset
                    }
                );
                expected.insert(relative, (bytes, day, hour));
            }
            let prepared = prepare_verified_ingest(
                VerifiedIngestRequest {
                    operation_id: None,
                    source_root: source.to_string_lossy().into(),
                    destination_root: destination.to_string_lossy().into(),
                    source_medium_key: "session:time-basis-fixture".into(),
                    source_identity_confidence: IdentityConfidence::Unresolved,
                    source_generation: 1,
                    max_workers: 2,
                    sort_mode: IngestSortMode::CameraInterval,
                    interval_minutes: Some(60),
                    capture_time_basis: basis,
                    custom_directory_fields: Vec::new(),
                    destination_depth_order: None,
                    auto_ingest_triggered: false,
                },
                Uuid::new_v4().to_string(),
            )
            .expect("production time-basis plan");
            let planned = prepared
                .files
                .iter()
                .map(|file| file.destination_relative_path.clone())
                .collect::<Vec<_>>();
            let completed =
                execute_prepared_ingest(prepared, Arc::new(AtomicBool::new(false)), None)
                    .expect("verified time-basis copy");
            assert_eq!(completed.copies.len(), captures.len());
            assert_eq!(
                completed
                    .copies
                    .iter()
                    .map(|file| file.destination_relative_path.clone())
                    .collect::<Vec<_>>(),
                planned
            );
            for copy in &completed.copies {
                let (bytes, day, hour) = expected
                    .get(&copy.source_relative_path)
                    .expect("source expectation");
                let components = copy
                    .destination_relative_path
                    .components()
                    .map(|component| component.as_os_str().to_string_lossy().into_owned())
                    .collect::<Vec<_>>();
                assert!(components[0].starts_with("FX3__"));
                assert_eq!(&components[1], day);
                assert_eq!(&components[2], hour);
                assert_eq!(
                    fs::read(&copy.final_path).expect("destination readback"),
                    *bytes
                );
                assert_eq!(copy.digest_hex, blake3::hash(bytes).to_hex().to_string());
                assert_eq!(
                    fs::read(source.join(&copy.source_relative_path)).expect("source preserved"),
                    *bytes
                );
                assert_eq!(
                    fs::metadata(source.join(&copy.source_relative_path))
                        .unwrap()
                        .modified()
                        .unwrap(),
                    modified
                );
            }
            let receipt: IngestReceipt = serde_json::from_slice(
                &fs::read(
                    destination
                        .join(".media-ingest-receipts")
                        .join(&completed.summary.receipt_name),
                )
                .expect("receipt bytes"),
            )
            .expect("receipt JSON");
            assert_eq!(receipt.files.len(), captures.len());
            assert_eq!(
                receipt.manifest_root_blake3,
                manifest_root(&completed.copies)
            );
            fs::remove_dir_all(root).expect("controlled fixture cleanup");
        }
    }

    #[test]
    fn source_mount_guard_requires_the_same_filesystem_identity() {
        assert!(source_filesystem_matches(12, 12));
        assert!(!source_filesystem_matches(12, 13));
    }

    #[test]
    fn destination_leases_share_only_the_exact_resolved_destination_key() {
        let registry = Arc::new(DestinationLeaseRegistry::default());
        let first = registry.acquire("destination-a".into());
        let second = registry.acquire("destination-b".into());
        assert_eq!(registry.active_count(), 2);
        drop(first);
        assert_eq!(registry.active_count(), 1);
        drop(second);
        assert_eq!(registry.active_count(), 0);
    }

    #[test]
    fn same_destination_lease_waits_until_the_prior_operation_releases() {
        let registry = Arc::new(DestinationLeaseRegistry::default());
        let first = registry.acquire("destination-a".into());
        let (sender, receiver) = mpsc::channel();
        let waiting_registry = Arc::clone(&registry);
        let worker = std::thread::spawn(move || {
            sender.send("waiting").expect("waiting signal");
            let _lease = waiting_registry.acquire("destination-a".into());
            sender.send("acquired").expect("acquired signal");
        });
        assert_eq!(receiver.recv().expect("waiting"), "waiting");
        assert!(receiver.try_recv().is_err());
        drop(first);
        assert_eq!(receiver.recv().expect("acquired"), "acquired");
        worker.join().expect("worker");
    }

    #[test]
    fn verified_copy_independently_hashes_and_commits() {
        let root = std::env::temp_dir().join(format!("ingest-test-{}", Uuid::new_v4()));
        let source_root = root.join("source");
        let destination_root = root.join("destination");
        fs::create_dir_all(&source_root).expect("source");
        fs::write(source_root.join("clip.mov"), b"camera bytes").expect("fixture");
        let plan = enumerate_regular_files(&source_root).expect("plan");
        let outcome = verified_copy(
            &source_root,
            &plan[0],
            &destination_root,
            &AtomicBool::new(false),
        )
        .expect("copy");
        assert_eq!(outcome.bytes, 12);
        assert_eq!(
            fs::read(outcome.final_path).expect("copied"),
            b"camera bytes"
        );
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn recovery_rehashes_an_existing_final_file_and_rejects_mutation() {
        let root = std::env::temp_dir().join(format!("ingest-recovery-{}", Uuid::new_v4()));
        let source_root = root.join("source");
        let destination_root = root.join("destination");
        fs::create_dir_all(&source_root).expect("source");
        fs::write(source_root.join("clip.mov"), b"camera bytes").expect("fixture");
        let source = enumerate_regular_files(&source_root)
            .expect("plan")
            .remove(0);
        verified_copy_to(
            &source_root,
            &source,
            Path::new("recovered/clip.mov"),
            "entry-1",
            &destination_root,
            &AtomicBool::new(false),
        )
        .expect("initial copy");
        let verification_progress = Arc::new(Mutex::new(Vec::new()));
        let verification_progress_by_callback = Arc::clone(&verification_progress);
        let progress: CopyProgressCallback = Arc::new(move |update| {
            verification_progress_by_callback
                .lock()
                .expect("verification progress")
                .push(update);
        });
        let recovered = verify_existing_copy_with_progress(
            &source_root,
            &source,
            Path::new("recovered/clip.mov"),
            "entry-1",
            &destination_root,
            &AtomicBool::new(false),
            Some(VerificationProgress {
                callback: progress,
                file_index: 1,
                total_files: 1,
            }),
        )
        .expect("independent recovery hash");
        assert_eq!(recovered.bytes, 12);
        let verification_progress = verification_progress.lock().expect("verification progress");
        assert_eq!(
            verification_progress
                .iter()
                .map(|update| update.bytes)
                .sum::<u64>(),
            12
        );
        assert!(verification_progress.iter().all(|update| {
            update.stage == CopyProgressStage::Verifying
                && update.file_index == 1
                && update.total_files == 1
        }));
        drop(verification_progress);
        fs::write(
            destination_root.join("recovered/clip.mov"),
            b"mutated bytes",
        )
        .expect("mutation");
        assert!(matches!(
            verify_existing_copy(
                &source_root,
                &source,
                Path::new("recovered/clip.mov"),
                "entry-1",
                &destination_root,
                &AtomicBool::new(false),
            ),
            Err(IngestError::VerificationFailed)
        ));
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn recovery_rejects_a_destination_changed_during_readback() {
        let root = std::env::temp_dir().join(format!("ingest-recovery-change-{}", Uuid::new_v4()));
        let source_root = root.join("source");
        let destination_root = root.join("destination");
        fs::create_dir_all(&source_root).expect("source");
        fs::write(source_root.join("clip.mov"), b"camera bytes").expect("fixture");
        let source = enumerate_regular_files(&source_root)
            .expect("plan")
            .remove(0);
        let initial = verified_copy_to(
            &source_root,
            &source,
            &source.relative_path,
            "entry-1",
            &destination_root,
            &AtomicBool::new(false),
        )
        .expect("initial copy");
        let final_path = initial.final_path;
        let progress: CopyProgressCallback = Arc::new(move |_| {
            fs::write(&final_path, b"changed").expect("destination changed after readback chunk");
        });
        let recovered = verify_existing_copy_with_progress(
            &source_root,
            &source,
            &source.relative_path,
            "entry-1",
            &destination_root,
            &AtomicBool::new(false),
            Some(VerificationProgress {
                callback: progress,
                file_index: 1,
                total_files: 1,
            }),
        );
        fs::remove_dir_all(root).expect("cleanup");
        assert!(matches!(recovered, Err(IngestError::VerificationFailed)));
    }

    #[test]
    fn cancelled_copy_keeps_an_entry_scoped_partial_until_explicit_recovery() {
        let root = std::env::temp_dir().join(format!("ingest-partial-{}", Uuid::new_v4()));
        let source_root = root.join("source");
        let destination_root = root.join("destination");
        fs::create_dir_all(&source_root).expect("source");
        fs::write(source_root.join("clip.mov"), b"camera bytes").expect("fixture");
        let source = enumerate_regular_files(&source_root)
            .expect("plan")
            .pop()
            .expect("source file");
        let entry_id = "recovery-entry";
        let cancelled = AtomicBool::new(true);
        assert!(matches!(
            verified_copy_to(
                &source_root,
                &source,
                &source.relative_path,
                entry_id,
                &destination_root,
                &cancelled,
            ),
            Err(IngestError::Cancelled)
        ));
        let partial = partial_path(&destination_root, entry_id);
        assert!(partial.is_file());
        assert!(!destination_root.join("clip.mov").exists());

        let recovered = verified_copy_to(
            &source_root,
            &source,
            &source.relative_path,
            entry_id,
            &destination_root,
            &AtomicBool::new(false),
        )
        .expect("explicit recovery copy");
        assert_eq!(recovered.bytes, source.byte_length);
        assert!(!partial.exists());
        assert!(destination_root.join("clip.mov").is_file());
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn final_destination_metadata_must_remain_a_regular_expected_length_file() {
        let root = std::env::temp_dir().join(format!("ingest-final-stat-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).expect("root");
        let path = root.join("clip.mov");
        fs::write(&path, b"camera").expect("fixture");
        sync_final_file(&path).expect("sync final file");
        assert!(destination_metadata_matches(&path, 6).expect("regular final path"));
        assert!(!destination_metadata_matches(&path, 7).expect("length mismatch"));
        fs::remove_file(&path).expect("remove file");
        fs::create_dir(&path).expect("directory replacement");
        assert!(!destination_metadata_matches(&path, 6).expect("directory rejected"));
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn inventory_skips_only_root_windows_volume_metadata() {
        let root = std::env::temp_dir().join(format!("ingest-metadata-{}", Uuid::new_v4()));
        fs::create_dir_all(root.join("System Volume Information")).expect("metadata directory");
        fs::write(root.join("System Volume Information/host.dat"), b"host").expect("metadata");
        fs::write(root.join("clip.mov"), b"camera").expect("camera fixture");
        let plan = enumerate_regular_files(&root).expect("plan");
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].relative_path, PathBuf::from("clip.mov"));
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn copy_accepts_nested_new_destinations_but_rejects_overlap_and_parent_traversal() {
        let root =
            std::env::temp_dir().join(format!("ingest-nested-destination-{}", Uuid::new_v4()));
        let source_root = root.join("source");
        fs::create_dir_all(&source_root).expect("source");
        fs::write(source_root.join("clip.mov"), b"camera bytes").expect("source fixture");
        let source = enumerate_regular_files(&source_root)
            .expect("plan")
            .remove(0);
        let destination = root.join("new").join("nested").join("destination");
        let copied = verified_copy(&source_root, &source, &destination, &AtomicBool::new(false))
            .expect("nested new copy");
        assert_eq!(
            fs::read(copied.final_path).expect("readback"),
            b"camera bytes"
        );
        assert!(matches!(
            validate_ingest_roots(&source_root, &source_root.join("new/nested")),
            Err(IngestError::InvalidPath)
        ));
        assert!(matches!(
            validate_ingest_roots(&source_root, &root.join("missing/../source")),
            Err(IngestError::InvalidPath)
        ));
        assert_eq!(
            fs::read(source_root.join("clip.mov")).expect("source retained"),
            b"camera bytes"
        );
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn inventory_excludes_mac_volume_metadata_but_keeps_nested_names_and_dotfiles() {
        let root = std::env::temp_dir().join(format!("ingest-mac-metadata-{}", Uuid::new_v4()));
        for name in [".fseventsd", ".Spotlight-V100", ".Trashes"] {
            fs::create_dir_all(root.join(name)).expect("root metadata directory");
            fs::write(root.join(name).join("host.dat"), b"host").expect("host metadata");
            fs::create_dir_all(root.join("DCIM").join(name)).expect("nested camera directory");
            fs::write(root.join("DCIM").join(name).join("clip.mov"), b"camera")
                .expect("nested camera file");
        }
        fs::write(root.join(".camera-settings"), b"camera settings").expect("ordinary dotfile");
        let plan = enumerate_regular_files(&root).expect("plan");
        assert_eq!(plan.len(), 4);
        assert!(plan
            .iter()
            .any(|file| file.relative_path == Path::new(".camera-settings")));
        assert!(
            plan.iter()
                .filter(|file| file.relative_path.starts_with("DCIM"))
                .count()
                == 3
        );
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn mac_publication_preserves_a_competing_file_and_the_verified_partial() {
        let root = std::env::temp_dir().join(format!("ingest-mac-race-{}", Uuid::new_v4()));
        let source_root = root.join("source");
        let destination_root = root.join("destination");
        fs::create_dir_all(&source_root).expect("source");
        fs::write(source_root.join("clip.mov"), b"camera bytes").expect("source file");
        let source = enumerate_regular_files(&source_root)
            .expect("plan")
            .remove(0);
        let final_path = destination_root.join("clip.mov");
        let competitor_path = final_path.clone();
        let progress: FileByteProgressCallback = Arc::new(move |stage, _| {
            if stage == CopyProgressStage::Verifying {
                fs::write(&competitor_path, b"other operator bytes")
                    .expect("competing publication");
            }
        });
        let result = verified_copy_to_with_progress(
            &source_root,
            &source,
            Path::new("clip.mov"),
            "race-entry",
            &destination_root,
            &AtomicBool::new(false),
            Some(&progress),
        );
        assert!(matches!(result, Err(IngestError::DestinationExists)));
        assert_eq!(
            fs::read(final_path).expect("competitor retained"),
            b"other operator bytes"
        );
        assert_eq!(
            fs::read(partial_path(&destination_root, "race-entry")).expect("partial retained"),
            b"camera bytes"
        );
        assert_eq!(
            fs::read(source_root.join("clip.mov")).expect("source retained"),
            b"camera bytes"
        );
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn mac_publication_refuses_existing_receipts_and_dangling_symlinks() {
        let root = std::env::temp_dir().join(format!("ingest-mac-publish-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).expect("root");
        let partial = root.join(".receipt.partial");
        let receipt = root.join("receipt.json");
        fs::write(&partial, b"new receipt").expect("partial");
        fs::write(&receipt, b"old receipt").expect("existing receipt");
        assert!(matches!(
            publish_verified_file(&partial, &receipt),
            Err(IngestError::DestinationExists)
        ));
        assert_eq!(
            fs::read(&receipt).expect("receipt retained"),
            b"old receipt"
        );
        fs::remove_file(&receipt).expect("remove fixture receipt");
        // exFAT has no symlink support; that check belongs to the APFS runs.
        if std::os::unix::fs::symlink(root.join("missing"), &receipt).is_ok() {
            assert!(matches!(
                publish_verified_file(&partial, &receipt),
                Err(IngestError::DestinationExists)
            ));
            assert!(fs::symlink_metadata(&receipt)
                .expect("symlink retained")
                .file_type()
                .is_symlink());
            fs::remove_file(&receipt).expect("remove fixture symlink");
        }
        assert_eq!(
            fs::read(&partial).expect("partial retained"),
            b"new receipt"
        );
        publish_verified_file(&partial, &receipt).expect("normal publication");
        assert_eq!(
            fs::read(&receipt).expect("published receipt"),
            b"new receipt"
        );
        assert!(!partial.exists());
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn enumeration_rejects_file_counts_above_its_bounded_policy() {
        let root = std::env::temp_dir().join(format!("ingest-count-limit-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).expect("root");
        fs::write(root.join("one.mov"), b"one").expect("one");
        fs::write(root.join("two.mov"), b"two").expect("two");
        assert!(matches!(
            enumerate_regular_files_with_limits(
                &root,
                SourceEnumerationLimits {
                    max_directory_depth: 64,
                    max_file_count: 1,
                },
            ),
            Err(IngestError::SourceLimitExceeded)
        ));
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn enumeration_rejects_nesting_above_its_bounded_policy() {
        let root = std::env::temp_dir().join(format!("ingest-depth-limit-{}", Uuid::new_v4()));
        let nested = root.join("one").join("two");
        fs::create_dir_all(&nested).expect("nested");
        fs::write(nested.join("clip.mov"), b"clip").expect("fixture");
        assert!(matches!(
            enumerate_regular_files_with_limits(
                &root,
                SourceEnumerationLimits {
                    max_directory_depth: 1,
                    max_file_count: 10,
                },
            ),
            Err(IngestError::SourceLimitExceeded)
        ));
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn state_machine_rejects_verified_back_to_copying() {
        assert!(can_transition(
            FileState::Verifying,
            FileState::ByteVerified
        ));
        assert!(!can_transition(FileState::ByteVerified, FileState::Copying));
        assert!(!can_transition(FileState::Committed, FileState::Copying));
    }

    #[test]
    fn bounded_batch_copies_multiple_files_without_unbounded_workers() {
        let root = std::env::temp_dir().join(format!("ingest-batch-{}", Uuid::new_v4()));
        let source_root = root.join("source");
        let destination_root = root.join("destination");
        fs::create_dir_all(&source_root).expect("source");
        fs::write(source_root.join("a.mov"), b"a").expect("fixture");
        fs::write(source_root.join("b.mov"), b"b").expect("fixture");
        let outcomes = verified_copy_batch(
            &source_root,
            enumerate_regular_files(&source_root).expect("plan"),
            &destination_root,
            WorkerLimits { max_workers: 1 },
            Arc::new(AtomicBool::new(false)),
        );
        assert_eq!(outcomes.len(), 2);
        assert!(outcomes.iter().all(Result::is_ok));
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn planned_batch_reports_each_written_byte_across_workers() {
        let root = std::env::temp_dir().join(format!("ingest-progress-{}", Uuid::new_v4()));
        let source_root = root.join("source");
        let destination_root = root.join("destination");
        fs::create_dir_all(&source_root).expect("source");
        let first_bytes = COPY_BUFFER_BYTES + 37;
        let second_bytes = COPY_BUFFER_BYTES / 2 + 19;
        fs::write(source_root.join("a.mov"), vec![0xA5; first_bytes]).expect("first fixture");
        fs::write(source_root.join("b.mov"), vec![0x5A; second_bytes]).expect("second fixture");
        let planned = enumerate_regular_files(&source_root)
            .expect("plan")
            .into_iter()
            .map(|source| PlannedCopyFile {
                entry_id: Uuid::new_v4().to_string(),
                destination_relative_path: source.relative_path.clone(),
                source,
            })
            .collect();
        let reported = Arc::new(AtomicU64::new(0));
        let reported_by_callback = Arc::clone(&reported);
        let reported_metadata = Arc::new(Mutex::new(Vec::new()));
        let reported_metadata_by_callback = Arc::clone(&reported_metadata);
        let progress: CopyProgressCallback = Arc::new(move |update| {
            reported_by_callback.fetch_add(update.bytes, Ordering::Relaxed);
            reported_metadata_by_callback
                .lock()
                .expect("progress metadata")
                .push(update);
        });
        let outcomes = verified_copy_batch_planned_with_progress(
            &source_root,
            planned,
            &destination_root,
            WorkerLimits { max_workers: 2 },
            Arc::new(AtomicBool::new(false)),
            Some(progress),
        );
        assert!(outcomes.iter().all(Result::is_ok));
        assert_eq!(
            reported.load(Ordering::Relaxed),
            ((first_bytes + second_bytes) * 2) as u64
        );
        let metadata = reported_metadata.lock().expect("progress metadata");
        assert!(!metadata.is_empty());
        assert!(metadata.iter().all(|update| {
            update.bytes > 0
                && update.file_index > 0
                && update.file_index <= 2
                && update.total_files == 2
        }));
        assert!(metadata.iter().any(|update| update.file_index == 1));
        assert!(metadata.iter().any(|update| update.file_index == 2));
        assert_eq!(
            metadata
                .iter()
                .filter(|update| update.stage == CopyProgressStage::Copying)
                .map(|update| update.bytes)
                .sum::<u64>(),
            (first_bytes + second_bytes) as u64
        );
        assert_eq!(
            metadata
                .iter()
                .filter(|update| update.stage == CopyProgressStage::Verifying)
                .map(|update| update.bytes)
                .sum::<u64>(),
            (first_bytes + second_bytes) as u64
        );
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn source_destination_overlap_is_rejected_before_copy() {
        let root = std::env::temp_dir().join(format!("ingest-overlap-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).expect("root");
        assert!(matches!(
            validate_ingest_roots(&root, &root.join("destination")),
            Err(IngestError::InvalidPath)
        ));
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn destination_preflight_uses_the_nearest_existing_parent() {
        let root = std::env::temp_dir().join(format!("ingest-space-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).expect("root");
        let nested_destination = root.join("not-created").join("yet");
        assert_eq!(
            nearest_existing_ancestor(&nested_destination).expect("existing parent"),
            root.as_path()
        );
        assert!(has_destination_space(&nested_destination, 0).expect("space query"));
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn a_destination_file_is_rejected_before_space_preflight_or_copy() {
        let root = std::env::temp_dir().join(format!("ingest-destination-file-{}", Uuid::new_v4()));
        let source_root = root.join("source");
        let destination_file = root.join("destination-file");
        fs::create_dir_all(&source_root).expect("source");
        fs::write(&destination_file, b"not a directory").expect("destination file");
        assert!(matches!(
            validate_ingest_roots(&source_root, &destination_file),
            Err(IngestError::InvalidPath)
        ));
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn root_marker_is_not_copied_as_camera_media() {
        let root = std::env::temp_dir().join(format!("ingest-marker-skip-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).expect("root");
        fs::write(
            root.join(MARKER_FILE_NAME),
            b"MIT1:00000000-0000-4000-8000-000000000000\n",
        )
        .expect("marker");
        fs::write(root.join("clip.mov"), b"clip").expect("clip");
        let files = enumerate_regular_files(&root).expect("plan");
        assert_eq!(
            files
                .iter()
                .map(|file| file.relative_path.clone())
                .collect::<Vec<_>>(),
            vec![PathBuf::from("clip.mov")]
        );
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn source_stability_compares_length_and_available_modification_evidence() {
        let root = std::env::temp_dir().join(format!("ingest-source-stability-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).expect("root");
        let path = root.join("clip.mov");
        fs::write(&path, b"first").expect("first");
        let before = fs::metadata(&path).expect("before");
        fs::write(&path, b"second-byte-change").expect("second");
        let after = fs::metadata(&path).expect("after");
        assert!(source_metadata_changed(&before, &after));
        assert!(!source_metadata_changed(&after, &after));
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn verified_copy_enforces_source_destination_separation() {
        let root = std::env::temp_dir().join(format!("ingest-guard-{}", Uuid::new_v4()));
        let source_root = root.join("source");
        fs::create_dir_all(&source_root).expect("source");
        fs::write(source_root.join("clip.mov"), b"camera bytes").expect("fixture");
        let plan = enumerate_regular_files(&source_root).expect("plan");
        assert!(matches!(
            verified_copy(
                &source_root,
                &plan[0],
                &source_root.join("nested-destination"),
                &AtomicBool::new(false),
            ),
            Err(IngestError::InvalidPath)
        ));
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn copy_primitive_rejects_unportable_destination_components() {
        let root = std::env::temp_dir().join(format!("ingest-portable-path-{}", Uuid::new_v4()));
        let source_root = root.join("source");
        let destination_root = root.join("destination");
        fs::create_dir_all(&source_root).expect("source");
        fs::write(source_root.join("clip.mov"), b"camera bytes").expect("fixture");
        let source = enumerate_regular_files(&source_root).expect("plan");
        assert!(matches!(
            verified_copy_to(
                &source_root,
                &source[0],
                Path::new("CON.mov"),
                "entry-id",
                &destination_root,
                &AtomicBool::new(false),
            ),
            Err(IngestError::InvalidPath)
        ));
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn receipt_is_written_as_versioned_json() {
        let root = std::env::temp_dir().join(format!("ingest-receipt-{}", Uuid::new_v4()));
        let path = root.join("receipt.json");
        write_receipt(
            &path,
            &IngestReceipt {
                schema_version: 1,
                manifest_algorithm: MANIFEST_ALGORITHM.into(),
                manifest_root_blake3: "test-root".into(),
                source_medium_key: "v1:card".into(),
                source_identity_confidence: "hardware_immutable".into(),
                source_generation: 2,
                files: vec![ReceiptFile {
                    relative_path: "clip.mov".into(),
                    bytes: 12,
                    blake3: "abc".into(),
                }],
            },
        )
        .expect("receipt");
        let json = fs::read_to_string(path).expect("read");
        assert!(json.contains("sourceMediumKey"));
        assert!(matches!(
            write_receipt(
                &root.join("receipt.json"),
                &IngestReceipt {
                    schema_version: 1,
                    manifest_algorithm: MANIFEST_ALGORITHM.into(),
                    manifest_root_blake3: "test-root".into(),
                    source_medium_key: "v1:card".into(),
                    source_identity_confidence: "hardware_immutable".into(),
                    source_generation: 2,
                    files: Vec::new(),
                },
            ),
            Err(IngestError::DestinationExists)
        ));
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn manifest_root_is_order_independent_but_binds_the_full_copy_record() {
        let first = VerifiedCopy {
            bytes: 5,
            digest_hex: "a".repeat(64),
            entry_id: "one".into(),
            source_relative_path: PathBuf::from("DCIM/A.MOV"),
            destination_relative_path: PathBuf::from("Camera/A.MOV"),
            final_path: PathBuf::from("unused"),
        };
        let second = VerifiedCopy {
            bytes: 7,
            digest_hex: "b".repeat(64),
            entry_id: "two".into(),
            source_relative_path: PathBuf::from("DCIM/B.MOV"),
            destination_relative_path: PathBuf::from("Camera/B.MOV"),
            final_path: PathBuf::from("unused"),
        };
        assert_eq!(
            manifest_root(&[first.clone(), second.clone()]),
            manifest_root(&[second.clone(), first.clone()])
        );
        let mut changed = second.clone();
        changed.bytes = 8;
        assert_ne!(
            manifest_root(&[first.clone(), changed]),
            manifest_root(&[first, second])
        );
    }
}
