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
pub type CopyProgressCallback = Arc<dyn Fn(u64) + Send + Sync>;

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
        let parent = destination_root.parent().ok_or(IngestError::InvalidPath)?;
        let name = destination_root
            .file_name()
            .ok_or(IngestError::InvalidPath)?;
        parent.canonicalize()?.join(name)
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
        // Windows creates this protected root directory on otherwise clean
        // removable media. It is host metadata, never camera media, and may
        // be unreadable to a normal desktop process. Do not let it make a
        // clean-card inventory fail; every other inaccessible entry remains
        // an explicit scan error.
        if relative_path == Path::new("System Volume Information") {
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
    if !destination_metadata_matches(&final_path, source.byte_length)? {
        return Err(IngestError::VerificationFailed);
    }
    let source_digest = hash_file(&source_path, cancel)?;
    let destination_digest = hash_file(&final_path, cancel)?;
    let after = fs::symlink_metadata(&source_path)?;
    if is_prohibited_source_entry(&after) || source_metadata_changed(&before, &after) {
        return Err(IngestError::SourceChanged);
    }
    if source_digest != destination_digest {
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
    progress: Option<&CopyProgressCallback>,
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
    let temporary_path = parent.join(format!(".ingest-{}.partial", Uuid::new_v4()));
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
            fs::rename(&temporary_path, &final_path)?;
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
        Err(error) => {
            let _ = fs::remove_file(&temporary_path);
            Err(error)
        }
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
    if files.is_empty() {
        return Vec::new();
    }
    let total = files.len();
    let worker_count = limits.max_workers.clamp(1, total);
    let (job_tx, job_rx) = mpsc::channel::<(usize, PlannedCopyFile)>();
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
            let Ok((index, file)) = job else { break };
            let result = verified_copy_to_with_progress(
                &source_root,
                &file.source,
                &file.destination_relative_path,
                &file.entry_id,
                &destination_root,
                &cancel,
                progress.as_ref(),
            );
            if result_tx.send((index, result)).is_err() {
                break;
            }
        }));
    }
    for (index, file) in files.into_iter().enumerate() {
        let _ = job_tx.send((index, file));
    }
    drop(job_tx);
    drop(result_tx);
    let mut outcomes = (0..total)
        .map(|_| None)
        .collect::<Vec<Option<Result<VerifiedCopy, IngestError>>>>();
    for _ in 0..total {
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
    fs::rename(temporary, path)?;
    Ok(())
}

fn copy_and_verify(
    source_path: &Path,
    temporary_path: &Path,
    expected_bytes: u64,
    cancel: &AtomicBool,
    progress: Option<&CopyProgressCallback>,
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
            progress(read as u64);
        }
    }
    if copied != expected_bytes {
        return Err(IngestError::SourceChanged);
    }
    temporary.sync_all()?;
    drop(temporary);
    let destination_digest = hash_file(temporary_path, cancel)?;
    let source_digest = source_hasher.finalize().to_hex().to_string();
    if destination_digest != source_digest {
        return Err(IngestError::VerificationFailed);
    }
    Ok((copied, source_digest))
}

fn hash_file(path: &Path, cancel: &AtomicBool) -> Result<String, IngestError> {
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
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicU64};

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
        let recovered = verify_existing_copy(
            &source_root,
            &source,
            Path::new("recovered/clip.mov"),
            "entry-1",
            &destination_root,
            &AtomicBool::new(false),
        )
        .expect("independent recovery hash");
        assert_eq!(recovered.bytes, 12);
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
        let progress: CopyProgressCallback = Arc::new(move |bytes| {
            reported_by_callback.fetch_add(bytes, Ordering::Relaxed);
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
