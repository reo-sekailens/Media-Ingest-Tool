//! Trusted desktop boundary for the media-ingest application.
//!
//! This module intentionally contains contracts and deterministic seams only.
//! Native discovery, transfer, verification, and format implementations belong
//! to their respective TASK001 follow-up modules.

pub mod device_discovery;
pub mod eject_provider;
pub mod format_profiles;
pub mod format_provider;
pub mod format_safety;
pub mod identity;
pub mod ingest;
pub mod local_store;
#[cfg(target_os = "macos")]
mod macos_disk_arbitration;
pub mod metadata;
pub mod organization;
pub mod reader_slots;
pub mod storage_marker;

use log::{debug, info, warn};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use std::{
    collections::{HashMap, HashSet},
    ffi::OsStr,
};
use tauri::ipc::Channel;
use tauri::{Emitter, Manager, State};

use crate::device_discovery::{DeviceDiscovery, NativeDeviceDiscovery};
use crate::format_profiles::{recommended_profile, FormatProfile};
use crate::format_safety::{consume_authorization, issue_authorization, FormatAuthorization};
use crate::identity::{derive_key, IdentityScope};
use crate::ingest::{
    destination_lease_key, enumerate_regular_files, has_destination_space, manifest_root,
    validate_ingest_roots, verified_copy_batch_planned_with_progress,
    verified_copy_batch_planned_with_progress_positions, verify_existing_copy_with_progress,
    write_receipt, CopyProgress, CopyProgressCallback, CopyProgressStage, DestinationLeaseRegistry,
    IngestError, IngestReceipt, PlannedCopyFile, ReceiptFile, VerificationProgress, WorkerLimits,
    MANIFEST_ALGORITHM,
};
use crate::local_store::{
    IngestHistoryEntry, IngestRunState, LocalStore, MarkerIngestProfile, PlannedFileRecord,
    ReaderSlotKind, RecoverableIngestRun, SourceIdentityRecord, VerifiedFileRecord,
};
use crate::metadata::inspect;
use crate::organization::{
    camera_identity, custom_directory_prefix, destination_relative_path_with_order_and_offset,
    CustomDirectoryField, DestinationDepthSegment, SortMode,
};

pub const IPC_CONTRACT_VERSION: u16 = 1;
static SNAPSHOT_SEQUENCE: AtomicU64 = AtomicU64::new(0);
static PROGRESS_SEQUENCE: AtomicU64 = AtomicU64::new(0);
/// The copy engine may complete many 1 MiB reads in one display frame. Keep
/// native progress exact while placing a firm upper bound on IPC and React
/// work for each active ingest.
const PROGRESS_EVENT_MIN_INTERVAL: Duration = Duration::from_millis(100);

struct AppState {
    store: Arc<Mutex<LocalStore>>,
    active_ingests: Arc<Mutex<HashMap<String, ActiveIngest>>>,
    destination_leases: Arc<DestinationLeaseRegistry>,
    format_authorizations: Arc<Mutex<HashMap<String, FormatAuthorization>>>,
    connection_generations: Arc<Mutex<ConnectionGenerationTracker>>,
}

#[derive(Default)]
struct ConnectionGenerationTracker {
    next_generation: u64,
    present: HashMap<String, u64>,
    seeded_from_store: bool,
}

struct ActiveIngest {
    cancellation: Arc<std::sync::atomic::AtomicBool>,
    source_medium_key: String,
    source_root: String,
    /// A device-change notification may arrive before a newly inserted card
    /// has mounted. Only cancel after this exact source was observed first.
    source_seen_in_snapshot: bool,
}

struct CancellationRegistration {
    active_ingests: Arc<Mutex<HashMap<String, ActiveIngest>>>,
    operation_id: String,
}

struct ProgressEmissionGate {
    last_emitted: Mutex<Option<Instant>>,
}

impl ProgressEmissionGate {
    fn should_emit(&self, now: Instant) -> bool {
        let Ok(mut last_emitted) = self.last_emitted.lock() else {
            // A poisoned UI-reporting mutex must never prevent a verified copy
            // from proceeding. Dropping this non-authoritative event is safe.
            return false;
        };
        if last_emitted.is_some_and(|last| now.duration_since(last) < PROGRESS_EVENT_MIN_INTERVAL) {
            return false;
        }
        *last_emitted = Some(now);
        true
    }
}

impl Drop for CancellationRegistration {
    fn drop(&mut self) {
        if let Ok(mut active_ingests) = self.active_ingests.lock() {
            active_ingests.remove(&self.operation_id);
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IpcResponse<T> {
    pub contract_version: u16,
    pub data: Option<T>,
    pub error: Option<IpcError>,
}

impl<T> IpcResponse<T> {
    fn success(data: T) -> Self {
        Self {
            contract_version: IPC_CONTRACT_VERSION,
            data: Some(data),
            error: None,
        }
    }

    fn failure(code: IpcErrorCode, message: impl Into<String>) -> Self {
        Self {
            contract_version: IPC_CONTRACT_VERSION,
            data: None,
            error: Some(IpcError {
                code,
                message: message.into(),
                os_error: None,
            }),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IpcError {
    pub code: IpcErrorCode,
    pub message: String,
    /// Sanitized OS context only; never a raw source or destination path.
    pub os_error: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum IpcErrorCode {
    DeviceUnavailable,
    InvalidRequest,
    OperationCancelled,
    UnsupportedPlatform,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceState {
    Available,
    EmptyReader,
    Busy,
    Removed,
    Unsupported,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IdentityConfidence {
    HardwareImmutable,
    HardwareStable,
    SessionOnly,
    Unresolved,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IdentityEvidenceKind {
    SdCid,
    StorageVpd,
    AppMarker,
    UsbSerial,
    ReaderSerial,
    VolumeFilesystemUuid,
    MountPath,
    DriveLetter,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityEvidence {
    pub kind: IdentityEvidenceKind,
    /// A local diagnostic fingerprint; raw hardware values remain native-only.
    pub fingerprint: String,
    pub immutable: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceIdentity {
    pub media_key: String,
    pub confidence: IdentityConfidence,
    pub evidence: Vec<IdentityEvidence>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageDeviceDetails {
    pub display_name: String,
    pub filesystem: Option<String>,
    pub total_bytes: Option<u64>,
    pub available_bytes: Option<u64>,
    /// Ephemeral presentation data. Never use this as a persistent identity.
    pub mount_locations: Vec<String>,
    pub reader_fingerprint: Option<String>,
    /// Product-family hint only; never an individual reader or card identity.
    pub reader_family: Option<device_discovery::ReaderFamily>,
    pub reader_slot: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageDevice {
    pub state: DeviceState,
    /// Increments only after this medium is absent from an observed native
    /// snapshot and later returns. It is not the snapshot refresh sequence.
    pub connection_generation: u64,
    pub identity: DeviceIdentity,
    pub details: StorageDeviceDetails,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceSnapshot {
    pub sequence: u64,
    pub devices: Vec<StorageDevice>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IngestState {
    Queued,
    Copying,
    Verifying,
    Formatting,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgressUpdate {
    pub sequence: u64,
    pub operation_id: String,
    pub state: IngestState,
    pub transferred_bytes: u64,
    pub total_bytes: Option<u64>,
    pub current_file_index: Option<usize>,
    pub total_files: Option<usize>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifiedIngestRequest {
    /// Client-generated opaque UUID used only for cancellation/progress.
    pub operation_id: Option<String>,
    pub source_root: String,
    pub destination_root: String,
    pub source_medium_key: String,
    pub source_identity_confidence: IdentityConfidence,
    pub source_generation: u64,
    pub max_workers: usize,
    pub sort_mode: IngestSortMode,
    pub interval_minutes: Option<u16>,
    #[serde(default)]
    pub custom_directory_fields: Vec<CustomDirectoryField>,
    #[serde(default)]
    pub destination_depth_order: Option<Vec<DestinationDepthSegment>>,
    /// Set only by the mount-triggered native-app workflow. A registered
    /// auto-format preference never runs after a user-started manual ingest.
    #[serde(default)]
    pub auto_ingest_triggered: bool,
}

/// Resuming never accepts UI-controlled paths, identity evidence, or a new
/// plan. The backend reloads the frozen native plan and only permits the exact
/// interrupted run after a fresh device/generation match.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ResumeVerifiedIngestRequest {
    pub run_id: String,
    pub max_workers: usize,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IngestSortMode {
    OriginalTree,
    CameraDay,
    CameraInterval,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifiedIngestResult {
    pub operation_id: String,
    pub source_medium_key: String,
    pub source_generation: u64,
    pub copied_files: usize,
    pub copied_bytes: u64,
    /// A basename only, so native source/destination paths never cross IPC.
    pub receipt_name: String,
    pub source_marker_status: SourceMarkerStatus,
    /// Result of the explicitly configured post-verification auto-format
    /// lifecycle. It never reports success unless remount validation and
    /// marker restoration both complete.
    pub auto_format_status: AutoFormatStatus,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AutoFormatStatus {
    NotConfigured,
    Skipped,
    Completed,
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IngestPlanPreview {
    pub operation_id: String,
    pub file_count: usize,
    pub total_bytes: u64,
    pub sample_destination_paths: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceMarkerStatus {
    Recognized,
    Created,
    Unavailable,
}

struct CompletedIngest {
    summary: VerifiedIngestResult,
    copies: Vec<crate::ingest::VerifiedCopy>,
    manifest_algorithm: String,
    manifest_root_blake3: String,
    source_root: PathBuf,
    auto_ingest_triggered: bool,
}

struct PreparedIngest {
    request: VerifiedIngestRequest,
    operation_id: String,
    source_root: PathBuf,
    destination_root: PathBuf,
    files: Vec<PlannedCopyFile>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ReaderSlotCalibrationRequest {
    pub reader_fingerprint: String,
    pub logical_unit: u8,
    pub slot_kind: CalibratedSlotKind,
    pub evidence_note: String,
}

/// This request deliberately carries no filesystem path, disk number, drive
/// letter, or provider arguments. It produces only a short-lived native token
/// after the receipt and currently observed medium agree.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FormatAuthorizationRequest {
    pub run_id: String,
    pub source_medium_key: String,
    pub source_generation: u64,
    pub source_identity_confidence: IdentityConfidence,
}

/// Recovery-only destructive operation. This deliberately has no mount path,
/// filesystem, or profile input: those stay native-owned. It is available only
/// for an already registered current card after the operator types the phrase.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ForceFormatAuthorizationRequest {
    pub source_medium_key: String,
    pub source_generation: u64,
    pub confirmation_phrase: String,
}

/// The webview identifies only the completed run and native source identity.
/// The backend re-resolves the current mount and never accepts a drive path.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SafeEjectRequest {
    pub run_id: String,
    pub source_medium_key: String,
    pub source_generation: u64,
    pub source_identity_confidence: IdentityConfidence,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SafeEjectResult {
    pub source_medium_key: String,
    pub source_generation: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FormatAuthorizationResult {
    pub confirmation_token: String,
    pub expires_in_seconds: u8,
}

/// The confirmation UI may submit only the short-lived opaque token it was
/// just issued. Target identity, mount discovery, and profile selection stay
/// exclusively on the native side of the boundary.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ExecuteFormatAuthorizationRequest {
    pub confirmation_token: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FormatExecutionResult {
    pub profile_id: String,
    pub marker_restored: bool,
}

/// Non-destructive explanation of the same conditions required before a
/// platform formatter may receive an authorization token.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FormatEligibility {
    pub eligible: bool,
    pub reason: String,
    pub recommended_profile: Option<FormatProfile>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SourceInventoryRequest {
    pub source_root: String,
    pub source_medium_key: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceInventory {
    pub file_count: usize,
    pub total_bytes: u64,
}

/// Destination recall is deliberately keyed only by the currently observed
/// medium identity. The frontend cannot select an arbitrary persisted profile.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DestinationProfileRequest {
    pub source_medium_key: String,
    pub destination_path: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RememberedDestination {
    pub destination_path: Option<String>,
    pub can_remember: bool,
}

/// An explicitly user-trusted marker profile. It is intentionally separate
/// from hardware identity: copying the marker can impersonate this profile.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CardRegistrationRequest {
    pub source_medium_key: String,
    pub destination_path: String,
    pub sort_mode: IngestSortMode,
    pub interval_minutes: Option<u16>,
    #[serde(default)]
    pub custom_directory_fields: Vec<CustomDirectoryField>,
    #[serde(default)]
    pub destination_depth_order: Option<Vec<DestinationDepthSegment>>,
    pub auto_ingest_enabled: bool,
    pub auto_format_enabled: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CardRegistration {
    pub registered: bool,
    pub auto_ingest_enabled: bool,
    pub auto_ingest_already_completed: bool,
    pub auto_format_enabled: bool,
    pub destination_path: Option<String>,
    pub sort_mode: Option<IngestSortMode>,
    pub interval_minutes: Option<u16>,
    pub custom_directory_fields: Vec<CustomDirectoryField>,
    pub destination_depth_order: Vec<DestinationDepthSegment>,
    pub marker_status: SourceMarkerStatus,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum CalibratedSlotKind {
    Sd,
    MicroSd,
}

/// Boundary for native adapters. Tests and UI development use the deterministic
/// implementation until TASK002 supplies platform-specific discovery.
pub trait DeviceSnapshotProvider: Send + Sync {
    fn snapshot(&self) -> DeviceSnapshot;
}

#[derive(Default)]
pub struct DeterministicDeviceSnapshotProvider;

impl DeviceSnapshotProvider for DeterministicDeviceSnapshotProvider {
    fn snapshot(&self) -> DeviceSnapshot {
        DeviceSnapshot {
            sequence: 0,
            devices: Vec::new(),
        }
    }
}

#[tauri::command]
async fn get_device_snapshot(
    state: State<'_, AppState>,
) -> Result<IpcResponse<DeviceSnapshot>, IpcError> {
    Ok(
        match blocking_native_device_snapshot(
            Arc::clone(&state.store),
            Arc::clone(&state.connection_generations),
        )
        .await
        {
            Ok(snapshot) => {
                debug!(
                    target: "media_ingest_tool::support",
                    "device_snapshot_complete device_count={}",
                    snapshot.devices.len()
                );
                IpcResponse::success(snapshot)
            }
            Err(()) => IpcResponse::failure(
                IpcErrorCode::OperationCancelled,
                "The native device snapshot worker ended before completing discovery",
            ),
        },
    )
}

#[tauri::command]
fn get_ingest_history(state: State<'_, AppState>) -> IpcResponse<Vec<IngestHistoryEntry>> {
    let Ok(store) = state.store.lock() else {
        return IpcResponse::failure(
            IpcErrorCode::DeviceUnavailable,
            "The local ingest history is unavailable",
        );
    };
    match store.recent_ingest_runs(20) {
        Ok(history) => IpcResponse::success(history),
        Err(_) => IpcResponse::failure(
            IpcErrorCode::DeviceUnavailable,
            "The local ingest history could not be read",
        ),
    }
}

#[tauri::command]
async fn scan_source_inventory(
    request: SourceInventoryRequest,
    state: State<'_, AppState>,
) -> Result<IpcResponse<SourceInventory>, IpcError> {
    let store = Arc::clone(&state.store);
    let connection_generations = Arc::clone(&state.connection_generations);
    Ok(
        match tauri::async_runtime::spawn_blocking(move || {
            let snapshot = native_device_snapshot(&store, &connection_generations);
            if !source_inventory_matches_current_device(&snapshot, &request) {
                return IpcResponse::failure(
                    IpcErrorCode::DeviceUnavailable,
                    "The selected mounted source card is no longer present",
                );
            }
            match enumerate_regular_files(&PathBuf::from(request.source_root)) {
                Ok(files) => {
                    let inventory = SourceInventory {
                        file_count: files.len(),
                        total_bytes: files.iter().map(|file| file.byte_length).sum(),
                    };
                    debug!(
                        target: "media_ingest_tool::support",
                        "source_inventory_complete file_count={} total_bytes={}",
                        inventory.file_count,
                        inventory.total_bytes
                    );
                    IpcResponse::success(inventory)
                }
                Err(IngestError::Io(error))
                    if error.kind() == std::io::ErrorKind::PermissionDenied =>
                {
                    IpcResponse::failure(
                        IpcErrorCode::DeviceUnavailable,
                        "The source contains protected entries; no ingest plan was created",
                    )
                }
                Err(IngestError::SourceLimitExceeded) => IpcResponse::failure(
                    IpcErrorCode::InvalidRequest,
                    "The source exceeds the safe media inventory limit",
                ),
                Err(_) => IpcResponse::failure(
                    IpcErrorCode::DeviceUnavailable,
                    "The source inventory could not be read safely",
                ),
            }
        })
        .await
        {
            Ok(response) => response,
            Err(_) => IpcResponse::failure(
                IpcErrorCode::OperationCancelled,
                "The source inventory worker ended before completing the scan",
            ),
        },
    )
}

#[tauri::command]
fn get_remembered_destination(
    source_medium_key: String,
    state: State<'_, AppState>,
) -> IpcResponse<RememberedDestination> {
    let snapshot = native_device_snapshot(&state.store, &state.connection_generations);
    let Some(identity) = source_identity_for_current_device(&snapshot, &source_medium_key) else {
        return IpcResponse::failure(
            IpcErrorCode::DeviceUnavailable,
            "The selected removable card is no longer present",
        );
    };
    let can_remember = allows_destination_recall(&identity);
    let Ok(store) = state.store.lock() else {
        return IpcResponse::failure(
            IpcErrorCode::DeviceUnavailable,
            "The local destination profiles are unavailable",
        );
    };
    let destination = if can_remember {
        store.primary_destination(&identity.identity_key)
    } else {
        Ok(None)
    };
    match destination {
        Ok(destination_path) => IpcResponse::success(RememberedDestination {
            destination_path,
            can_remember,
        }),
        Err(_) => IpcResponse::failure(
            IpcErrorCode::DeviceUnavailable,
            "The remembered destination could not be read",
        ),
    }
}

#[tauri::command]
fn remember_destination(
    request: DestinationProfileRequest,
    state: State<'_, AppState>,
) -> IpcResponse<RememberedDestination> {
    let destination = PathBuf::from(&request.destination_path);
    if request.source_medium_key.trim().is_empty()
        || !destination.is_absolute()
        || !destination.is_dir()
    {
        return IpcResponse::failure(
            IpcErrorCode::InvalidRequest,
            "Choose an existing absolute destination directory before remembering it",
        );
    }
    let snapshot = native_device_snapshot(&state.store, &state.connection_generations);
    let Some(identity) = source_identity_for_current_device(&snapshot, &request.source_medium_key)
    else {
        return IpcResponse::failure(
            IpcErrorCode::DeviceUnavailable,
            "The selected removable card is no longer present",
        );
    };
    if !allows_destination_recall(&identity) {
        return IpcResponse::success(RememberedDestination {
            destination_path: None,
            can_remember: false,
        });
    }
    let Ok(mut store) = state.store.lock() else {
        return IpcResponse::failure(
            IpcErrorCode::DeviceUnavailable,
            "The local destination profiles are unavailable",
        );
    };
    match store.set_primary_destination(&identity, &request.destination_path) {
        Ok(true) => IpcResponse::success(RememberedDestination {
            destination_path: Some(request.destination_path),
            can_remember: true,
        }),
        Ok(false) => IpcResponse::success(RememberedDestination {
            destination_path: None,
            can_remember: false,
        }),
        Err(_) => IpcResponse::failure(
            IpcErrorCode::DeviceUnavailable,
            "The remembered destination could not be saved",
        ),
    }
}

#[tauri::command]
fn register_card_marker(
    request: CardRegistrationRequest,
    state: State<'_, AppState>,
) -> IpcResponse<CardRegistration> {
    let destination = PathBuf::from(&request.destination_path);
    if request.source_medium_key.trim().is_empty()
        || !destination.is_absolute()
        || !destination.is_dir()
    {
        return IpcResponse::failure(
            IpcErrorCode::InvalidRequest,
            "Choose an existing absolute destination before registering this card",
        );
    }
    if matches!(request.sort_mode, IngestSortMode::CameraInterval)
        && !matches!(request.interval_minutes, Some(1..=1_440))
    {
        return IpcResponse::failure(
            IpcErrorCode::InvalidRequest,
            "Choose a valid 1 to 1,440 minute interval before registering this card",
        );
    }
    if custom_directory_prefix(&request.custom_directory_fields).is_err() {
        return IpcResponse::failure(
            IpcErrorCode::InvalidRequest,
            "Custom fields and destination depth order must form one valid complete path",
        );
    }
    let destination_depth_order = match crate::organization::canonical_destination_depth_order(
        &sort_mode_for_request(&request.sort_mode, request.interval_minutes),
        request.custom_directory_fields.len(),
        request.destination_depth_order.as_deref(),
    ) {
        Ok(order) => order,
        Err(_) => {
            return IpcResponse::failure(
                IpcErrorCode::InvalidRequest,
                "Custom fields and destination depth order must form one valid complete path",
            )
        }
    };
    let snapshot = native_device_snapshot(&state.store, &state.connection_generations);
    let current = snapshot
        .devices
        .iter()
        .filter(|device| {
            device.state == DeviceState::Available
                && device.identity.media_key == request.source_medium_key
                && device.details.mount_locations.len() == 1
        })
        .collect::<Vec<_>>();
    if current.len() != 1 {
        return IpcResponse::failure(
            IpcErrorCode::DeviceUnavailable,
            "The card must be uniquely mounted before it can be registered",
        );
    }
    let root = PathBuf::from(&current[0].details.mount_locations[0]);
    let marker_status = match source_marker_status(&root) {
        SourceMarkerStatus::Unavailable => {
            return IpcResponse::failure(
                IpcErrorCode::DeviceUnavailable,
                "The card marker could not be created; check that the card is writable",
            )
        }
        status => status,
    };
    let marker_token = match crate::storage_marker::read_marker(&root) {
        Ok(Some(token)) => token,
        _ => {
            return IpcResponse::failure(
                IpcErrorCode::DeviceUnavailable,
                "The card marker could not be read after registration",
            )
        }
    };
    let Ok(mut store) = state.store.lock() else {
        return IpcResponse::failure(
            IpcErrorCode::DeviceUnavailable,
            "The local card registration store is unavailable",
        );
    };
    let profile = MarkerIngestProfile {
        marker_token,
        destination_path: request.destination_path,
        sort_mode: ingest_sort_mode_name(&request.sort_mode).into(),
        interval_minutes: match request.sort_mode {
            IngestSortMode::CameraInterval => request.interval_minutes,
            _ => None,
        },
        custom_directory_fields: request.custom_directory_fields,
        destination_depth_order,
        auto_ingest_enabled: request.auto_ingest_enabled,
        auto_format_enabled: request.auto_ingest_enabled && request.auto_format_enabled,
    };
    match store.save_marker_ingest_profile(&profile) {
        Ok(()) => IpcResponse::success(CardRegistration {
            registered: true,
            auto_ingest_enabled: profile.auto_ingest_enabled,
            auto_ingest_already_completed: false,
            auto_format_enabled: profile.auto_format_enabled,
            destination_path: Some(profile.destination_path),
            sort_mode: ingest_sort_mode_from_name(&profile.sort_mode),
            interval_minutes: profile.interval_minutes,
            custom_directory_fields: profile.custom_directory_fields,
            destination_depth_order: profile.destination_depth_order,
            marker_status,
        }),
        Err(_) => IpcResponse::failure(
            IpcErrorCode::DeviceUnavailable,
            "The card registration could not be saved",
        ),
    }
}

/// Looks up an opt-in profile only after discovery has observed its exact
/// current marker. The marker remains convenience evidence, never a format
/// authorization or hardware-identity substitute.
#[tauri::command]
async fn get_auto_ingest_profile(
    source_medium_key: String,
    state: State<'_, AppState>,
) -> Result<IpcResponse<CardRegistration>, IpcError> {
    let store = Arc::clone(&state.store);
    let connection_generations = Arc::clone(&state.connection_generations);
    Ok(
        match tauri::async_runtime::spawn_blocking(move || {
            get_auto_ingest_profile_blocking(
                source_medium_key,
                store.as_ref(),
                connection_generations.as_ref(),
            )
        })
        .await
        {
            Ok(response) => response,
            Err(_) => IpcResponse::failure(
                IpcErrorCode::OperationCancelled,
                "The auto-ingest profile worker ended before completing the lookup",
            ),
        },
    )
}

fn get_auto_ingest_profile_blocking(
    source_medium_key: String,
    store: &Mutex<LocalStore>,
    connection_generations: &Mutex<ConnectionGenerationTracker>,
) -> IpcResponse<CardRegistration> {
    let snapshot = native_device_snapshot(store, connection_generations);
    let current = snapshot
        .devices
        .iter()
        .filter(|device| {
            device.state == DeviceState::Available
                && device.identity.media_key == source_medium_key
                && device.details.mount_locations.len() == 1
        })
        .collect::<Vec<_>>();
    if current.len() != 1 {
        return IpcResponse::success(CardRegistration {
            registered: false,
            auto_ingest_enabled: false,
            auto_ingest_already_completed: false,
            auto_format_enabled: false,
            destination_path: None,
            sort_mode: None,
            interval_minutes: None,
            custom_directory_fields: Vec::new(),
            destination_depth_order: Vec::new(),
            marker_status: SourceMarkerStatus::Unavailable,
        });
    }
    let root = PathBuf::from(&current[0].details.mount_locations[0]);
    let marker_token = match crate::storage_marker::read_marker(&root) {
        Ok(Some(token)) => token,
        _ => {
            return IpcResponse::success(CardRegistration {
                registered: false,
                auto_ingest_enabled: false,
                auto_ingest_already_completed: false,
                auto_format_enabled: false,
                destination_path: None,
                sort_mode: None,
                interval_minutes: None,
                custom_directory_fields: Vec::new(),
                destination_depth_order: Vec::new(),
                marker_status: SourceMarkerStatus::Unavailable,
            })
        }
    };
    let Ok(store) = store.lock() else {
        return IpcResponse::failure(
            IpcErrorCode::DeviceUnavailable,
            "The local card registration store is unavailable",
        );
    };
    match store.marker_ingest_profile(&marker_token) {
        Ok(Some(profile)) => {
            let already_completed = match store.has_completed_auto_ingest_for_source(
                &current[0].identity.media_key,
                current[0].connection_generation,
            ) {
                Ok(true) => true,
                Ok(false) => {
                    // A completed auto-format can briefly make this same
                    // mounted card appear as a fresh generation while the
                    // provider validates and restores its marker. Suppress
                    // only that empty post-format state. The next camera
                    // mount containing any source file remains eligible.
                    store
                        .has_completed_format_for_source(&current[0].identity.media_key)
                        .unwrap_or(false)
                        && enumerate_regular_files(&root)
                            .map(|files| files.is_empty())
                            .unwrap_or(false)
                }
                Err(_) => {
                    return IpcResponse::failure(
                        IpcErrorCode::DeviceUnavailable,
                        "The local auto-ingest history could not be read",
                    )
                }
            };
            IpcResponse::success(CardRegistration {
                registered: true,
                auto_ingest_enabled: profile.auto_ingest_enabled,
                // The native ledger, rather than a per-webview Set, prevents a
                // restart from copying the same mounted card again.
                auto_ingest_already_completed: already_completed,
                auto_format_enabled: profile.auto_format_enabled,
                destination_path: Some(profile.destination_path),
                sort_mode: ingest_sort_mode_from_name(&profile.sort_mode),
                interval_minutes: profile.interval_minutes,
                custom_directory_fields: profile.custom_directory_fields,
                destination_depth_order: profile.destination_depth_order,
                marker_status: SourceMarkerStatus::Recognized,
            })
        }
        Ok(None) => IpcResponse::success(CardRegistration {
            registered: false,
            auto_ingest_enabled: false,
            auto_ingest_already_completed: false,
            auto_format_enabled: false,
            destination_path: None,
            sort_mode: None,
            interval_minutes: None,
            custom_directory_fields: Vec::new(),
            destination_depth_order: Vec::new(),
            marker_status: SourceMarkerStatus::Recognized,
        }),
        Err(_) => IpcResponse::failure(
            IpcErrorCode::DeviceUnavailable,
            "The card registration could not be read",
        ),
    }
}

fn source_inventory_matches_current_device(
    snapshot: &DeviceSnapshot,
    request: &SourceInventoryRequest,
) -> bool {
    let mount_key = mount_match_key(&request.source_root);
    snapshot.devices.iter().any(|device| {
        device.state == DeviceState::Available
            && device.identity.media_key == request.source_medium_key
            && device
                .details
                .mount_locations
                .iter()
                .any(|mount| mount_match_key(mount) == mount_key)
    })
}

fn current_source_for_ingest_request<'a>(
    snapshot: &'a DeviceSnapshot,
    request: &VerifiedIngestRequest,
) -> Option<&'a StorageDevice> {
    let mount_key = mount_match_key(&request.source_root);
    snapshot.devices.iter().find(|device| {
        device.state == DeviceState::Available
            && device.identity.media_key == request.source_medium_key
            && device.connection_generation == request.source_generation
            && device
                .details
                .mount_locations
                .iter()
                .any(|mount| mount_match_key(mount) == mount_key)
    })
}

/// Recovery accepts the exact still-mounted insertion even when the host can
/// only observe mutable continuity evidence. A generation change is accepted
/// solely for a hardware-immutable identity; a marker, drive letter, or mount
/// must never make a removed/replaced card eligible for recovery.
fn current_source_for_recovery<'a>(
    snapshot: &'a DeviceSnapshot,
    recovery: &RecoverableIngestRun,
) -> Option<&'a StorageDevice> {
    let mount_key = mount_match_key(&recovery.source_root);
    snapshot.devices.iter().find(|device| {
        device.state == DeviceState::Available
            && device.identity.media_key == recovery.source_identity_key
            && device
                .details
                .mount_locations
                .iter()
                .any(|mount| mount_match_key(mount) == mount_key)
            && (device.connection_generation == recovery.source_generation
                || device.identity.confidence == IdentityConfidence::HardwareImmutable)
    })
}

/// Reports the current non-destructive quick-format gate without issuing an
/// authorization token or permitting a destructive operation.
#[tauri::command]
fn get_format_eligibility(
    request: FormatAuthorizationRequest,
    state: State<'_, AppState>,
) -> IpcResponse<FormatEligibility> {
    IpcResponse::success(evaluate_format_eligibility(&request, &state))
}

/// Eject is permitted only after an exact completed receipt is sealed for the
/// currently re-observed card, and never while this app is still ingesting
/// from it. Unlike format, safe removal is non-destructive, so a current
/// uniquely-resolved mount/generation is sufficient even where the reader
/// cannot expose an immutable card identifier. The native provider receives
/// the resolved mount, not an IPC path, and must acquire an exclusive OS
/// handle before ejecting.
#[tauri::command]
async fn safe_eject(
    request: SafeEjectRequest,
    state: State<'_, AppState>,
) -> Result<IpcResponse<SafeEjectResult>, IpcError> {
    let mount_root = match safe_eject_mount(&request, &state) {
        Ok(mount_root) => mount_root,
        Err(message) => {
            return Ok(IpcResponse::failure(
                IpcErrorCode::DeviceUnavailable,
                message,
            ));
        }
    };
    Ok(
        match tauri::async_runtime::spawn_blocking(move || {
            crate::eject_provider::safe_eject(&mount_root)
        })
        .await
        {
            Ok(Ok(())) => IpcResponse::success(SafeEjectResult {
                source_medium_key: request.source_medium_key,
                source_generation: request.source_generation,
            }),
            Ok(Err(crate::eject_provider::SafeEjectError::UnsupportedPlatform)) => {
                IpcResponse::failure(
                    IpcErrorCode::UnsupportedPlatform,
                    "Safe eject is not available on this platform yet",
                )
            }
            Ok(Err(crate::eject_provider::SafeEjectError::DeviceBusy(veto_name))) => {
                let detail = veto_name
                    .filter(|value| !value.trim().is_empty())
                    .map(|value| format!(" Windows reported: {value}."))
                    .unwrap_or_default();
                IpcResponse::failure(
                    IpcErrorCode::DeviceUnavailable,
                    format!(
                        "Windows could not safely remove this card because it is still in use. Close any app or file browser using it, then try again.{detail}"
                    ),
                )
            }
            Ok(Err(crate::eject_provider::SafeEjectError::DeviceNotEjectable)) => {
                IpcResponse::failure(
                    IpcErrorCode::DeviceUnavailable,
                    "Windows could not safely remove this card device; remove it manually only after Windows reports it safe",
                )
            }
            Ok(Err(crate::eject_provider::SafeEjectError::EjectFailed)) | Err(_) => {
                IpcResponse::failure(
                    IpcErrorCode::DeviceUnavailable,
                    "Windows could not confirm that this card was safely ejected",
                )
            }
        },
    )
}

fn safe_eject_mount(request: &SafeEjectRequest, state: &AppState) -> Result<PathBuf, &'static str> {
    if request.run_id.trim().is_empty() || request.source_medium_key.trim().is_empty() {
        return Err("A completed ingest receipt and current source card are required");
    }
    if state
        .active_ingests
        .lock()
        .map(|active| {
            active
                .values()
                .any(|ingest| ingest.source_medium_key == request.source_medium_key)
        })
        .unwrap_or(true)
    {
        return Err("The source card is still participating in an ingest");
    }
    let has_receipt = state
        .store
        .lock()
        .ok()
        .and_then(|store| {
            store
                .has_completed_receipt_for_source(
                    &request.run_id,
                    &request.source_medium_key,
                    request.source_generation,
                )
                .ok()
        })
        .unwrap_or(false);
    if !has_receipt {
        return Err("No sealed completed receipt matches the current source card");
    }
    let snapshot = native_device_snapshot(&state.store, &state.connection_generations);
    let devices = snapshot
        .devices
        .iter()
        .filter(|device| {
            device.state == DeviceState::Available
                && device.identity.media_key == request.source_medium_key
                && device.connection_generation == request.source_generation
                && device.details.mount_locations.len() == 1
        })
        .collect::<Vec<_>>();
    if devices.len() != 1 {
        return Err("The verified source card is no longer uniquely present");
    }
    Ok(PathBuf::from(&devices[0].details.mount_locations[0]))
}

/// Establishes the non-destructive half of the quick-format protocol. A
/// future platform provider must consume this single-use token and repeat its
/// own exact-device check after privilege elevation; it must never accept a
/// UI-provided mount path or command line.
#[tauri::command]
fn request_format_authorization(
    request: FormatAuthorizationRequest,
    state: State<'_, AppState>,
) -> IpcResponse<FormatAuthorizationResult> {
    let eligibility = evaluate_format_eligibility(&request, &state);
    if !eligibility.eligible {
        return IpcResponse::failure(IpcErrorCode::DeviceUnavailable, eligibility.reason);
    }
    let profile = eligibility
        .recommended_profile
        .as_ref()
        .expect("eligible format authorization has an allowlisted profile");
    let authorization = match issue_authorization(
        &request.source_medium_key,
        request.source_generation,
        &request.run_id,
        profile.id,
        true,
        true,
        std::time::SystemTime::now(),
    ) {
        Ok(authorization) => authorization,
        Err(_) => {
            return IpcResponse::failure(
                IpcErrorCode::DeviceUnavailable,
                "The quick-format authorization could not be created",
            )
        }
    };
    let confirmation_token = authorization.token.clone();
    let Ok(mut authorizations) = state.format_authorizations.lock() else {
        return IpcResponse::failure(
            IpcErrorCode::DeviceUnavailable,
            "The quick-format authorization store is unavailable",
        );
    };
    authorizations.retain(|_, existing| existing.expires_at > std::time::SystemTime::now());
    authorizations.insert(confirmation_token.clone(), authorization);
    IpcResponse::success(FormatAuthorizationResult {
        confirmation_token,
        expires_in_seconds: 60,
    })
}

#[tauri::command]
fn request_force_format_authorization(
    request: ForceFormatAuthorizationRequest,
    state: State<'_, AppState>,
) -> IpcResponse<FormatAuthorizationResult> {
    if request.confirmation_phrase.trim() != "FORCE REFORMAT" {
        return IpcResponse::failure(
            IpcErrorCode::InvalidRequest,
            "Type FORCE REFORMAT exactly before preparing this recovery action",
        );
    }
    let snapshot = native_device_snapshot(&state.store, &state.connection_generations);
    let current = snapshot
        .devices
        .iter()
        .filter(|device| {
            device.state == DeviceState::Available
                && device.identity.media_key == request.source_medium_key
                && device.connection_generation == request.source_generation
                && device.details.mount_locations.len() == 1
        })
        .collect::<Vec<_>>();
    let Some(device) = (current.len() == 1).then(|| current[0]) else {
        return IpcResponse::failure(
            IpcErrorCode::DeviceUnavailable,
            "The selected card is no longer uniquely present",
        );
    };
    if state
        .active_ingests
        .lock()
        .map(|active| {
            active
                .values()
                .any(|ingest| ingest.source_medium_key == request.source_medium_key)
        })
        .unwrap_or(true)
    {
        return IpcResponse::failure(
            IpcErrorCode::DeviceUnavailable,
            "The source medium is still participating in an ingest",
        );
    }
    let Some(profile) = recommended_profile(device.details.total_bytes) else {
        return IpcResponse::failure(
            IpcErrorCode::DeviceUnavailable,
            "No safe generic format profile is available for this card capacity",
        );
    };
    let run_id = format!("force-reformat:{}", uuid::Uuid::new_v4());
    let authorization = match issue_authorization(
        &request.source_medium_key,
        request.source_generation,
        &run_id,
        profile.id,
        true,
        true,
        std::time::SystemTime::now(),
    ) {
        Ok(authorization) => authorization,
        Err(_) => {
            return IpcResponse::failure(
                IpcErrorCode::DeviceUnavailable,
                "The force-reformat authorization could not be created",
            )
        }
    };
    let confirmation_token = authorization.token.clone();
    let Ok(mut authorizations) = state.format_authorizations.lock() else {
        return IpcResponse::failure(
            IpcErrorCode::DeviceUnavailable,
            "The quick-format authorization store is unavailable",
        );
    };
    authorizations.retain(|_, existing| existing.expires_at > std::time::SystemTime::now());
    authorizations.insert(confirmation_token.clone(), authorization);
    IpcResponse::success(FormatAuthorizationResult {
        confirmation_token,
        expires_in_seconds: 60,
    })
}

/// Consumes a fresh confirmation token and performs the native-only format
/// sequence. The token is removed before the second device observation, so a
/// disappearing card, an expired token, or a failed provider attempt can
/// never be retried by replaying the same confirmation.
#[tauri::command]
fn execute_format_authorization(
    request: ExecuteFormatAuthorizationRequest,
    state: State<'_, AppState>,
) -> IpcResponse<FormatExecutionResult> {
    if request.confirmation_token.trim().is_empty() {
        return IpcResponse::failure(
            IpcErrorCode::InvalidRequest,
            "A fresh quick-format confirmation is required",
        );
    }
    let preview = match state.format_authorizations.lock() {
        Ok(authorizations) => authorizations.get(&request.confirmation_token).cloned(),
        Err(_) => None,
    };
    let Some(preview) = preview else {
        return IpcResponse::failure(
            IpcErrorCode::DeviceUnavailable,
            "The quick-format confirmation is no longer available",
        );
    };
    let snapshot = native_device_snapshot(&state.store, &state.connection_generations);
    let matching = snapshot
        .devices
        .iter()
        .filter(|device| {
            device.state == DeviceState::Available
                && device.identity.media_key == preview.medium_key
                && device.connection_generation == preview.generation
                && device.details.mount_locations.len() == 1
        })
        .collect::<Vec<_>>();
    let Some(device) = (matching.len() == 1).then(|| matching[0]) else {
        // Consume on a changed or unavailable device, even before a provider
        // is reached, so no stale UI confirmation can be replayed.
        if let Ok(mut authorizations) = state.format_authorizations.lock() {
            let _ = consume_authorization(
                &mut authorizations,
                &request.confirmation_token,
                &preview.medium_key,
                preview.generation,
                std::time::SystemTime::now(),
            );
        }
        return IpcResponse::failure(
            IpcErrorCode::DeviceUnavailable,
            "The confirmed card is no longer uniquely present",
        );
    };
    let Some(profile) = recommended_profile(device.details.total_bytes) else {
        return IpcResponse::failure(
            IpcErrorCode::DeviceUnavailable,
            "No safe generic format profile is available for this card capacity",
        );
    };
    let Ok(mut authorizations) = state.format_authorizations.lock() else {
        return IpcResponse::failure(
            IpcErrorCode::DeviceUnavailable,
            "The quick-format authorization store is unavailable",
        );
    };
    let authorization = match consume_authorization(
        &mut authorizations,
        &request.confirmation_token,
        &device.identity.media_key,
        device.connection_generation,
        std::time::SystemTime::now(),
    ) {
        Ok(authorization) => authorization,
        Err(_) => {
            return IpcResponse::failure(
                IpcErrorCode::DeviceUnavailable,
                "The quick-format confirmation expired or no longer matches this card",
            )
        }
    };
    drop(authorizations);
    if authorization.profile_id != profile.id {
        return IpcResponse::failure(
            IpcErrorCode::DeviceUnavailable,
            "The card capacity changed after confirmation",
        );
    }
    let has_receipt = state.store.lock().ok().and_then(|store| {
        store
            .has_completed_receipt_for_source(
                &authorization.run_id,
                &authorization.medium_key,
                authorization.generation,
            )
            .ok()
    }) == Some(true);
    if !authorization.force_reformat && !has_receipt {
        return IpcResponse::failure(
            IpcErrorCode::DeviceUnavailable,
            "The sealed verification receipt is no longer current for this card",
        );
    }
    let managed_card_matches = state
        .store
        .lock()
        .map(|store| managed_card_matches_sealed_receipt(&store, &authorization.run_id, device))
        .unwrap_or(false);
    if !authorization.force_reformat && !managed_card_matches {
        return IpcResponse::failure(
            IpcErrorCode::DeviceUnavailable,
            "The registered managed-card witness no longer matches the sealed receipt",
        );
    }
    let Some(capacity) = device.details.total_bytes else {
        return IpcResponse::failure(
            IpcErrorCode::DeviceUnavailable,
            "Card capacity is unavailable",
        );
    };
    let expected = crate::format_provider::ExpectedFormatTarget {
        medium_key: authorization.medium_key.clone(),
        connection_generation: authorization.generation,
        expected_capacity_bytes: capacity,
        current_mount_root: PathBuf::from(&device.details.mount_locations[0]),
    };
    let pre_format_device = device.clone();
    let profile_id = profile.id.to_owned();
    // Preserve an optional registered-card marker before the provider erases
    // the filesystem. It is restored only after remount and exact immutable
    // identity revalidation below.
    let marker_token = crate::storage_marker::read_marker(&expected.current_mount_root)
        .ok()
        .flatten()
        .and_then(|marker| {
            state
                .store
                .lock()
                .ok()
                .and_then(|store| store.marker_ingest_profile(&marker).ok().flatten())
                .map(|registered| registered.marker_token)
        });
    let profile_id_for_receipt = profile_id.clone();
    let store = Arc::clone(&state.store);
    let connection_generations = Arc::clone(&state.connection_generations);
    let run_id = authorization.run_id.clone();
    let medium_key = authorization.medium_key.clone();
    let generation = authorization.generation;
    let outcome: Result<bool, crate::format_provider::FormatProviderError> = (move || {
        let provider = crate::format_provider::current_platform_provider();
        let target = provider.resolve_exact_target(&expected)?;
        provider.quick_format(&target, &profile)?;
        let validated = provider.wait_for_validated_mount(&expected, &profile)?;
        // The managed marker must be gone before this workflow can claim a
        // format occurred. A matching exFAT mount alone is not evidence: the
        // pre-format card can have that same filesystem.
        if crate::storage_marker::read_record(&validated.root)
            .map_err(|_| crate::format_provider::FormatProviderError::ValidationFailed)?
            .is_some()
        {
            return Err(crate::format_provider::FormatProviderError::ValidationFailed);
        }
        sentinel_round_trip(&validated.root)
            .map_err(|_| crate::format_provider::FormatProviderError::ValidationFailed)?;
        let post_format = native_device_snapshot(&store, &connection_generations);
        let exact_target_present = post_format.devices.iter().any(|device| {
            if authorization.force_reformat {
                same_force_format_target(device, &validated.root, capacity)
            } else {
                same_managed_format_target(&pre_format_device, device, &validated.root, capacity)
            }
        });
        if !exact_target_present {
            return Err(crate::format_provider::FormatProviderError::TargetChanged);
        }
        let marker_restored = match marker_token {
            Some(marker_token) => {
                crate::storage_marker::restore_marker(&validated.root, &marker_token)
                    .map(|_| true)
                    .map_err(|_| crate::format_provider::FormatProviderError::ValidationFailed)?
            }
            None => false,
        };
        if !authorization.force_reformat {
            let recorded = store.lock().ok().and_then(|mut store| {
                store
                    .record_completed_format(
                        &run_id,
                        &medium_key,
                        generation,
                        &profile_id_for_receipt,
                    )
                    .ok()
            }) == Some(true);
            if !recorded {
                return Err(crate::format_provider::FormatProviderError::ValidationFailed);
            }
        }
        Ok(marker_restored)
    })();
    match outcome {
        Ok(marker_restored) => IpcResponse::success(FormatExecutionResult {
            profile_id,
            marker_restored,
        }),
        Err(error) => IpcResponse::failure(
            IpcErrorCode::DeviceUnavailable,
            format_provider_error_message(error),
        ),
    }
}

fn format_provider_error_message(error: crate::format_provider::FormatProviderError) -> String {
    match error {
        crate::format_provider::FormatProviderError::UnsupportedPlatform => {
            "Quick format is not available on this platform".into()
        }
        crate::format_provider::FormatProviderError::TargetUnavailable
        | crate::format_provider::FormatProviderError::TargetChanged => {
            "The confirmed card changed or is no longer available".into()
        }
        crate::format_provider::FormatProviderError::TargetReopenFailed => {
            "Windows could not reopen the exact native volume target".into()
        }
        crate::format_provider::FormatProviderError::TargetCapacityMismatch => {
            "Windows reported a different capacity for the confirmed volume".into()
        }
        crate::format_provider::FormatProviderError::NotRemovable => {
            "The selected device is not removable media".into()
        }
        crate::format_provider::FormatProviderError::WriteProtected => {
            "The card is write-protected".into()
        }
        crate::format_provider::FormatProviderError::Busy => {
            "The card is in use; close applications using it and try again".into()
        }
        crate::format_provider::FormatProviderError::FormatInputFailed => {
            "Windows rejected the documented native format input".into()
        }
        crate::format_provider::FormatProviderError::FormatOutputMissing => {
            "Windows returned no native format result".into()
        }
        crate::format_provider::FormatProviderError::FormatResultUnreadable => {
            "Windows returned an unreadable native format result".into()
        }
        crate::format_provider::FormatProviderError::FormatFailed => {
            "Windows could not quick-format the confirmed card".into()
        }
        crate::format_provider::FormatProviderError::FormatFailedWithCode(code) => {
            format!("Windows native quick format failed with status 0x{code:08X}")
        }
        crate::format_provider::FormatProviderError::RemountFailed => {
            "The card did not remount after quick format".into()
        }
        crate::format_provider::FormatProviderError::ValidationFailed => {
            "Quick format completed but the post-format validation did not pass".into()
        }
    }
}

fn evaluate_format_eligibility(
    request: &FormatAuthorizationRequest,
    state: &AppState,
) -> FormatEligibility {
    let blocked = |reason: &str| FormatEligibility {
        eligible: false,
        reason: reason.into(),
        recommended_profile: None,
    };
    if request.run_id.trim().is_empty() || request.source_medium_key.trim().is_empty() {
        return blocked("A completed ingest run and current source medium are required");
    }
    let snapshot = native_device_snapshot(&state.store, &state.connection_generations);
    let current_devices = snapshot
        .devices
        .iter()
        .filter(|device| {
            device.state == DeviceState::Available
                && device.identity.media_key == request.source_medium_key
                && device.connection_generation == request.source_generation
                && device.details.mount_locations.len() == 1
        })
        .collect::<Vec<_>>();
    if current_devices.len() != 1 {
        return blocked("The verified source medium is no longer uniquely present");
    }
    let Some(profile) = recommended_profile(current_devices[0].details.total_bytes) else {
        return blocked("No safe generic format profile is available for this card capacity");
    };
    if state
        .active_ingests
        .lock()
        .map(|active| {
            active
                .values()
                .any(|ingest| ingest.source_medium_key == request.source_medium_key)
        })
        .unwrap_or(true)
    {
        return blocked("The source medium is still participating in an ingest");
    }
    let managed_card_matches = match state.store.lock() {
        Ok(store) => {
            managed_card_matches_sealed_receipt(&store, &request.run_id, current_devices[0])
        }
        Err(_) => false,
    };
    if !managed_card_matches {
        return blocked(
            "This card is not the current registered managed card for the sealed receipt",
        );
    }
    FormatEligibility {
        eligible: true,
        reason: "Receipt and current managed-card witness are ready for the platform formatter"
            .into(),
        recommended_profile: Some(profile),
    }
}

fn native_device_snapshot(
    store: &Mutex<LocalStore>,
    connection_generations: &Mutex<ConnectionGenerationTracker>,
) -> DeviceSnapshot {
    let discovery = NativeDeviceDiscovery;
    let sequence = SNAPSHOT_SEQUENCE.fetch_add(1, Ordering::Relaxed) + 1;
    let mut snapshot = snapshot_from_volumes(sequence, discovery.enumerate_removable_volumes());
    let persisted_generations = if let Ok(store) = store.lock() {
        apply_calibrated_slots(&mut snapshot, &store);
        store.latest_source_generations().unwrap_or_default()
    } else {
        Vec::new()
    };
    if let Ok(mut tracker) = connection_generations.lock() {
        seed_connection_generations(&mut tracker, persisted_generations);
        assign_connection_generations(&mut snapshot, &mut tracker);
    }
    snapshot
}

async fn blocking_native_device_snapshot(
    store: Arc<Mutex<LocalStore>>,
    connection_generations: Arc<Mutex<ConnectionGenerationTracker>>,
) -> Result<DeviceSnapshot, ()> {
    tauri::async_runtime::spawn_blocking(move || {
        native_device_snapshot(&store, &connection_generations)
    })
    .await
    .map_err(|_| ())
}

fn seed_connection_generations(
    tracker: &mut ConnectionGenerationTracker,
    persisted_generations: impl IntoIterator<Item = (String, u64)>,
) {
    if tracker.seeded_from_store {
        return;
    }
    for (identity_key, generation) in persisted_generations {
        tracker.next_generation = tracker.next_generation.max(generation);
        tracker.present.insert(identity_key, generation);
    }
    tracker.seeded_from_store = true;
}

fn assign_connection_generations(
    snapshot: &mut DeviceSnapshot,
    tracker: &mut ConnectionGenerationTracker,
) {
    let present = snapshot
        .devices
        .iter()
        .filter(|device| device.state == DeviceState::Available)
        .map(|device| device.identity.media_key.clone())
        .collect::<HashSet<_>>();
    tracker
        .present
        .retain(|medium_key, _| present.contains(medium_key));
    for device in &mut snapshot.devices {
        if device.state != DeviceState::Available {
            device.connection_generation = 0;
            continue;
        }
        let generation = *tracker
            .present
            .entry(device.identity.media_key.clone())
            .or_insert_with(|| {
                tracker.next_generation = tracker.next_generation.saturating_add(1);
                tracker.next_generation
            });
        device.connection_generation = generation;
    }
}

fn mount_match_key(value: &str) -> String {
    let normalized = value.trim().replace('\\', "/");
    let trimmed = normalized.trim_end_matches('/');
    if trimmed.is_empty() {
        "/".into()
    } else {
        trimmed.to_ascii_lowercase()
    }
}

/// A watcher only cancels a run when its source was previously witnessed and
/// is no longer represented by the same medium key and mount root. This is a
/// cooperative stop: the copy layer still independently detects I/O failure
/// and source changes, while the cancellation flag is checked between chunks.
fn cancel_missing_active_sources(
    active_ingests: &Mutex<HashMap<String, ActiveIngest>>,
    snapshot: &DeviceSnapshot,
) {
    let available_sources = snapshot
        .devices
        .iter()
        .filter(|device| device.state == DeviceState::Available)
        .flat_map(|device| {
            device
                .details
                .mount_locations
                .iter()
                .map(move |mount| (&device.identity.media_key, mount_match_key(mount)))
        })
        .collect::<HashSet<_>>();
    let Ok(mut active_ingests) = active_ingests.lock() else {
        return;
    };
    for active in active_ingests.values_mut() {
        let source_present = available_sources.contains(&(
            &active.source_medium_key,
            mount_match_key(&active.source_root),
        ));
        if source_present {
            active.source_seen_in_snapshot = true;
        } else if active.source_seen_in_snapshot {
            active.cancellation.store(true, Ordering::Release);
        }
    }
}

fn has_active_ingests(active_ingests: &Mutex<HashMap<String, ActiveIngest>>) -> bool {
    active_ingests
        .lock()
        .map(|active| !active.is_empty())
        .unwrap_or(true)
}

fn send_reconciled_device_snapshot(
    channel: &Channel<DeviceSnapshot>,
    store: &Mutex<LocalStore>,
    connection_generations: &Mutex<ConnectionGenerationTracker>,
    active_ingests: &Mutex<HashMap<String, ActiveIngest>>,
) -> bool {
    let snapshot = native_device_snapshot(store, connection_generations);
    cancel_missing_active_sources(active_ingests, &snapshot);
    channel.send(snapshot).is_ok()
}

/// Windows uses Configuration Manager device-interface notifications rather
/// than periodic drive-letter polling. The subscription is registered before
/// its initial enumeration, so arrival/removal races are reconciled by a fresh
/// native snapshot rather than inferred from the event payload.
#[cfg(windows)]
#[tauri::command]
fn watch_device_snapshots(
    channel: Channel<DeviceSnapshot>,
    state: State<'_, AppState>,
) -> IpcResponse<()> {
    use std::sync::mpsc::{sync_channel, RecvTimeoutError};
    use std::time::Duration;

    let store = Arc::clone(&state.store);
    let connection_generations = Arc::clone(&state.connection_generations);
    let active_ingests = Arc::clone(&state.active_ingests);
    let (ready_tx, ready_rx) = sync_channel(1);
    std::thread::spawn(move || {
        let subscription =
            match device_discovery::DeviceChangeSubscription::register_disk_interfaces() {
                Ok(subscription) => {
                    let _ = ready_tx.send(true);
                    subscription
                }
                Err(_) => {
                    let _ = ready_tx.send(false);
                    return;
                }
            };
        if !send_reconciled_device_snapshot(
            &channel,
            &store,
            &connection_generations,
            &active_ingests,
        ) {
            return;
        }
        while let Ok(()) | Err(std::sync::mpsc::RecvTimeoutError::Timeout) =
            subscription.recv_timeout(Duration::from_secs(1))
        {
            if !send_reconciled_device_snapshot(
                &channel,
                &store,
                &connection_generations,
                &active_ingests,
            ) {
                break;
            }
        }
    });
    match ready_rx.recv_timeout(Duration::from_secs(2)) {
        Ok(true) => IpcResponse::success(()),
        Ok(false) => IpcResponse::failure(
            IpcErrorCode::DeviceUnavailable,
            "Windows storage change monitoring could not be registered",
        ),
        Err(RecvTimeoutError::Timeout) => IpcResponse::failure(
            IpcErrorCode::DeviceUnavailable,
            "Windows storage change monitoring did not initialize in time",
        ),
        Err(RecvTimeoutError::Disconnected) => IpcResponse::failure(
            IpcErrorCode::DeviceUnavailable,
            "Windows storage change monitoring ended before initialization",
        ),
    }
}

/// macOS registers Disk Arbitration callbacks before its initial snapshot.
/// The callback carries no disk target: it merely wakes this worker to take a
/// fresh native snapshot. The existing structured `diskutil` adapter remains
/// a bounded fallback until native description-to-volume projection has
/// hardware evidence.
#[cfg(target_os = "macos")]
#[tauri::command]
fn watch_device_snapshots(
    channel: Channel<DeviceSnapshot>,
    state: State<'_, AppState>,
) -> IpcResponse<()> {
    use std::sync::mpsc::{sync_channel, RecvTimeoutError};

    let store = Arc::clone(&state.store);
    let connection_generations = Arc::clone(&state.connection_generations);
    let active_ingests = Arc::clone(&state.active_ingests);
    let native_channel = channel.clone();
    let (ready_tx, ready_rx) = sync_channel(1);
    std::thread::spawn(move || {
        let (subscription, events) = match macos_disk_arbitration::subscribe(1) {
            Ok(subscription) => {
                let _ = ready_tx.send(true);
                subscription
            }
            Err(error) => {
                warn!(
                    target: "media_ingest::macos::discovery",
                    "Disk Arbitration lifecycle subscription unavailable; using bounded snapshot fallback ({error})"
                );
                let _ = ready_tx.send(false);
                return;
            }
        };
        if !send_reconciled_device_snapshot(
            &native_channel,
            &store,
            &connection_generations,
            &active_ingests,
        ) {
            return;
        }
        loop {
            match events.recv_timeout(Duration::from_secs(30)) {
                Ok(request) => {
                    if !send_reconciled_device_snapshot(
                        &native_channel,
                        &store,
                        &connection_generations,
                        &active_ingests,
                    ) {
                        break;
                    }
                    subscription.acknowledge(request);
                }
                // A periodic snapshot catches missed callbacks, queue loss,
                // and sleep/wake without making polling the primary detector.
                Err(RecvTimeoutError::Timeout) => {
                    if !send_reconciled_device_snapshot(
                        &native_channel,
                        &store,
                        &connection_generations,
                        &active_ingests,
                    ) {
                        break;
                    }
                }
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
    });
    match ready_rx.recv_timeout(Duration::from_secs(2)) {
        Ok(true) => IpcResponse::success(()),
        // Preserve a usable mounted-volume inventory if the native lifecycle
        // service is unavailable. This still runs solely in Rust and never
        // trusts a webview-supplied mount path.
        Ok(false) | Err(RecvTimeoutError::Timeout) | Err(RecvTimeoutError::Disconnected) => {
            let store = Arc::clone(&state.store);
            let connection_generations = Arc::clone(&state.connection_generations);
            let active_ingests = Arc::clone(&state.active_ingests);
            std::thread::spawn(move || loop {
                if !send_reconciled_device_snapshot(
                    &channel,
                    &store,
                    &connection_generations,
                    &active_ingests,
                ) {
                    break;
                }
                std::thread::sleep(Duration::from_secs(30));
            });
            IpcResponse::success(())
        }
    }
}

/// Linux does not have an event subscription wired into this binary yet, but
/// it must still drive registered-card auto-ingest. Reconcile the native
/// snapshot at a bounded cadence rather than treating a one-time UI refresh as
/// a mount event. The worker holds no target supplied by the webview and ends
/// as soon as its channel is closed.
#[cfg(all(not(windows), not(target_os = "macos")))]
#[tauri::command]
fn watch_device_snapshots(
    channel: Channel<DeviceSnapshot>,
    state: State<'_, AppState>,
) -> IpcResponse<()> {
    use std::time::Duration;

    let store = Arc::clone(&state.store);
    let connection_generations = Arc::clone(&state.connection_generations);
    let active_ingests = Arc::clone(&state.active_ingests);
    std::thread::spawn(move || loop {
        if !send_reconciled_device_snapshot(
            &channel,
            &store,
            &connection_generations,
            &active_ingests,
        ) {
            break;
        }
        std::thread::sleep(Duration::from_secs(1));
    });
    IpcResponse::success(())
}

#[tauri::command]
fn calibrate_reader_slot(
    request: ReaderSlotCalibrationRequest,
    state: State<'_, AppState>,
) -> IpcResponse<()> {
    if request.reader_fingerprint.trim().is_empty() || request.evidence_note.trim().is_empty() {
        return IpcResponse::failure(
            IpcErrorCode::InvalidRequest,
            "A reader fingerprint and controlled-insertion evidence note are required",
        );
    }
    let Ok(mut store) = state.store.lock() else {
        return IpcResponse::failure(
            IpcErrorCode::DeviceUnavailable,
            "The local calibration store is unavailable",
        );
    };
    let slot_kind = match request.slot_kind {
        CalibratedSlotKind::Sd => ReaderSlotKind::Sd,
        CalibratedSlotKind::MicroSd => ReaderSlotKind::MicroSd,
    };
    match store.save_reader_slot_calibration(
        &request.reader_fingerprint,
        request.logical_unit,
        slot_kind,
        &request.evidence_note,
    ) {
        Ok(()) => IpcResponse::success(()),
        Err(_) => IpcResponse::failure(
            IpcErrorCode::DeviceUnavailable,
            "The local calibration could not be saved",
        ),
    }
}

#[tauri::command]
async fn preview_verified_ingest(
    mut request: VerifiedIngestRequest,
    state: State<'_, AppState>,
) -> Result<IpcResponse<IngestPlanPreview>, IpcError> {
    let snapshot = match blocking_native_device_snapshot(
        Arc::clone(&state.store),
        Arc::clone(&state.connection_generations),
    )
    .await
    {
        Ok(snapshot) => snapshot,
        Err(()) => {
            return Ok(IpcResponse::failure(
                IpcErrorCode::OperationCancelled,
                "The native device snapshot worker ended before planning",
            ))
        }
    };
    let Some(observed) = current_source_for_ingest_request(&snapshot, &request) else {
        return Ok(IpcResponse::failure(
            IpcErrorCode::DeviceUnavailable,
            "The selected card changed, was removed, or remounted before planning",
        ));
    };
    request.source_identity_confidence = observed.identity.confidence.clone();
    let operation_id = request
        .operation_id
        .as_deref()
        .and_then(|value| uuid::Uuid::parse_str(value).ok())
        .map(|value| value.to_string())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    Ok(
        match tauri::async_runtime::spawn_blocking(move || {
            prepare_verified_ingest(request, operation_id.clone()).map(|prepared| {
                IngestPlanPreview {
                    operation_id,
                    file_count: prepared.files.len(),
                    total_bytes: prepared
                        .files
                        .iter()
                        .map(|file| file.source.byte_length)
                        .sum(),
                    sample_destination_paths: prepared
                        .files
                        .iter()
                        .take(3)
                        .map(|file| {
                            file.destination_relative_path
                                .to_string_lossy()
                                .replace('\\', "/")
                        })
                        .collect(),
                }
            })
        })
        .await
        {
            Ok(Ok(preview)) => IpcResponse::success(preview),
            Ok(Err(error)) => IpcResponse::failure(
                IpcErrorCode::InvalidRequest,
                match error {
                    IngestError::UnsafeSourceEntry => {
                        "Source contains a symlink or reparse point; it was not planned"
                    }
                    IngestError::SourceChanged => "Source media changed while planning",
                    IngestError::SourceLimitExceeded => {
                        "The source exceeds the safe media inventory limit"
                    }
                    _ => "The ingest plan could not be created safely",
                },
            ),
            Err(_) => IpcResponse::failure(
                IpcErrorCode::OperationCancelled,
                "The ingest planner ended before completing the preview",
            ),
        },
    )
}

/// Reconciles a crash-interrupted run against the exact frozen plan. This
/// command intentionally has no source/destination paths: it reloads them
/// from the native store and refuses a changed medium, generation, or mount.
#[tauri::command]
async fn resume_verified_ingest(
    request: ResumeVerifiedIngestRequest,
    channel: Channel<ProgressUpdate>,
    state: State<'_, AppState>,
) -> Result<IpcResponse<VerifiedIngestResult>, IpcError> {
    let (recovery, planned) = {
        let Ok(store) = state.store.lock() else {
            return Ok(persistence_failure());
        };
        let Some(recovery) =
            store
                .recoverable_ingest_run(&request.run_id)
                .map_err(|_| IpcError {
                    code: IpcErrorCode::DeviceUnavailable,
                    message: "The local recovery ledger is unavailable".into(),
                    os_error: None,
                })?
        else {
            return Ok(IpcResponse::failure(
                IpcErrorCode::InvalidRequest,
                "This ingest run is not eligible for explicit recovery",
            ));
        };
        let planned = store
            .recovery_planned_files(&request.run_id)
            .map_err(|_| IpcError {
                code: IpcErrorCode::DeviceUnavailable,
                message: "The frozen recovery plan is unavailable".into(),
                os_error: None,
            })?;
        (recovery, planned)
    };
    if planned.is_empty() {
        return Ok(IpcResponse::failure(
            IpcErrorCode::InvalidRequest,
            "The interrupted run has no complete frozen file plan",
        ));
    }
    let snapshot = match blocking_native_device_snapshot(
        Arc::clone(&state.store),
        Arc::clone(&state.connection_generations),
    )
    .await
    {
        Ok(snapshot) => snapshot,
        Err(()) => return Ok(persistence_failure()),
    };
    let Some(observed) = current_source_for_recovery(&snapshot, &recovery) else {
        return Ok(IpcResponse::failure(
            IpcErrorCode::DeviceUnavailable,
            "The exact source card generation is not currently mounted for recovery",
        ));
    };
    let prepared = match prepare_recovery_ingest(&recovery, planned, request.max_workers, observed)
    {
        Ok(prepared) => prepared,
        Err(error) => return Ok(error_response(error)),
    };
    let operation_id = recovery.run_id.clone();
    let planned_bytes = prepared
        .files
        .iter()
        .map(|file| file.source.byte_length)
        .sum::<u64>();
    let preflight_source_root = prepared.source_root.clone();
    let preflight_destination_root = prepared.destination_root.clone();
    let destination_key = match tauri::async_runtime::spawn_blocking(move || {
        validate_ingest_roots(&preflight_source_root, &preflight_destination_root)
            .and_then(|()| has_destination_space(&preflight_destination_root, planned_bytes))
            .and_then(|has_space| {
                has_space
                    .then_some(())
                    .ok_or(IngestError::InsufficientDestinationSpace)
            })
            .and_then(|()| destination_lease_key(&preflight_destination_root))
    })
    .await
    {
        Ok(Ok(key)) => key,
        Ok(Err(error)) => return Ok(error_response(error)),
        Err(_) => return Ok(persistence_failure()),
    };
    let cancellation = Arc::new(std::sync::atomic::AtomicBool::new(false));
    {
        let Ok(mut active_ingests) = state.active_ingests.lock() else {
            return Ok(persistence_failure());
        };
        if active_ingests.contains_key(&operation_id) {
            return Ok(IpcResponse::failure(
                IpcErrorCode::InvalidRequest,
                "This recovery is already active",
            ));
        }
        active_ingests.insert(
            operation_id.clone(),
            ActiveIngest {
                cancellation: Arc::clone(&cancellation),
                source_medium_key: recovery.source_identity_key.clone(),
                source_root: recovery.source_root.clone(),
                source_seen_in_snapshot: false,
            },
        );
    }
    let _cancellation_registration = CancellationRegistration {
        active_ingests: Arc::clone(&state.active_ingests),
        operation_id: operation_id.clone(),
    };
    {
        let Ok(mut store) = state.store.lock() else {
            return Ok(persistence_failure());
        };
        if !store
            .begin_explicit_recovery(&operation_id, observed.connection_generation)
            .unwrap_or(false)
        {
            return Ok(persistence_failure());
        }
    }
    send_progress(
        &channel,
        &operation_id,
        IngestState::Copying,
        0,
        Some(planned_bytes),
    );
    let copy_channel = channel.clone();
    let progress_operation_id = operation_id.clone();
    let copied_transferred = Arc::new(AtomicU64::new(0));
    let copied_by_worker = Arc::clone(&copied_transferred);
    let verified_transferred = Arc::new(AtomicU64::new(0));
    let verified_by_worker = Arc::clone(&verified_transferred);
    let destination_leases = Arc::clone(&state.destination_leases);
    let progress_gate = Arc::new(ProgressEmissionGate {
        last_emitted: Mutex::new(None),
    });
    let result = tauri::async_runtime::spawn_blocking(move || {
        let _destination_lease = destination_leases.acquire(destination_key);
        let progress_gate = Arc::clone(&progress_gate);
        let progress: CopyProgressCallback = Arc::new(move |update| {
            let total = match update.stage {
                CopyProgressStage::Copying => {
                    copied_by_worker.fetch_add(update.bytes, Ordering::Relaxed) + update.bytes
                }
                CopyProgressStage::Verifying => {
                    verified_by_worker.fetch_add(update.bytes, Ordering::Relaxed) + update.bytes
                }
            };
            send_throttled_copy_progress(
                &progress_gate,
                &copy_channel,
                &progress_operation_id,
                total,
                Some(planned_bytes),
                update,
            );
        });
        execute_recovery_ingest(prepared, cancellation, Some(progress))
    })
    .await;
    match result {
        Ok(Ok(mut completed)) => {
            send_progress(
                &channel,
                &operation_id,
                IngestState::Verifying,
                completed.summary.copied_bytes,
                Some(planned_bytes),
            );
            {
                let Ok(mut store) = state.store.lock() else {
                    return Ok(persistence_failure());
                };
                if !persist_verified_completion(&mut store, &operation_id, &completed) {
                    let _ = store.return_to_recovery_required(
                        &operation_id,
                        "recovered completion persistence failed",
                    );
                    return Ok(persistence_failure());
                }
            }
            let managed_witness =
                managed_card_witness(&completed.summary, &completed.manifest_root_blake3, &state);
            completed.summary.source_marker_status =
                source_marker_status_with_content_witness(&completed.source_root, &managed_witness);
            send_progress(
                &channel,
                &operation_id,
                IngestState::Formatting,
                completed.summary.copied_bytes,
                Some(planned_bytes),
            );
            completed.summary.auto_format_status = run_auto_format_after_verified_ingest(
                &completed.summary,
                &completed.source_root,
                Some(&managed_witness),
                completed.auto_ingest_triggered,
                &state,
            )
            .await;
            record_auto_format_outcome(
                &state,
                &operation_id,
                &completed.summary.auto_format_status,
            );
            send_progress(
                &channel,
                &operation_id,
                IngestState::Completed,
                completed.summary.copied_bytes,
                Some(planned_bytes),
            );
            Ok(IpcResponse::success(completed.summary))
        }
        Ok(Err(error)) => {
            if let Ok(mut store) = state.store.lock() {
                let _ = store.return_to_recovery_required(
                    &operation_id,
                    "recovery verification or copy failed",
                );
            }
            send_progress(
                &channel,
                &operation_id,
                progress_state_for_error(&error),
                copied_transferred.load(Ordering::Relaxed),
                Some(planned_bytes),
            );
            Ok(error_response(error))
        }
        Err(_) => {
            if let Ok(mut store) = state.store.lock() {
                let _ = store.return_to_recovery_required(
                    &operation_id,
                    "recovery worker ended unexpectedly",
                );
            }
            Ok(persistence_failure())
        }
    }
}

/// Executes a deliberately bounded, verified ingest.  It is async at the IPC
/// edge so the desktop runtime remains responsive while the blocking copy
/// workers run on Tauri's blocking pool.
#[tauri::command]
async fn start_verified_ingest(
    mut request: VerifiedIngestRequest,
    channel: Channel<ProgressUpdate>,
    state: State<'_, AppState>,
) -> Result<IpcResponse<VerifiedIngestResult>, IpcError> {
    let current_snapshot = match blocking_native_device_snapshot(
        Arc::clone(&state.store),
        Arc::clone(&state.connection_generations),
    )
    .await
    {
        Ok(snapshot) => snapshot,
        Err(()) => return Ok(persistence_failure()),
    };
    let Some(observed_source) = current_source_for_ingest_request(&current_snapshot, &request)
    else {
        return Ok(IpcResponse::failure(
            IpcErrorCode::DeviceUnavailable,
            "The selected card changed, was removed, or remounted before ingest could start",
        ));
    };
    // Identity confidence comes from the fresh native observation, never from
    // a webview-supplied request field.
    request.source_identity_confidence = observed_source.identity.confidence.clone();
    let operation_id = request
        .operation_id
        .as_deref()
        .and_then(|value| uuid::Uuid::parse_str(value).ok())
        .map(|value| value.to_string())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    info!(
        target: "media_ingest_tool::support",
        "ingest_start_requested operation_id={} worker_limit={} automatic={}",
        operation_id,
        request.max_workers,
        request.auto_ingest_triggered
    );
    let cancellation = Arc::new(std::sync::atomic::AtomicBool::new(false));
    {
        let Ok(mut active_ingests) = state.active_ingests.lock() else {
            return Ok(persistence_failure());
        };
        if active_ingests.contains_key(&operation_id) {
            return Ok(IpcResponse::failure(
                IpcErrorCode::InvalidRequest,
                "An ingest already uses this operation ID",
            ));
        }
        active_ingests.insert(
            operation_id.clone(),
            ActiveIngest {
                cancellation: Arc::clone(&cancellation),
                source_medium_key: request.source_medium_key.clone(),
                source_root: request.source_root.clone(),
                source_seen_in_snapshot: false,
            },
        );
    }
    let _cancellation_registration = CancellationRegistration {
        active_ingests: Arc::clone(&state.active_ingests),
        operation_id: operation_id.clone(),
    };
    send_progress(&channel, &operation_id, IngestState::Queued, 0, None);
    let source_identity = source_identity_for_request(&request);
    let prepare_operation_id = operation_id.clone();
    let prepared = match tauri::async_runtime::spawn_blocking(move || {
        prepare_verified_ingest(request, prepare_operation_id)
    })
    .await
    {
        Ok(Ok(prepared)) => prepared,
        Ok(Err(error)) => {
            send_progress(
                &channel,
                &operation_id,
                progress_state_for_error(&error),
                0,
                None,
            );
            return Ok(error_response(error));
        }
        Err(_) => {
            send_progress(&channel, &operation_id, IngestState::Failed, 0, None);
            return Ok(IpcResponse::failure(
                IpcErrorCode::OperationCancelled,
                "The ingest planner ended before preparing a transfer",
            ));
        }
    };
    let planned_bytes = prepared
        .files
        .iter()
        .map(|file| file.source.byte_length)
        .sum::<u64>();
    // A post-format remount can reach this command before the profile lookup
    // observes the recorded format receipt. Never persist an empty automatic
    // run: it has no media receipt to seal and must not turn a harmless race
    // into a failed ingest history row.
    if skips_empty_auto_ingest(&prepared.request, prepared.files.len()) {
        let summary = VerifiedIngestResult {
            operation_id: operation_id.clone(),
            source_medium_key: prepared.request.source_medium_key.clone(),
            source_generation: prepared.request.source_generation,
            copied_files: 0,
            copied_bytes: 0,
            receipt_name: String::new(),
            source_marker_status: source_marker_status(&prepared.source_root),
            auto_format_status: AutoFormatStatus::Skipped,
        };
        send_progress(&channel, &operation_id, IngestState::Completed, 0, Some(0));
        return Ok(IpcResponse::success(summary));
    }
    let preflight_source_root = prepared.source_root.clone();
    let preflight_destination_root = prepared.destination_root.clone();
    let destination_key = match tauri::async_runtime::spawn_blocking(move || {
        validate_ingest_roots(&preflight_source_root, &preflight_destination_root)
            .and_then(|()| has_destination_space(&preflight_destination_root, planned_bytes))
            .and_then(|has_space| {
                has_space
                    .then_some(())
                    .ok_or(IngestError::InsufficientDestinationSpace)
            })
            .and_then(|()| destination_lease_key(&preflight_destination_root))
    })
    .await
    {
        Ok(Ok(key)) => key,
        Ok(Err(error)) => {
            send_progress(
                &channel,
                &operation_id,
                progress_state_for_error(&error),
                0,
                Some(planned_bytes),
            );
            return Ok(error_response(error));
        }
        Err(_) => {
            send_progress(
                &channel,
                &operation_id,
                IngestState::Failed,
                0,
                Some(planned_bytes),
            );
            return Ok(IpcResponse::failure(
                IpcErrorCode::OperationCancelled,
                "The ingest preflight worker ended before copying could start",
            ));
        }
    };
    send_progress(
        &channel,
        &operation_id,
        IngestState::Copying,
        0,
        Some(planned_bytes),
    );
    {
        let Ok(mut store) = state.store.lock() else {
            return Ok(persistence_failure());
        };
        if store
            .begin_ingest_run_with_mode(
                &operation_id,
                &source_identity,
                prepared.request.source_generation,
                &prepared.source_root.to_string_lossy(),
                &prepared.destination_root.to_string_lossy(),
                prepared.request.auto_ingest_triggered,
            )
            .is_err()
        {
            send_progress(
                &channel,
                &operation_id,
                IngestState::Failed,
                0,
                Some(planned_bytes),
            );
            return Ok(persistence_failure());
        }
        let planned = prepared.files.iter().all(|file| {
            store
                .record_planned_file(
                    &operation_id,
                    &PlannedFileRecord {
                        entry_id: file.entry_id.clone(),
                        source_relative_path: file
                            .source
                            .relative_path
                            .to_string_lossy()
                            .replace('\\', "/"),
                        destination_relative_path: file
                            .destination_relative_path
                            .to_string_lossy()
                            .replace('\\', "/"),
                        byte_length: file.source.byte_length,
                    },
                )
                .unwrap_or(false)
        });
        if !planned
            || store
                .planned_file_count(&operation_id)
                .map(|count| count != prepared.files.len() as u64)
                .unwrap_or(true)
            || !store
                .transition_ingest_run(&operation_id, IngestRunState::Copying, "plan persisted")
                .unwrap_or(false)
        {
            let _ = store.transition_ingest_run(
                &operation_id,
                IngestRunState::Failed,
                "planned file manifest persistence failed",
            );
            send_progress(
                &channel,
                &operation_id,
                IngestState::Failed,
                0,
                Some(planned_bytes),
            );
            return Ok(persistence_failure());
        }
    }
    let copy_channel = channel.clone();
    let progress_operation_id = operation_id.clone();
    let copied_transferred = Arc::new(AtomicU64::new(0));
    let copied_by_worker = Arc::clone(&copied_transferred);
    let verified_transferred = Arc::new(AtomicU64::new(0));
    let verified_by_worker = Arc::clone(&verified_transferred);
    let destination_leases = Arc::clone(&state.destination_leases);
    let progress_gate = Arc::new(ProgressEmissionGate {
        last_emitted: Mutex::new(None),
    });
    let result = tauri::async_runtime::spawn_blocking(move || {
        let _destination_lease = destination_leases.acquire(destination_key);
        let progress_gate = Arc::clone(&progress_gate);
        let progress: CopyProgressCallback = Arc::new(move |update| {
            let total = match update.stage {
                CopyProgressStage::Copying => {
                    copied_by_worker.fetch_add(update.bytes, Ordering::Relaxed) + update.bytes
                }
                CopyProgressStage::Verifying => {
                    verified_by_worker.fetch_add(update.bytes, Ordering::Relaxed) + update.bytes
                }
            };
            send_throttled_copy_progress(
                &progress_gate,
                &copy_channel,
                &progress_operation_id,
                total,
                Some(planned_bytes),
                update,
            );
        });
        execute_prepared_ingest(prepared, cancellation, Some(progress))
    })
    .await;
    let response = match result {
        Ok(Ok(mut completed)) => {
            send_progress(
                &channel,
                &operation_id,
                IngestState::Verifying,
                completed.summary.copied_bytes,
                Some(planned_bytes),
            );
            {
                let Ok(mut store) = state.store.lock() else {
                    send_progress(
                        &channel,
                        &operation_id,
                        IngestState::Failed,
                        completed.summary.copied_bytes,
                        Some(planned_bytes),
                    );
                    return Ok(persistence_failure());
                };
                if !persist_verified_completion(&mut store, &operation_id, &completed) {
                    let _ = store.transition_ingest_run(
                        &operation_id,
                        IngestRunState::Failed,
                        "verified completion persistence failed",
                    );
                    send_progress(
                        &channel,
                        &operation_id,
                        IngestState::Failed,
                        completed.summary.copied_bytes,
                        Some(planned_bytes),
                    );
                    return Ok(persistence_failure());
                }
            }
            let managed_witness =
                managed_card_witness(&completed.summary, &completed.manifest_root_blake3, &state);
            completed.summary.source_marker_status =
                source_marker_status_with_content_witness(&completed.source_root, &managed_witness);
            send_progress(
                &channel,
                &operation_id,
                IngestState::Formatting,
                completed.summary.copied_bytes,
                Some(planned_bytes),
            );
            completed.summary.auto_format_status = run_auto_format_after_verified_ingest(
                &completed.summary,
                &completed.source_root,
                Some(&managed_witness),
                completed.auto_ingest_triggered,
                &state,
            )
            .await;
            record_auto_format_outcome(
                &state,
                &operation_id,
                &completed.summary.auto_format_status,
            );
            send_progress(
                &channel,
                &operation_id,
                IngestState::Completed,
                completed.summary.copied_bytes,
                Some(planned_bytes),
            );
            info!(
                target: "media_ingest_tool::support",
                "ingest_completed operation_id={} copied_files={} copied_bytes={} auto_format_status={:?}",
                operation_id,
                completed.summary.copied_files,
                completed.summary.copied_bytes,
                completed.summary.auto_format_status
            );
            Ok(IpcResponse::success(completed.summary))
        }
        Ok(Err(error)) => {
            warn!(
                target: "media_ingest_tool::support",
                "ingest_recovery_required operation_id={} outcome={:?}",
                operation_id,
                error
            );
            if let Ok(mut store) = state.store.lock() {
                let _ = store.return_to_recovery_required(
                    &operation_id,
                    "copy, verification, or receipt failure; explicit recovery required",
                );
            }
            send_progress(
                &channel,
                &operation_id,
                progress_state_for_error(&error),
                copied_transferred.load(Ordering::Relaxed),
                Some(planned_bytes),
            );
            Ok(error_response(error))
        }
        Err(_) => {
            warn!(
                target: "media_ingest_tool::support",
                "ingest_worker_ended operation_id={}",
                operation_id
            );
            if let Ok(mut store) = state.store.lock() {
                let _ = store.return_to_recovery_required(
                    &operation_id,
                    "ingest worker ended unexpectedly; explicit recovery required",
                );
            }
            send_progress(
                &channel,
                &operation_id,
                IngestState::Failed,
                copied_transferred.load(Ordering::Relaxed),
                Some(planned_bytes),
            );
            Ok(persistence_failure())
        }
    };
    response
}

/// Persists the only sequence that can make an ingest complete: exact planned
/// entries become verified, an immutable receipt is sealed, then the run state
/// advances. The caller must create the optional source marker only after this
/// returns true and the database lock is released.
fn persist_verified_completion(
    store: &mut LocalStore,
    operation_id: &str,
    completed: &CompletedIngest,
) -> bool {
    let files = completed
        .copies
        .iter()
        .map(|copy| VerifiedFileRecord {
            entry_id: copy.entry_id.clone(),
            source_relative_path: copy
                .source_relative_path
                .to_string_lossy()
                .replace('\\', "/"),
            destination_relative_path: copy
                .destination_relative_path
                .to_string_lossy()
                .replace('\\', "/"),
            byte_length: copy.bytes,
            source_blake3: copy.digest_hex.clone(),
            destination_blake3: copy.digest_hex.clone(),
        })
        .collect::<Vec<_>>();
    store
        .commit_verified_completion(
            operation_id,
            &files,
            &completed.manifest_algorithm,
            &completed.manifest_root_blake3,
            completed.summary.copied_files as u64,
            completed.summary.copied_bytes,
        )
        .unwrap_or(false)
}

fn send_progress(
    channel: &Channel<ProgressUpdate>,
    operation_id: &str,
    state: IngestState,
    transferred_bytes: u64,
    total_bytes: Option<u64>,
) {
    let _ = channel.send(ProgressUpdate {
        sequence: PROGRESS_SEQUENCE.fetch_add(1, Ordering::Relaxed) + 1,
        operation_id: operation_id.into(),
        state,
        transferred_bytes,
        total_bytes,
        current_file_index: None,
        total_files: None,
    });
}

fn send_copy_progress(
    channel: &Channel<ProgressUpdate>,
    operation_id: &str,
    transferred_bytes: u64,
    total_bytes: Option<u64>,
    update: CopyProgress,
) {
    let _ = channel.send(ProgressUpdate {
        sequence: PROGRESS_SEQUENCE.fetch_add(1, Ordering::Relaxed) + 1,
        operation_id: operation_id.into(),
        state: match update.stage {
            CopyProgressStage::Copying => IngestState::Copying,
            CopyProgressStage::Verifying => IngestState::Verifying,
        },
        transferred_bytes,
        total_bytes,
        current_file_index: Some(update.file_index),
        total_files: Some(update.total_files),
    });
}

/// Copy/hash workers can produce an event for every buffer. Coalesce only the
/// presentation layer; the atomic byte totals and terminal lifecycle events
/// remain exact and are never used as transfer authority.
fn send_throttled_copy_progress(
    gate: &ProgressEmissionGate,
    channel: &Channel<ProgressUpdate>,
    operation_id: &str,
    transferred_bytes: u64,
    total_bytes: Option<u64>,
    update: CopyProgress,
) {
    if gate.should_emit(Instant::now()) {
        send_copy_progress(
            channel,
            operation_id,
            transferred_bytes,
            total_bytes,
            update,
        );
    }
}

#[tauri::command]
fn cancel_verified_ingest(operation_id: String, state: State<'_, AppState>) -> IpcResponse<()> {
    let Ok(active_ingests) = state.active_ingests.lock() else {
        return IpcResponse::failure(
            IpcErrorCode::DeviceUnavailable,
            "The local ingest scheduler is unavailable",
        );
    };
    let Some(active) = active_ingests.get(&operation_id) else {
        return IpcResponse::failure(
            IpcErrorCode::InvalidRequest,
            "No active ingest matches this operation ID",
        );
    };
    active.cancellation.store(true, Ordering::Release);
    info!(
        target: "media_ingest_tool::support",
        "ingest_cancellation_requested operation_id={}",
        operation_id
    );
    IpcResponse::success(())
}

fn prepare_verified_ingest(
    request: VerifiedIngestRequest,
    operation_id: String,
) -> Result<PreparedIngest, IngestError> {
    if request.source_medium_key.trim().is_empty() {
        return Err(IngestError::InvalidPath);
    }
    if matches!(request.sort_mode, IngestSortMode::CameraInterval)
        && !matches!(request.interval_minutes, Some(1..=1_440))
    {
        return Err(IngestError::InvalidPath);
    }
    if custom_directory_prefix(&request.custom_directory_fields).is_err()
        || crate::organization::canonical_destination_depth_order(
            &sort_mode_for_request(&request.sort_mode, request.interval_minutes),
            request.custom_directory_fields.len(),
            request.destination_depth_order.as_deref(),
        )
        .is_err()
    {
        return Err(IngestError::InvalidPath);
    }
    let source_root = PathBuf::from(&request.source_root);
    let destination_root = PathBuf::from(&request.destination_root);
    let files = plan_copy_files(
        enumerate_regular_files(&source_root)?,
        &source_root,
        &request,
        &operation_id,
    )?;
    Ok(PreparedIngest {
        request,
        operation_id,
        source_root,
        destination_root,
        files,
    })
}

fn prepare_recovery_ingest(
    recovery: &RecoverableIngestRun,
    planned: Vec<PlannedFileRecord>,
    max_workers: usize,
    observed: &StorageDevice,
) -> Result<PreparedIngest, IngestError> {
    let source_root = PathBuf::from(&recovery.source_root);
    let destination_root = PathBuf::from(&recovery.destination_root);
    validate_ingest_roots(&source_root, &destination_root)?;
    let files = planned
        .into_iter()
        .map(|file| {
            let source = PathBuf::from(&file.source_relative_path);
            let destination = PathBuf::from(&file.destination_relative_path);
            if source.as_os_str().is_empty()
                || destination.as_os_str().is_empty()
                || source.is_absolute()
                || destination.is_absolute()
            {
                return Err(IngestError::InvalidPath);
            }
            Ok(PlannedCopyFile {
                entry_id: file.entry_id,
                source: crate::ingest::PlannedSourceFile {
                    relative_path: source,
                    byte_length: file.byte_length,
                },
                destination_relative_path: destination,
            })
        })
        .collect::<Result<Vec<_>, IngestError>>()?;
    Ok(PreparedIngest {
        request: VerifiedIngestRequest {
            operation_id: Some(recovery.run_id.clone()),
            source_root: recovery.source_root.clone(),
            destination_root: recovery.destination_root.clone(),
            source_medium_key: recovery.source_identity_key.clone(),
            source_identity_confidence: observed.identity.confidence.clone(),
            source_generation: observed.connection_generation,
            max_workers,
            sort_mode: IngestSortMode::OriginalTree,
            interval_minutes: None,
            custom_directory_fields: Vec::new(),
            destination_depth_order: None,
            auto_ingest_triggered: recovery.auto_ingest_triggered,
        },
        operation_id: recovery.run_id.clone(),
        source_root,
        destination_root,
        files,
    })
}

fn execute_prepared_ingest(
    prepared: PreparedIngest,
    cancellation: Arc<std::sync::atomic::AtomicBool>,
    progress: Option<CopyProgressCallback>,
) -> Result<CompletedIngest, IngestError> {
    let outcomes = verified_copy_batch_planned_with_progress(
        &prepared.source_root,
        prepared.files.clone(),
        &prepared.destination_root,
        WorkerLimits {
            // More workers are not automatically faster for a card reader;
            // retain a safe bounded ceiling regardless of untrusted UI input.
            max_workers: prepared.request.max_workers.clamp(1, 16),
        },
        cancellation,
        progress,
    );
    let copies = outcomes
        .into_iter()
        .collect::<Result<Vec<_>, IngestError>>()?;
    finish_prepared_ingest(prepared, copies)
}

/// Reconciles final files that survived a process crash before the SQLite
/// completion transaction. Existing paths must independently rehash; only
/// absent paths re-enter the normal bounded copy primitive. A conflicting or
/// corrupted existing path is never overwritten.
fn execute_recovery_ingest(
    prepared: PreparedIngest,
    cancellation: Arc<std::sync::atomic::AtomicBool>,
    progress: Option<CopyProgressCallback>,
) -> Result<CompletedIngest, IngestError> {
    let mut existing = Vec::new();
    let mut missing = Vec::new();
    let total_files = prepared.files.len();
    for (index, file) in prepared.files.iter().enumerate() {
        let final_path = prepared
            .destination_root
            .join(&file.destination_relative_path);
        if final_path.exists() {
            existing.push(verify_existing_copy_with_progress(
                &prepared.source_root,
                &file.source,
                &file.destination_relative_path,
                &file.entry_id,
                &prepared.destination_root,
                &cancellation,
                progress.clone().map(|callback| VerificationProgress {
                    callback,
                    file_index: index + 1,
                    total_files,
                }),
            )?);
        } else {
            missing.push((index + 1, file.clone()));
        }
    }
    let copied = verified_copy_batch_planned_with_progress_positions(
        &prepared.source_root,
        missing,
        total_files,
        &prepared.destination_root,
        WorkerLimits {
            max_workers: prepared.request.max_workers.clamp(1, 16),
        },
        cancellation,
        progress,
    )
    .into_iter()
    .collect::<Result<Vec<_>, IngestError>>()?;
    existing.extend(copied);
    finish_prepared_ingest(prepared, existing)
}

fn finish_prepared_ingest(
    prepared: PreparedIngest,
    copies: Vec<crate::ingest::VerifiedCopy>,
) -> Result<CompletedIngest, IngestError> {
    let receipt_name = format!("{}.json", prepared.operation_id);
    let receipt_path = prepared
        .destination_root
        .join(".media-ingest-receipts")
        .join(&receipt_name);
    let copied_bytes = copies.iter().map(|copy| copy.bytes).sum();
    let manifest_root_blake3 = manifest_root(&copies);
    let receipt = IngestReceipt {
        schema_version: 1,
        manifest_algorithm: MANIFEST_ALGORITHM.into(),
        manifest_root_blake3: manifest_root_blake3.clone(),
        source_medium_key: prepared.request.source_medium_key.clone(),
        source_identity_confidence: identity_confidence_name(
            prepared.request.source_identity_confidence,
        )
        .into(),
        source_generation: prepared.request.source_generation,
        files: copies
            .iter()
            .map(|copy| ReceiptFile {
                relative_path: copy
                    .final_path
                    .strip_prefix(&prepared.destination_root)
                    .unwrap_or(&copy.final_path)
                    .to_string_lossy()
                    .replace('\\', "/"),
                bytes: copy.bytes,
                blake3: copy.digest_hex.clone(),
            })
            .collect(),
    };
    ensure_recovery_receipt(&receipt_path, &receipt)?;
    Ok(CompletedIngest {
        summary: VerifiedIngestResult {
            operation_id: prepared.operation_id,
            source_medium_key: prepared.request.source_medium_key,
            source_generation: prepared.request.source_generation,
            copied_files: copies.len(),
            copied_bytes,
            receipt_name,
            // The card itself remains untouched until the command has sealed
            // the SQLite receipt and recorded the completed lifecycle state.
            source_marker_status: SourceMarkerStatus::Unavailable,
            auto_format_status: AutoFormatStatus::NotConfigured,
        },
        copies,
        manifest_algorithm: MANIFEST_ALGORITHM.into(),
        manifest_root_blake3,
        source_root: prepared.source_root,
        auto_ingest_triggered: prepared.request.auto_ingest_triggered,
    })
}

/// A receipt published before a crash may legitimately outlive the database
/// transaction. It is accepted only after all final files were independently
/// rehashed and only when its complete immutable projection matches exactly;
/// malformed or conflicting receipts are never replaced.
fn ensure_recovery_receipt(
    path: &std::path::Path,
    receipt: &IngestReceipt,
) -> Result<(), IngestError> {
    if !path.exists() {
        return write_receipt(path, receipt);
    }
    let existing = std::fs::read(path)?;
    let existing = serde_json::from_slice::<IngestReceipt>(&existing)
        .map_err(|_| IngestError::VerificationFailed)?;
    if &existing == receipt {
        Ok(())
    } else {
        Err(IngestError::VerificationFailed)
    }
}

fn source_marker_status(source_root: &std::path::Path) -> SourceMarkerStatus {
    match crate::storage_marker::ensure_marker(source_root) {
        Ok(crate::storage_marker::MarkerState::Existing) => SourceMarkerStatus::Recognized,
        Ok(crate::storage_marker::MarkerState::Created) => SourceMarkerStatus::Created,
        // A read-only card or an occupied reserved filename must not change
        // the already-verified transfer result. The UI receives only a status,
        // never a raw source path or OS error.
        Err(_) => SourceMarkerStatus::Unavailable,
    }
}

fn skips_empty_auto_ingest(request: &VerifiedIngestRequest, file_count: usize) -> bool {
    request.auto_ingest_triggered && file_count == 0
}

/// A completed ingest leaves a compact, path-free witness of the exact
/// verified media manifest alongside its random managed-card token. This is
/// useful continuity evidence only: it remains copyable filesystem state.
fn source_marker_status_with_content_witness(
    source_root: &std::path::Path,
    manifest_root_blake3: &str,
) -> SourceMarkerStatus {
    match crate::storage_marker::ensure_marker_with_fingerprint(source_root, manifest_root_blake3) {
        Ok(crate::storage_marker::MarkerState::Existing) => SourceMarkerStatus::Recognized,
        Ok(crate::storage_marker::MarkerState::Created) => SourceMarkerStatus::Created,
        Err(_) => SourceMarkerStatus::Unavailable,
    }
}

/// Hashes bounded current host observations with the sealed media manifest.
/// The resulting value detects ordinary card/content swaps better than the
/// token alone while retaining no mount path, label, raw serial, or filename.
fn managed_card_witness(
    result: &VerifiedIngestResult,
    manifest_root_blake3: &str,
    state: &AppState,
) -> String {
    let snapshot = native_device_snapshot(&state.store, &state.connection_generations);
    let device = snapshot.devices.iter().find(|device| {
        device.state == DeviceState::Available
            && device.identity.media_key == result.source_medium_key
            && device.connection_generation == result.source_generation
    });
    device.map_or_else(
        || managed_card_witness_for_device(manifest_root_blake3, None),
        |device| managed_card_witness_for_device(manifest_root_blake3, Some(device)),
    )
}

fn managed_card_witness_for_device(
    manifest_root_blake3: &str,
    device: Option<&StorageDevice>,
) -> String {
    let (capacity, filesystem, reader, slot) = device.map_or_else(
        || (String::new(), String::new(), String::new(), String::new()),
        |device| {
            (
                device
                    .details
                    .total_bytes
                    .map_or_else(String::new, |value| value.to_string()),
                device.details.filesystem.clone().unwrap_or_default(),
                device
                    .details
                    .reader_fingerprint
                    .clone()
                    .unwrap_or_default(),
                device.details.reader_slot.clone().unwrap_or_default(),
            )
        },
    );
    blake3::hash(
        format!(
            "mit2-managed-witness\0{manifest_root_blake3}\0{capacity}\0{filesystem}\0{reader}\0{slot}"
        )
        .as_bytes(),
    )
    .to_hex()
    .to_string()
}

fn managed_card_matches_sealed_receipt(
    store: &LocalStore,
    run_id: &str,
    device: &StorageDevice,
) -> bool {
    let Some(root) = device.details.mount_locations.first().map(PathBuf::from) else {
        return false;
    };
    let Ok(Some(marker)) = crate::storage_marker::read_record(&root) else {
        return false;
    };
    let Ok(Some(profile)) = store.marker_ingest_profile(&marker.token) else {
        return false;
    };
    if profile.marker_token != marker.token {
        return false;
    }
    let Ok(Some(manifest_root)) = store.receipt_manifest_root(run_id) else {
        return false;
    };
    let expected = managed_card_witness_for_device(&manifest_root, Some(device));
    marker.content_fingerprint.as_deref() == Some(expected.as_str())
        // Older MIT2 records stored the sealed root directly. Keep this
        // bounded migration path; the next verified ingest upgrades it.
        || marker.content_fingerprint.as_deref() == Some(manifest_root.as_str())
}

/// Before format, a managed card is bound by its registered marker witness.
/// Format removes that marker, so post-format validation retains the provider's
/// exact volume/capacity check and requires the same calibrated reader slot.
/// Hardware-immutable identity remains the stronger path whenever available.
fn same_managed_format_target(
    before: &StorageDevice,
    after: &StorageDevice,
    validated_root: &std::path::Path,
    expected_capacity: u64,
) -> bool {
    after.state == DeviceState::Available
        && after.details.total_bytes == Some(expected_capacity)
        && after.details.mount_locations.len() == 1
        && Path::new(&after.details.mount_locations[0]) == validated_root
        && ((before.identity.confidence == IdentityConfidence::HardwareImmutable
            && after.identity.confidence == IdentityConfidence::HardwareImmutable
            && after.identity.media_key == before.identity.media_key)
            || (before.details.reader_fingerprint.is_some()
                && before.details.reader_fingerprint == after.details.reader_fingerprint
                && before.details.reader_slot.is_some()
                && before.details.reader_slot == after.details.reader_slot))
}

/// The recovery route intentionally bypasses receipt, marker, and media
/// continuity evidence. The provider has already revalidated the native
/// opaque target immediately before formatting; after remount we retain only
/// the target's removable-volume availability, exact validated mount, and
/// capacity checks. It never permits an arbitrary path from the webview.
fn same_force_format_target(
    after: &StorageDevice,
    validated_root: &std::path::Path,
    expected_capacity: u64,
) -> bool {
    after.state == DeviceState::Available
        && after.details.total_bytes == Some(expected_capacity)
        && after.details.mount_locations.len() == 1
        && Path::new(&after.details.mount_locations[0]) == validated_root
}

/// Runs only after the receipt transaction has sealed and only for a card the
/// operator explicitly registered with auto-format enabled. Every native
/// target decision is freshly observed here; the webview supplies no target.
async fn run_auto_format_after_verified_ingest(
    result: &VerifiedIngestResult,
    source_root: &std::path::Path,
    managed_witness: Option<&str>,
    auto_ingest_triggered: bool,
    state: &AppState,
) -> AutoFormatStatus {
    let result = result.clone();
    let source_root = source_root.to_path_buf();
    let managed_witness = managed_witness.map(str::to_owned);
    let store = Arc::clone(&state.store);
    let connection_generations = Arc::clone(&state.connection_generations);
    let active_ingests = Arc::clone(&state.active_ingests);
    match tauri::async_runtime::spawn_blocking(move || {
        maybe_auto_format_after_verified_ingest(
            &result,
            &source_root,
            managed_witness.as_deref(),
            auto_ingest_triggered,
            store.as_ref(),
            connection_generations.as_ref(),
            active_ingests.as_ref(),
        )
    })
    .await
    {
        Ok(status) => status,
        Err(_) => AutoFormatStatus::Failed,
    }
}

fn maybe_auto_format_after_verified_ingest(
    result: &VerifiedIngestResult,
    source_root: &std::path::Path,
    managed_witness: Option<&str>,
    auto_ingest_triggered: bool,
    store: &Mutex<LocalStore>,
    connection_generations: &Mutex<ConnectionGenerationTracker>,
    active_ingests: &Mutex<HashMap<String, ActiveIngest>>,
) -> AutoFormatStatus {
    macro_rules! skipped {
        ($reason:literal) => {{
            if let Ok(mut store) = store.lock() {
                let _ = store.record_ingest_note(&result.operation_id, $reason);
            }
            return AutoFormatStatus::Skipped;
        }};
    }
    if !auto_ingest_triggered {
        return AutoFormatStatus::NotConfigured;
    }
    // Formatting an already-empty registered card achieves nothing and would
    // otherwise repeat on every mount after the first successful format.
    if result.copied_files == 0 {
        skipped!("managed auto-format skipped: verified source contained no files");
    }
    let conflicting_ingest = active_ingests
        .lock()
        .map(|active| {
            active.iter().any(|(operation_id, ingest)| {
                operation_id != &result.operation_id
                    && ingest.source_medium_key == result.source_medium_key
            })
        })
        .unwrap_or(true);
    if conflicting_ingest {
        skipped!("managed auto-format skipped: another source ingest is active");
    }
    let receipt_is_current = store.lock().ok().and_then(|store| {
        store
            .has_completed_receipt_for_source(
                &result.operation_id,
                &result.source_medium_key,
                result.source_generation,
            )
            .ok()
    }) == Some(true);
    if !receipt_is_current {
        skipped!("managed auto-format skipped: sealed receipt no longer matches this mount");
    }
    let marker_record = match crate::storage_marker::read_record(source_root) {
        Ok(Some(record)) => record,
        _ => return AutoFormatStatus::NotConfigured,
    };
    let marker = marker_record.token;
    let profile = match store
        .lock()
        .ok()
        .and_then(|store| store.marker_ingest_profile(&marker).ok().flatten())
    {
        Some(profile) if profile.auto_format_enabled => profile,
        _ => return AutoFormatStatus::NotConfigured,
    };
    let snapshot = native_device_snapshot(store, connection_generations);
    // The receipt contains no raw source identity because that information is
    // deliberately native-only. Re-resolve by the mounted root and require a
    // single immutable device that still contains the same registered marker.
    let managed_witness_matches = managed_witness
        .zip(marker_record.content_fingerprint.as_deref())
        .is_some_and(|(expected, observed)| expected == observed);
    let matching = snapshot
        .devices
        .iter()
        .filter(|device| {
            device.state == DeviceState::Available
                && device.identity.media_key == result.source_medium_key
                && device.connection_generation == result.source_generation
                && (device.identity.confidence == IdentityConfidence::HardwareImmutable
                    || managed_witness_matches)
                && device.details.mount_locations.len() == 1
                && device.details.mount_locations[0] == source_root.to_string_lossy()
                && crate::storage_marker::read_marker(std::path::Path::new(
                    &device.details.mount_locations[0],
                ))
                .ok()
                .flatten()
                .as_deref()
                    == Some(marker.as_str())
        })
        .collect::<Vec<_>>();
    let Some(device) = (matching.len() == 1).then(|| matching[0]) else {
        skipped!("managed auto-format skipped: current mount, generation, marker, or witness did not resolve uniquely");
    };
    let Some(format_profile) = recommended_profile(device.details.total_bytes) else {
        skipped!(
            "managed auto-format skipped: no allowlisted profile matches the current capacity"
        );
    };
    let Some(capacity) = device.details.total_bytes else {
        skipped!("managed auto-format skipped: current capacity is unavailable");
    };
    let expected = crate::format_provider::ExpectedFormatTarget {
        medium_key: device.identity.media_key.clone(),
        connection_generation: device.connection_generation,
        expected_capacity_bytes: capacity,
        current_mount_root: source_root.to_path_buf(),
    };
    let provider = crate::format_provider::current_platform_provider();
    let target = match provider.resolve_exact_target(&expected) {
        Ok(target) => target,
        // A registered preference must not turn an unimplemented native
        // formatter into a failed ingest. The card remains intact and the UI
        // distinguishes this from a failed destructive operation.
        Err(crate::format_provider::FormatProviderError::UnsupportedPlatform) => {
            skipped!("managed auto-format skipped: no native format provider is installed");
        }
        Err(_) => return AutoFormatStatus::Failed,
    };
    if let Err(error) = provider.quick_format(&target, &format_profile) {
        let reason = match error {
            crate::format_provider::FormatProviderError::Busy => {
                "managed auto-format failed: Windows reported the card is busy".into()
            }
            crate::format_provider::FormatProviderError::WriteProtected => {
                "managed auto-format failed: Windows reported write protection".into()
            }
            crate::format_provider::FormatProviderError::TargetChanged => {
                "managed auto-format failed: native target changed before formatting".into()
            }
            crate::format_provider::FormatProviderError::FormatFailedWithCode(code) => {
                format!("managed auto-format failed: native quick-format returned code {code}")
            }
            crate::format_provider::FormatProviderError::FormatInputFailed => {
                "managed auto-format failed: Windows rejected the documented format input".into()
            }
            crate::format_provider::FormatProviderError::FormatOutputMissing => {
                "managed auto-format failed: Windows returned no format result".into()
            }
            crate::format_provider::FormatProviderError::FormatResultUnreadable => {
                "managed auto-format failed: Windows returned an unreadable format result".into()
            }
            _ => "managed auto-format failed: native quick-format provider rejected the operation"
                .into(),
        };
        if let Ok(mut store) = store.lock() {
            let _ = store.record_ingest_note(&result.operation_id, &reason);
        }
        return AutoFormatStatus::Failed;
    }
    let validated = match provider.wait_for_validated_mount(&expected, &format_profile) {
        Ok(validated) => validated,
        Err(_) => return AutoFormatStatus::Failed,
    };
    if crate::storage_marker::read_record(&validated.root)
        .ok()
        .flatten()
        .is_some()
    {
        return AutoFormatStatus::Failed;
    }
    if sentinel_round_trip(&validated.root).is_err() {
        return AutoFormatStatus::Failed;
    }
    if crate::storage_marker::restore_marker(&validated.root, &profile.marker_token).is_err() {
        return AutoFormatStatus::Failed;
    }
    let receipt_persisted = store.lock().ok().and_then(|mut store| {
        store
            .record_completed_format(
                &result.operation_id,
                &result.source_medium_key,
                result.source_generation,
                format_profile.id,
            )
            .ok()
    }) == Some(true);
    if receipt_persisted {
        AutoFormatStatus::Completed
    } else {
        AutoFormatStatus::Failed
    }
}

fn record_auto_format_outcome(state: &AppState, run_id: &str, outcome: &AutoFormatStatus) {
    let reason = match outcome {
        AutoFormatStatus::Completed => "managed auto-format completed and receipt recorded",
        AutoFormatStatus::Failed => "managed auto-format reached the native provider but failed",
        AutoFormatStatus::Skipped => "managed auto-format skipped by a native safety gate",
        AutoFormatStatus::NotConfigured => "managed auto-format was not configured for this ingest",
    };
    if let Ok(mut store) = state.store.lock() {
        let _ = store.record_ingest_note(run_id, reason);
    }
}

fn sentinel_round_trip(root: &std::path::Path) -> std::io::Result<()> {
    use std::io::{Read, Write};
    let path = root.join(format!(
        ".media-ingest-format-sentinel-{}",
        uuid::Uuid::new_v4()
    ));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)?;
    file.write_all(b"media-ingest-format-validation")?;
    file.sync_all()?;
    drop(file);
    let mut reopened = std::fs::File::open(&path)?;
    let mut bytes = Vec::new();
    reopened.read_to_end(&mut bytes)?;
    std::fs::remove_file(path)?;
    (bytes == b"media-ingest-format-validation")
        .then_some(())
        .ok_or_else(|| std::io::Error::other("format sentinel mismatch"))
}

fn ingest_sort_mode_name(mode: &IngestSortMode) -> &'static str {
    match mode {
        IngestSortMode::OriginalTree => "original_tree",
        IngestSortMode::CameraDay => "camera_day",
        IngestSortMode::CameraInterval => "camera_interval",
    }
}

fn ingest_sort_mode_from_name(value: &str) -> Option<IngestSortMode> {
    match value {
        "original_tree" => Some(IngestSortMode::OriginalTree),
        "camera_day" => Some(IngestSortMode::CameraDay),
        "camera_interval" => Some(IngestSortMode::CameraInterval),
        _ => None,
    }
}

fn plan_copy_files(
    source_files: Vec<crate::ingest::PlannedSourceFile>,
    source_root: &std::path::Path,
    request: &VerifiedIngestRequest,
    operation_id: &str,
) -> Result<Vec<PlannedCopyFile>, IngestError> {
    let mode = sort_mode_for_request(&request.sort_mode, request.interval_minutes);
    let mut used = HashSet::new();
    source_files
        .into_iter()
        .map(|source| {
            let metadata = inspect(&source_root.join(&source.relative_path));
            let camera = camera_identity(
                &metadata.make,
                &metadata.model,
                metadata.body_serial.as_deref(),
                operation_id,
            );
            let initial = destination_relative_path_with_order_and_offset(
                &source.relative_path.to_string_lossy(),
                &camera,
                metadata.capture_time,
                metadata.capture_offset_known,
                mode.clone(),
                &request.custom_directory_fields,
                request.destination_depth_order.as_deref(),
            )
            .map_err(|_| IngestError::InvalidPath)?;
            let destination_relative_path =
                unique_destination_path(initial, &source.relative_path, &mut used)?;
            Ok(PlannedCopyFile {
                entry_id: uuid::Uuid::new_v4().to_string(),
                source,
                destination_relative_path,
            })
        })
        .collect()
}

fn sort_mode_for_request(sort_mode: &IngestSortMode, interval_minutes: Option<u16>) -> SortMode {
    match sort_mode {
        IngestSortMode::OriginalTree => SortMode::OriginalTree,
        IngestSortMode::CameraDay => SortMode::CameraDay,
        IngestSortMode::CameraInterval => SortMode::CameraInterval {
            minutes: interval_minutes.unwrap_or(60),
        },
    }
}

fn unique_destination_path(
    path: PathBuf,
    source_relative_path: &std::path::Path,
    used: &mut HashSet<String>,
) -> Result<PathBuf, IngestError> {
    let portable_key =
        crate::organization::portable_destination_key(&path).ok_or(IngestError::InvalidPath)?;
    if used.insert(portable_key) {
        return Ok(path);
    }
    let stem = path
        .file_stem()
        .and_then(OsStr::to_str)
        .ok_or(IngestError::InvalidPath)?;
    let extension = path.extension().and_then(OsStr::to_str);
    let fingerprint = blake3::hash(source_relative_path.to_string_lossy().as_bytes())
        .to_hex()
        .to_string();
    let filename = match extension {
        Some(extension) => format!("{stem}__{}.{extension}", &fingerprint[..10]),
        None => format!("{stem}__{}", &fingerprint[..10]),
    };
    let unique = path.with_file_name(crate::organization::sanitize_destination_component(
        &filename,
    ));
    let portable_key =
        crate::organization::portable_destination_key(&unique).ok_or(IngestError::InvalidPath)?;
    if used.insert(portable_key) {
        Ok(unique)
    } else {
        Err(IngestError::DestinationExists)
    }
}

fn source_identity_for_request(request: &VerifiedIngestRequest) -> SourceIdentityRecord {
    source_identity_record(
        &request.source_medium_key,
        request.source_identity_confidence.clone(),
    )
}

fn source_identity_for_current_device(
    snapshot: &DeviceSnapshot,
    source_medium_key: &str,
) -> Option<SourceIdentityRecord> {
    snapshot
        .devices
        .iter()
        .find(|device| {
            device.state == DeviceState::Available && device.identity.media_key == source_medium_key
        })
        .map(|device| {
            source_identity_record(
                &device.identity.media_key,
                device.identity.confidence.clone(),
            )
        })
}

fn source_identity_record(
    identity_key: &str,
    confidence: IdentityConfidence,
) -> SourceIdentityRecord {
    SourceIdentityRecord {
        identity_key: identity_key.into(),
        source: "tauri.device_snapshot".into(),
        normalized_value: identity_key.into(),
        strength: match confidence {
            IdentityConfidence::HardwareImmutable => {
                crate::identity::IdentityStrength::HardwareStrong
            }
            IdentityConfidence::HardwareStable => {
                crate::identity::IdentityStrength::HardwareReported
            }
            IdentityConfidence::SessionOnly => crate::identity::IdentityStrength::Session,
            IdentityConfidence::Unresolved => crate::identity::IdentityStrength::Ambiguous,
        },
    }
}

fn allows_destination_recall(identity: &SourceIdentityRecord) -> bool {
    matches!(
        identity.strength,
        crate::identity::IdentityStrength::HardwareStrong
            | crate::identity::IdentityStrength::HardwareReported
    )
}

fn persistence_failure() -> IpcResponse<VerifiedIngestResult> {
    IpcResponse::failure(
        IpcErrorCode::DeviceUnavailable,
        "The local ingest manifest could not record this operation; no completion was reported",
    )
}

fn identity_confidence_name(confidence: IdentityConfidence) -> &'static str {
    match confidence {
        IdentityConfidence::HardwareImmutable => "hardware_immutable",
        IdentityConfidence::HardwareStable => "hardware_stable",
        IdentityConfidence::SessionOnly => "session_only",
        IdentityConfidence::Unresolved => "unresolved",
    }
}

fn progress_state_for_error(error: &IngestError) -> IngestState {
    if matches!(error, IngestError::Cancelled) {
        IngestState::Cancelled
    } else {
        IngestState::Failed
    }
}

fn error_response(error: IngestError) -> IpcResponse<VerifiedIngestResult> {
    match error {
        IngestError::Cancelled => IpcResponse::failure(
            IpcErrorCode::OperationCancelled,
            "The ingest was cancelled before verification completed",
        ),
        IngestError::VerificationFailed => IpcResponse::failure(
            IpcErrorCode::DeviceUnavailable,
            "Destination verification failed; formatting remains unavailable",
        ),
        IngestError::SourceChanged => IpcResponse::failure(
            IpcErrorCode::DeviceUnavailable,
            "Source media changed during ingest; no completion receipt was written",
        ),
        IngestError::UnsafeSourceEntry => IpcResponse::failure(
            IpcErrorCode::InvalidRequest,
            "Source contains a symlink or reparse point; it was not copied",
        ),
        IngestError::DestinationExists => IpcResponse::failure(
            IpcErrorCode::InvalidRequest,
            "A destination file or receipt already exists; existing media was not overwritten",
        ),
        IngestError::InsufficientDestinationSpace => IpcResponse::failure(
            IpcErrorCode::DeviceUnavailable,
            "The destination does not have enough currently available space for this ingest plan",
        ),
        IngestError::SourceLimitExceeded => IpcResponse::failure(
            IpcErrorCode::InvalidRequest,
            "The source exceeds the safe media inventory limit",
        ),
        IngestError::InvalidPath => IpcResponse::failure(
            IpcErrorCode::InvalidRequest,
            "Source and destination must be distinct, valid local directories",
        ),
        IngestError::Io(_) => IpcResponse::failure(
            IpcErrorCode::DeviceUnavailable,
            "Native I/O could not complete the ingest; no completion receipt was written",
        ),
    }
}

fn snapshot_from_volumes(
    sequence: u64,
    volumes: Vec<device_discovery::DiscoveredVolume>,
) -> DeviceSnapshot {
    let devices = volumes
        .into_iter()
        .map(|volume| {
            let sd_cid_candidate = volume
                .reader_topology
                .as_ref()
                .and_then(|topology| topology.reported_sd_cid.as_ref())
                .and_then(|cid| {
                    crate::identity::IdentityCandidate::new(
                        "windows.sd.cid",
                        IdentityScope::Medium,
                        cid,
                        crate::identity::IdentityStrength::HardwareStrong,
                    )
                });
            let marker_candidate = volume.marker_token.as_ref().and_then(|token| {
                crate::identity::IdentityCandidate::new(
                    "media-ingest.marker",
                    IdentityScope::Filesystem,
                    token,
                    crate::identity::IdentityStrength::Filesystem,
                )
            });
            StorageDevice {
                state: match volume.state {
                    device_discovery::DiscoveryState::Ready => DeviceState::Available,
                    device_discovery::DiscoveryState::PermissionDenied => DeviceState::Busy,
                    device_discovery::DiscoveryState::Unavailable => DeviceState::EmptyReader,
                },
                connection_generation: 0,
                identity: DeviceIdentity {
                    // A managed-card marker selects an opt-in profile, but it
                    // must never replace the native/session operation key.
                    // Creating or upgrading the marker during a verified
                    // ingest would otherwise change the key mid-operation and
                    // make the exact receipt/current-mount gates disagree.
                    media_key: sd_cid_candidate
                        .as_ref()
                        .map(|cid| derive_key(IdentityScope::Medium, cid))
                        .unwrap_or(volume.session_key),
                    confidence: if sd_cid_candidate.is_some() {
                        IdentityConfidence::HardwareImmutable
                    } else {
                        IdentityConfidence::Unresolved
                    },
                    evidence: volume
                        .identity_candidates
                        .iter()
                        .map(|candidate| IdentityEvidence {
                            kind: match candidate.scope {
                                IdentityScope::Filesystem => {
                                    IdentityEvidenceKind::VolumeFilesystemUuid
                                }
                                IdentityScope::Session => IdentityEvidenceKind::MountPath,
                                _ => IdentityEvidenceKind::MountPath,
                            },
                            fingerprint: derive_key(candidate.scope, candidate),
                            immutable: false,
                        })
                        .chain(sd_cid_candidate.iter().map(|candidate| IdentityEvidence {
                            kind: IdentityEvidenceKind::SdCid,
                            fingerprint: derive_key(IdentityScope::Medium, candidate),
                            immutable: true,
                        }))
                        .chain(marker_candidate.iter().map(|candidate| IdentityEvidence {
                            kind: IdentityEvidenceKind::AppMarker,
                            fingerprint: derive_key(IdentityScope::Filesystem, candidate),
                            immutable: false,
                        }))
                        .chain(
                            volume
                                .reader_topology
                                .iter()
                                .flat_map(|topology| topology.reported_vpd_identifiers.iter())
                                .map(|identifier| {
                                    let candidate = crate::identity::IdentityCandidate::new(
                                        "windows.storage.vpd83",
                                        IdentityScope::Topology,
                                        identifier,
                                        crate::identity::IdentityStrength::Topology,
                                    )
                                    .expect("VPD identifier is normalized before fingerprinting");
                                    IdentityEvidence {
                                        kind: IdentityEvidenceKind::StorageVpd,
                                        fingerprint: derive_key(
                                            IdentityScope::Topology,
                                            &candidate,
                                        ),
                                        // VPD data is useful diagnostic evidence, but a USB reader can
                                        // report a logical-unit identifier that does not follow media.
                                        immutable: false,
                                    }
                                }),
                        )
                        .collect(),
                },
                details: StorageDeviceDetails {
                    display_name: volume.display_name,
                    filesystem: volume.filesystem,
                    total_bytes: volume.total_bytes,
                    available_bytes: volume.available_bytes,
                    mount_locations: volume.mount_locations,
                    reader_fingerprint: volume.reader_topology.as_ref().map(|topology| {
                        let source = format!(
                            "{}|{}|{}",
                            topology.vendor.as_deref().unwrap_or("unknown"),
                            topology.product.as_deref().unwrap_or("unknown"),
                            topology.reader_serial.as_deref().unwrap_or("unknown"),
                        );
                        let candidate = crate::identity::IdentityCandidate::new(
                            "windows.storage-descriptor.reader",
                            IdentityScope::Reader,
                            source,
                            crate::identity::IdentityStrength::Topology,
                        )
                        .expect("reader topology fingerprint has a stable fallback");
                        derive_key(IdentityScope::Reader, &candidate)
                    }),
                    reader_family: volume
                        .reader_topology
                        .as_ref()
                        .and_then(device_discovery::ReaderTopology::recognized_family),
                    reader_slot: volume.reader_topology.as_ref().and_then(|topology| {
                        topology
                            .logical_unit
                            .map(|logical_unit| format!("Logical unit {logical_unit}"))
                    }),
                },
            }
        })
        .collect();
    DeviceSnapshot { sequence, devices }
}

fn apply_calibrated_slots(snapshot: &mut DeviceSnapshot, store: &LocalStore) {
    for device in &mut snapshot.devices {
        let (Some(reader_fingerprint), Some(logical_unit)) = (
            device.details.reader_fingerprint.as_deref(),
            device
                .details
                .reader_slot
                .as_deref()
                .and_then(|slot| slot.strip_prefix("Logical unit "))
                .and_then(|value| value.parse::<u8>().ok()),
        ) else {
            continue;
        };
        if let Ok(Some(slot_kind)) = store.reader_slot_kind(reader_fingerprint, logical_unit) {
            device.details.reader_slot = Some(match slot_kind {
                ReaderSlotKind::Sd => "SD slot (calibrated)".into(),
                ReaderSlotKind::MicroSd => "microSD slot (calibrated)".into(),
            });
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(
            tauri_plugin_log::Builder::new()
                .clear_targets()
                .targets([
                    tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::Stdout),
                    tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::LogDir {
                        file_name: Some("media-ingest-tool".into()),
                    }),
                ])
                .level(log::LevelFilter::Debug)
                .max_file_size(5 * 1024 * 1024)
                .rotation_strategy(tauri_plugin_log::RotationStrategy::KeepAll)
                .timezone_strategy(tauri_plugin_log::TimezoneStrategy::UseLocal)
                .build(),
        )
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            info!(
                target: "media_ingest_tool::support",
                "application_start version={} platform={} verbose_support_logging=true",
                env!("CARGO_PKG_VERSION"),
                std::env::consts::OS
            );
            let data_directory = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_directory)?;
            let mut store = LocalStore::open(data_directory.join("media-ingest.sqlite3"))?;
            store.reconcile_interrupted_runs()?;
            info!(
                target: "media_ingest_tool::support",
                "local_store_ready interrupted_runs_reconciled=true"
            );
            app.manage(AppState {
                store: Arc::new(Mutex::new(store)),
                active_ingests: Arc::new(Mutex::new(HashMap::new())),
                destination_leases: Arc::new(DestinationLeaseRegistry::default()),
                format_authorizations: Arc::new(Mutex::new(HashMap::new())),
                connection_generations: Arc::new(
                    Mutex::new(ConnectionGenerationTracker::default()),
                ),
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            let state = window.state::<AppState>();
            match event {
                tauri::WindowEvent::CloseRequested { api, .. } => {
                    if has_active_ingests(&state.active_ingests) {
                        api.prevent_close();
                        let _ = window.emit("media-ingest://close-requested", ());
                    }
                }
                // Desktop Tauri does not expose suspend/resume window events.
                // Recheck on foreground focus, which is the first reliable
                // operator-visible point after a sleep/wake cycle.
                tauri::WindowEvent::Focused(true) => {
                    let _ = window.emit("media-ingest://reactivated", ());
                }
                _ => {}
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_device_snapshot,
            get_ingest_history,
            scan_source_inventory,
            get_remembered_destination,
            remember_destination,
            register_card_marker,
            get_auto_ingest_profile,
            get_format_eligibility,
            safe_eject,
            preview_verified_ingest,
            request_format_authorization,
            request_force_format_authorization,
            execute_format_authorization,
            watch_device_snapshots,
            calibrate_reader_slot,
            start_verified_ingest,
            resume_verified_ingest,
            cancel_verified_ingest
        ])
        .run(tauri::generate_context!())
        .expect("error while running media-ingest desktop application");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_gate_bounds_rapid_worker_events_without_delaying_the_next_cadence() {
        let gate = ProgressEmissionGate {
            last_emitted: Mutex::new(None),
        };
        let now = Instant::now();
        assert!(gate.should_emit(now));
        assert!(!gate.should_emit(now + Duration::from_millis(99)));
        assert!(gate.should_emit(now + PROGRESS_EVENT_MIN_INTERVAL));
    }

    #[test]
    fn fake_snapshot_is_deterministic_and_empty() {
        let provider = DeterministicDeviceSnapshotProvider;
        assert_eq!(provider.snapshot(), provider.snapshot());
        assert!(provider.snapshot().devices.is_empty());
    }

    #[test]
    fn safe_eject_allows_a_current_marker_backed_card_to_reach_native_resolution() {
        let database =
            std::env::temp_dir().join(format!("safe-eject-{}.sqlite3", uuid::Uuid::new_v4()));
        let state = AppState {
            store: Arc::new(Mutex::new(LocalStore::open(&database).expect("store"))),
            active_ingests: Arc::new(Mutex::new(HashMap::new())),
            destination_leases: Arc::new(DestinationLeaseRegistry::default()),
            format_authorizations: Arc::new(Mutex::new(HashMap::new())),
            connection_generations: Arc::new(Mutex::new(ConnectionGenerationTracker::default())),
        };
        let request = SafeEjectRequest {
            run_id: "completed-run".into(),
            source_medium_key: "filesystem:mutable".into(),
            source_generation: 1,
            source_identity_confidence: IdentityConfidence::Unresolved,
        };
        assert_eq!(
            safe_eject_mount(&request, &state),
            Err("No sealed completed receipt matches the current source card")
        );
        drop(state);
        let _ = std::fs::remove_file(database);
    }

    #[test]
    fn close_gate_recognizes_an_active_ingest() {
        let active = Mutex::new(HashMap::new());
        assert!(!has_active_ingests(&active));
        active.lock().expect("active ingest store").insert(
            "run".into(),
            ActiveIngest {
                cancellation: Arc::new(std::sync::atomic::AtomicBool::new(false)),
                source_medium_key: "hardware:fixture".into(),
                source_root: "F:\\".into(),
                source_seen_in_snapshot: true,
            },
        );
        assert!(has_active_ingests(&active));
    }

    #[test]
    fn auto_format_never_runs_for_a_manual_or_empty_ingest() {
        let database =
            std::env::temp_dir().join(format!("auto-format-{}.sqlite3", uuid::Uuid::new_v4()));
        let store = Mutex::new(LocalStore::open(&database).expect("store"));
        let generations = Mutex::new(ConnectionGenerationTracker::default());
        let active_ingests = Mutex::new(HashMap::new());
        let result = VerifiedIngestResult {
            operation_id: "10000000-0000-4000-8000-000000000040".into(),
            source_medium_key: "v1:card".into(),
            source_generation: 1,
            copied_files: 1,
            copied_bytes: 1,
            receipt_name: "receipt.json".into(),
            source_marker_status: SourceMarkerStatus::Recognized,
            auto_format_status: AutoFormatStatus::NotConfigured,
        };
        assert_eq!(
            maybe_auto_format_after_verified_ingest(
                &result,
                std::path::Path::new("not-used-for-manual-ingest"),
                None,
                false,
                &store,
                &generations,
                &active_ingests,
            ),
            AutoFormatStatus::NotConfigured
        );
        let empty = VerifiedIngestResult {
            copied_files: 0,
            ..result.clone()
        };
        assert_eq!(
            maybe_auto_format_after_verified_ingest(
                &empty,
                std::path::Path::new("not-used-for-empty-ingest"),
                None,
                true,
                &store,
                &generations,
                &active_ingests,
            ),
            AutoFormatStatus::Skipped
        );
        let competing_cancellation = Arc::new(std::sync::atomic::AtomicBool::new(false));
        active_ingests.lock().expect("active ingests").insert(
            "other-run".into(),
            ActiveIngest {
                cancellation: competing_cancellation,
                source_medium_key: result.source_medium_key.clone(),
                source_root: "other-mounted-root".into(),
                source_seen_in_snapshot: false,
            },
        );
        assert_eq!(
            maybe_auto_format_after_verified_ingest(
                &result,
                std::path::Path::new("not-used-for-competing-ingest"),
                None,
                true,
                &store,
                &generations,
                &active_ingests,
            ),
            AutoFormatStatus::Skipped
        );
        drop(store);
        std::fs::remove_file(database).expect("cleanup store");
    }

    #[test]
    fn empty_auto_ingest_is_skipped_before_it_can_persist_a_failed_run() {
        let request = VerifiedIngestRequest {
            operation_id: None,
            source_root: "M:\\".into(),
            destination_root: "F:\\ingest".into(),
            source_medium_key: "v1:managed-card".into(),
            source_identity_confidence: IdentityConfidence::Unresolved,
            source_generation: 22,
            max_workers: 2,
            sort_mode: IngestSortMode::CameraDay,
            interval_minutes: None,
            custom_directory_fields: Vec::new(),
            destination_depth_order: None,
            auto_ingest_triggered: true,
        };
        assert!(skips_empty_auto_ingest(&request, 0));
        assert!(!skips_empty_auto_ingest(&request, 1));
        assert!(!skips_empty_auto_ingest(
            &VerifiedIngestRequest {
                auto_ingest_triggered: false,
                ..request
            },
            0
        ));
    }

    #[test]
    fn response_serializes_a_versioned_typed_envelope() {
        let response = IpcResponse::success(DeviceSnapshot {
            sequence: 4,
            devices: Vec::new(),
        });
        let json = serde_json::to_value(response).expect("response serializes");

        assert_eq!(json["contractVersion"], IPC_CONTRACT_VERSION);
        assert_eq!(json["data"]["sequence"], 4);
        assert!(json["error"].is_null());
    }

    #[test]
    fn source_inventory_requires_the_current_medium_and_mount_pairing() {
        let volume = device_discovery::volume_from_parts(
            "F:\\".into(),
            "CARD".into(),
            Some("exFAT".into()),
            None,
            Some(100),
            Some(50),
        );
        let snapshot = snapshot_from_volumes(1, vec![volume]);
        let request = SourceInventoryRequest {
            source_root: "f:\\".into(),
            source_medium_key: snapshot.devices[0].identity.media_key.clone(),
        };
        assert!(source_inventory_matches_current_device(&snapshot, &request));

        let stale_mount = SourceInventoryRequest {
            source_root: "G:\\".into(),
            ..request.clone()
        };
        assert!(!source_inventory_matches_current_device(
            &snapshot,
            &stale_mount
        ));

        let replaced_medium = SourceInventoryRequest {
            source_medium_key: "v1:another-card".into(),
            ..request
        };
        assert!(!source_inventory_matches_current_device(
            &snapshot,
            &replaced_medium
        ));
    }

    #[test]
    fn destination_recall_uses_only_the_current_strong_medium_identity() {
        let snapshot = DeviceSnapshot {
            sequence: 1,
            devices: vec![StorageDevice {
                state: DeviceState::Available,
                connection_generation: 0,
                identity: DeviceIdentity {
                    media_key: "v1:exact-card".into(),
                    confidence: IdentityConfidence::HardwareImmutable,
                    evidence: Vec::new(),
                },
                details: StorageDeviceDetails {
                    display_name: "card".into(),
                    filesystem: Some("exFAT".into()),
                    total_bytes: Some(1),
                    available_bytes: Some(1),
                    mount_locations: vec!["F:\\".into()],
                    reader_fingerprint: None,
                    reader_family: None,
                    reader_slot: None,
                },
            }],
        };
        let identity =
            source_identity_for_current_device(&snapshot, "v1:exact-card").expect("current card");
        assert!(allows_destination_recall(&identity));
        assert_eq!(
            identity.strength,
            crate::identity::IdentityStrength::HardwareStrong
        );
        assert!(source_identity_for_current_device(&snapshot, "v1:other-card").is_none());

        let unresolved = source_identity_record("v1:reader-slot", IdentityConfidence::Unresolved);
        assert!(!allows_destination_recall(&unresolved));
    }

    #[test]
    fn force_reformat_postcheck_accepts_a_current_unresolved_removable_mount() {
        let device = StorageDevice {
            state: DeviceState::Available,
            connection_generation: 4,
            identity: DeviceIdentity {
                media_key: "session:unresolved-card".into(),
                confidence: IdentityConfidence::Unresolved,
                evidence: Vec::new(),
            },
            details: StorageDeviceDetails {
                display_name: "unregistered card".into(),
                filesystem: Some("exFAT".into()),
                total_bytes: Some(64_000_000_000),
                available_bytes: Some(32_000_000_000),
                mount_locations: vec!["F:\\".into()],
                reader_fingerprint: None,
                reader_family: None,
                reader_slot: None,
            },
        };
        assert!(same_force_format_target(
            &device,
            Path::new("F:\\"),
            64_000_000_000
        ));
        assert!(!same_force_format_target(
            &device,
            Path::new("G:\\"),
            64_000_000_000
        ));
        assert!(!same_force_format_target(
            &device,
            Path::new("F:\\"),
            32_000_000_000
        ));
    }

    #[test]
    fn connection_generation_changes_only_after_an_observed_removal() {
        let device = || StorageDevice {
            state: DeviceState::Available,
            connection_generation: 0,
            identity: DeviceIdentity {
                media_key: "v1:exact-card".into(),
                confidence: IdentityConfidence::HardwareImmutable,
                evidence: Vec::new(),
            },
            details: StorageDeviceDetails {
                display_name: "card".into(),
                filesystem: Some("exFAT".into()),
                total_bytes: Some(1),
                available_bytes: Some(1),
                mount_locations: vec!["F:\\".into()],
                reader_fingerprint: None,
                reader_family: None,
                reader_slot: None,
            },
        };
        let mut tracker = ConnectionGenerationTracker::default();
        let mut first = DeviceSnapshot {
            sequence: 1,
            devices: vec![device()],
        };
        assign_connection_generations(&mut first, &mut tracker);
        assert_eq!(first.devices[0].connection_generation, 1);
        let request = VerifiedIngestRequest {
            operation_id: None,
            source_root: "F:\\".into(),
            destination_root: "D:\\Ingest".into(),
            source_medium_key: "v1:exact-card".into(),
            source_identity_confidence: IdentityConfidence::HardwareImmutable,
            source_generation: 1,
            max_workers: 1,
            sort_mode: IngestSortMode::OriginalTree,
            interval_minutes: None,
            custom_directory_fields: Vec::new(),
            destination_depth_order: None,
            auto_ingest_triggered: false,
        };
        assert!(current_source_for_ingest_request(&first, &request).is_some());

        let mut refreshed = DeviceSnapshot {
            sequence: 2,
            devices: vec![device()],
        };
        assign_connection_generations(&mut refreshed, &mut tracker);
        assert_eq!(refreshed.devices[0].connection_generation, 1);

        let mut removed = DeviceSnapshot {
            sequence: 3,
            devices: Vec::new(),
        };
        assign_connection_generations(&mut removed, &mut tracker);

        let mut reinserted = DeviceSnapshot {
            sequence: 4,
            devices: vec![device()],
        };
        assign_connection_generations(&mut reinserted, &mut tracker);
        assert_eq!(reinserted.devices[0].connection_generation, 2);
        assert!(current_source_for_ingest_request(&reinserted, &request).is_none());

        let recovery = RecoverableIngestRun {
            run_id: "interrupted".into(),
            source_identity_key: "v1:exact-card".into(),
            source_generation: 1,
            source_root: "F:\\".into(),
            destination_root: "D:\\Ingest".into(),
            auto_ingest_triggered: false,
        };
        assert!(current_source_for_recovery(&first, &recovery).is_some());
        assert!(current_source_for_recovery(&reinserted, &recovery).is_some());

        let mut unresolved_same_mount = first.clone();
        unresolved_same_mount.devices[0].identity.confidence = IdentityConfidence::Unresolved;
        assert!(current_source_for_recovery(&unresolved_same_mount, &recovery).is_some());

        let mut unresolved_reinsert = unresolved_same_mount;
        unresolved_reinsert.devices[0].connection_generation = 2;
        assert!(current_source_for_recovery(&unresolved_reinsert, &recovery).is_none());
    }

    #[test]
    fn persisted_generation_preserves_a_mounted_insertion_across_restart() {
        let device = || StorageDevice {
            state: DeviceState::Available,
            connection_generation: 0,
            identity: DeviceIdentity {
                media_key: "marker:registered-card".into(),
                confidence: IdentityConfidence::Unresolved,
                evidence: Vec::new(),
            },
            details: StorageDeviceDetails {
                display_name: "card".into(),
                filesystem: Some("exFAT".into()),
                total_bytes: Some(1),
                available_bytes: Some(1),
                mount_locations: vec!["F:\\".into()],
                reader_fingerprint: None,
                reader_family: None,
                reader_slot: None,
            },
        };
        let mut tracker = ConnectionGenerationTracker::default();
        seed_connection_generations(&mut tracker, [("marker:registered-card".into(), 7)]);

        let mut after_restart = DeviceSnapshot {
            sequence: 1,
            devices: vec![device()],
        };
        assign_connection_generations(&mut after_restart, &mut tracker);
        assert_eq!(after_restart.devices[0].connection_generation, 7);

        let mut removed = DeviceSnapshot {
            sequence: 2,
            devices: Vec::new(),
        };
        assign_connection_generations(&mut removed, &mut tracker);
        let mut reinserted = DeviceSnapshot {
            sequence: 3,
            devices: vec![device()],
        };
        assign_connection_generations(&mut reinserted, &mut tracker);
        assert_eq!(reinserted.devices[0].connection_generation, 8);
    }

    #[test]
    fn identity_contract_keeps_mutable_evidence_explicit() {
        let evidence = IdentityEvidence {
            kind: IdentityEvidenceKind::DriveLetter,
            fingerprint: "local:drive-letter-placeholder".into(),
            immutable: false,
        };
        let json = serde_json::to_value(evidence).expect("evidence serializes");

        assert_eq!(json["kind"], "drive_letter");
        assert_eq!(json["immutable"], false);
    }

    #[test]
    fn filesystem_marker_evidence_does_not_replace_the_native_operation_key() {
        let mut volume = device_discovery::volume_from_parts(
            "F:\\".into(),
            "CAMERA_CARD".into(),
            Some("exFAT".into()),
            Some("1111-2222".into()),
            Some(100),
            Some(50),
        );
        volume.marker_token = Some("MIT1:00000000-0000-4000-8000-000000000000".into());
        let mut remounted_volume = device_discovery::volume_from_parts(
            "G:\\".into(),
            "CAMERA_CARD".into(),
            Some("exFAT".into()),
            Some("3333-4444".into()),
            Some(100),
            Some(50),
        );
        remounted_volume.marker_token = volume.marker_token.clone();
        let snapshot = snapshot_from_volumes(12, vec![volume, remounted_volume]);

        assert_eq!(snapshot.sequence, 12);
        assert_eq!(snapshot.devices.len(), 2);
        assert_eq!(
            snapshot.devices[0].identity.confidence,
            IdentityConfidence::Unresolved
        );
        assert!(!snapshot.devices[0].identity.evidence[0].immutable);
        let marker = snapshot.devices[0]
            .identity
            .evidence
            .iter()
            .find(|evidence| evidence.kind == IdentityEvidenceKind::AppMarker)
            .expect("marker evidence");
        assert!(!marker.immutable);
        assert!(!marker.fingerprint.contains("MIT1"));
        assert_ne!(
            snapshot.devices[0].identity.media_key,
            snapshot.devices[1].identity.media_key
        );
    }

    #[test]
    fn reader_topology_is_exposed_as_a_fingerprint_not_raw_serial() {
        let mut volume = device_discovery::volume_from_parts(
            "F:\\".into(),
            "CAMERA_CARD".into(),
            Some("exFAT".into()),
            None,
            Some(100),
            Some(50),
        );
        volume.reader_topology = Some(device_discovery::ReaderTopology {
            vendor: Some("SanDisk".into()),
            product: Some("PRO-READER".into()),
            reader_serial: Some("RAW-SERIAL-MUST-NOT-CROSS-IPC".into()),
            physical_device_number: Some(2),
            logical_unit: Some(1),
            reported_vpd_identifiers: vec!["vpd83:1:2:0:RAW-CARD-ID".into()],
            reported_sd_cid: None,
        });
        let snapshot = snapshot_from_volumes(1, vec![volume]);
        let details = &snapshot.devices[0].details;
        assert_eq!(
            snapshot.devices[0].identity.confidence,
            IdentityConfidence::Unresolved
        );
        assert!(snapshot.devices[0].identity.media_key.starts_with("v1:"));
        assert!(details.reader_fingerprint.is_some());
        assert!(!details
            .reader_fingerprint
            .as_deref()
            .is_some_and(|value| value.contains("RAW-SERIAL")));
        assert_eq!(details.reader_slot.as_deref(), Some("Logical unit 1"));
        let vpd = snapshot.devices[0]
            .identity
            .evidence
            .iter()
            .find(|evidence| evidence.kind == IdentityEvidenceKind::StorageVpd)
            .expect("VPD evidence");
        assert!(!vpd.immutable);
        assert!(!vpd.fingerprint.contains("RAW-CARD-ID"));
    }

    #[test]
    fn direct_sd_cid_becomes_the_only_immutable_medium_identity() {
        let mut volume = device_discovery::volume_from_parts(
            "F:\\".into(),
            "CAMERA_CARD".into(),
            Some("exFAT".into()),
            Some("FORMAT-CHANGES".into()),
            Some(100),
            Some(50),
        );
        volume.marker_token = Some("MIT1:00000000-0000-4000-8000-000000000000".into());
        volume.reader_topology = Some(device_discovery::ReaderTopology {
            vendor: Some("native controller".into()),
            product: Some("SD host".into()),
            reader_serial: None,
            physical_device_number: Some(2),
            logical_unit: Some(0),
            reported_vpd_identifiers: vec!["reader-scoped-vpd".into()],
            reported_sd_cid: Some("03534453434F44455800000001ABCDEF".into()),
        });
        let snapshot = snapshot_from_volumes(1, vec![volume]);
        let device = &snapshot.devices[0];
        assert_eq!(
            device.identity.confidence,
            IdentityConfidence::HardwareImmutable
        );
        let cid = device
            .identity
            .evidence
            .iter()
            .find(|evidence| evidence.kind == IdentityEvidenceKind::SdCid)
            .expect("CID evidence");
        assert!(cid.immutable);
        assert!(!cid.fingerprint.contains("035344"));
        assert_ne!(device.identity.media_key, "FORMAT-CHANGES");
    }

    #[test]
    fn calibrated_reader_lun_projects_only_the_matching_slot() {
        let root = std::env::temp_dir().join(format!("slot-projection-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).expect("root");
        let mut store = LocalStore::open(root.join("store.sqlite3")).expect("store");
        let mut volume = device_discovery::volume_from_parts(
            "F:\\".into(),
            "CARD".into(),
            Some("exFAT".into()),
            None,
            Some(100),
            Some(50),
        );
        volume.reader_topology = Some(device_discovery::ReaderTopology {
            vendor: Some("SanDisk".into()),
            product: Some("PRO-READER".into()),
            reader_serial: Some("reader-1".into()),
            physical_device_number: Some(2),
            logical_unit: Some(1),
            reported_vpd_identifiers: Vec::new(),
            reported_sd_cid: None,
        });
        let mut snapshot = snapshot_from_volumes(1, vec![volume]);
        let fingerprint = snapshot.devices[0]
            .details
            .reader_fingerprint
            .clone()
            .expect("fingerprint");
        store
            .save_reader_slot_calibration(
                &fingerprint,
                1,
                ReaderSlotKind::MicroSd,
                "controlled microSD insertion",
            )
            .expect("calibrate");
        apply_calibrated_slots(&mut snapshot, &store);
        assert_eq!(
            snapshot.devices[0].details.reader_slot.as_deref(),
            Some("microSD slot (calibrated)")
        );
        drop(store);
        std::fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn identity_confidence_is_explicit_in_receipts() {
        assert_eq!(
            identity_confidence_name(IdentityConfidence::HardwareImmutable),
            "hardware_immutable"
        );
        assert_eq!(
            identity_confidence_name(IdentityConfidence::Unresolved),
            "unresolved"
        );
    }

    #[test]
    fn source_removal_cancels_only_after_the_source_was_observed() {
        let cancellation = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let active_ingests = Mutex::new(HashMap::from([(
            "active-run".into(),
            ActiveIngest {
                cancellation: Arc::clone(&cancellation),
                source_medium_key: "v1:card".into(),
                source_root: "F:\\".into(),
                source_seen_in_snapshot: false,
            },
        )]));
        let present = DeviceSnapshot {
            sequence: 1,
            devices: vec![StorageDevice {
                state: DeviceState::Available,
                connection_generation: 0,
                identity: DeviceIdentity {
                    media_key: "v1:card".into(),
                    confidence: IdentityConfidence::Unresolved,
                    evidence: Vec::new(),
                },
                details: StorageDeviceDetails {
                    display_name: "card".into(),
                    filesystem: Some("exFAT".into()),
                    total_bytes: Some(1),
                    available_bytes: Some(1),
                    mount_locations: vec!["F:\\".into()],
                    reader_fingerprint: None,
                    reader_family: None,
                    reader_slot: None,
                },
            }],
        };
        cancel_missing_active_sources(&active_ingests, &present);
        assert!(!cancellation.load(Ordering::Acquire));

        cancel_missing_active_sources(
            &active_ingests,
            &DeviceSnapshot {
                sequence: 2,
                devices: Vec::new(),
            },
        );
        assert!(cancellation.load(Ordering::Acquire));

        let late_observer_cancellation = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let late_observer = Mutex::new(HashMap::from([(
            "late-run".into(),
            ActiveIngest {
                cancellation: Arc::clone(&late_observer_cancellation),
                source_medium_key: "v1:late-card".into(),
                source_root: "G:\\".into(),
                source_seen_in_snapshot: false,
            },
        )]));
        cancel_missing_active_sources(
            &late_observer,
            &DeviceSnapshot {
                sequence: 3,
                devices: Vec::new(),
            },
        );
        assert!(!late_observer_cancellation.load(Ordering::Acquire));
    }

    #[test]
    fn completion_persistence_seals_before_creating_a_source_marker() {
        let root = std::env::temp_dir().join(format!("native-ingest-{}", uuid::Uuid::new_v4()));
        let source = root.join("source");
        let destination = root.join("destination");
        std::fs::create_dir_all(&source).expect("source");
        std::fs::write(source.join("clip.mov"), b"verified camera bytes").expect("fixture");
        let prepared = prepare_verified_ingest(
            VerifiedIngestRequest {
                operation_id: None,
                source_root: source.to_string_lossy().into(),
                destination_root: destination.to_string_lossy().into(),
                source_medium_key: "session:fixture".into(),
                source_identity_confidence: IdentityConfidence::Unresolved,
                source_generation: 7,
                max_workers: 2,
                sort_mode: IngestSortMode::OriginalTree,
                interval_minutes: None,
                custom_directory_fields: Vec::new(),
                destination_depth_order: None,
                auto_ingest_triggered: false,
            },
            "test-operation".into(),
        )
        .expect("prepare");
        assert_eq!(prepared.files.len(), 1);
        let planned_entry_id = prepared.files[0].entry_id.clone();
        let source_identity = source_identity_for_request(&prepared.request);
        let mut store = LocalStore::open(root.join("store.sqlite3")).expect("store");
        store
            .begin_ingest_run(
                "test-operation",
                &source_identity,
                7,
                &source.to_string_lossy(),
                &destination.to_string_lossy(),
            )
            .expect("begin run");
        for file in &prepared.files {
            assert!(store
                .record_planned_file(
                    "test-operation",
                    &PlannedFileRecord {
                        entry_id: file.entry_id.clone(),
                        source_relative_path: file
                            .source
                            .relative_path
                            .to_string_lossy()
                            .replace('\\', "/"),
                        destination_relative_path: file
                            .destination_relative_path
                            .to_string_lossy()
                            .replace('\\', "/"),
                        byte_length: file.source.byte_length,
                    },
                )
                .expect("plan file"));
        }
        assert!(store
            .transition_ingest_run("test-operation", IngestRunState::Copying, "plan persisted")
            .expect("copying state"));
        let result = execute_prepared_ingest(
            prepared,
            Arc::new(std::sync::atomic::AtomicBool::new(false)),
            None,
        )
        .expect("ingest");
        assert_eq!(result.summary.copied_files, 1);
        assert_eq!(result.summary.copied_bytes, 21);
        assert_eq!(
            result.summary.source_marker_status,
            SourceMarkerStatus::Unavailable
        );
        assert!(crate::storage_marker::read_marker(&source)
            .expect("marker read")
            .is_none());
        assert!(persist_verified_completion(
            &mut store,
            "test-operation",
            &result
        ));
        assert!(store
            .has_completed_receipt_for_source("test-operation", &source_identity.identity_key, 7)
            .expect("completed receipt"));
        drop(store);
        assert_eq!(
            source_marker_status(&result.source_root),
            SourceMarkerStatus::Created
        );
        assert_eq!(result.copies.len(), 1);
        assert_eq!(result.copies[0].entry_id, planned_entry_id);
        assert_eq!(
            std::fs::read(destination.join("clip.mov")).expect("copied file"),
            b"verified camera bytes"
        );
        let receipt = std::fs::read_to_string(
            destination
                .join(".media-ingest-receipts")
                .join(result.summary.receipt_name),
        )
        .expect("receipt");
        assert!(receipt.contains("session_only") || receipt.contains("unresolved"));
        std::fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn recovery_rehashes_published_files_and_copies_only_the_missing_plan_entry() {
        let root = std::env::temp_dir().join(format!("recovery-ingest-{}", uuid::Uuid::new_v4()));
        let source = root.join("source");
        let destination = root.join("destination");
        std::fs::create_dir_all(&source).expect("source");
        std::fs::write(source.join("first.mov"), b"first camera bytes").expect("first");
        std::fs::write(source.join("second.mov"), b"second camera bytes").expect("second");
        let request = VerifiedIngestRequest {
            operation_id: Some("10000000-0000-4000-8000-000000000099".into()),
            source_root: source.to_string_lossy().into(),
            destination_root: destination.to_string_lossy().into(),
            source_medium_key: "session:recovery-fixture".into(),
            source_identity_confidence: IdentityConfidence::Unresolved,
            source_generation: 1,
            max_workers: 2,
            sort_mode: IngestSortMode::OriginalTree,
            interval_minutes: None,
            custom_directory_fields: Vec::new(),
            destination_depth_order: None,
            auto_ingest_triggered: false,
        };
        let initial = execute_prepared_ingest(
            prepare_verified_ingest(request.clone(), request.operation_id.clone().expect("id"))
                .expect("initial plan"),
            Arc::new(std::sync::atomic::AtomicBool::new(false)),
            None,
        )
        .expect("initial copy");
        let missing_path = initial
            .copies
            .iter()
            .find(|copy| copy.source_relative_path == std::path::Path::new("second.mov"))
            .expect("second copy")
            .final_path
            .clone();
        std::fs::remove_file(missing_path).expect("simulate missing final file");
        let recovered = execute_recovery_ingest(
            prepare_verified_ingest(request.clone(), request.operation_id.expect("id"))
                .expect("recovery plan"),
            Arc::new(std::sync::atomic::AtomicBool::new(false)),
            None,
        )
        .expect("recovery");
        assert_eq!(recovered.summary.copied_files, 2);
        assert_eq!(
            recovered
                .copies
                .iter()
                .map(|copy| copy.final_path.exists())
                .collect::<Vec<_>>(),
            vec![true, true]
        );
        std::fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    #[ignore = "requires the connected sacrificial SD card and F:\\dummy\\1"]
    fn hardware_sd_card_verified_ingest_probe() {
        let source = PathBuf::from(r"D:\DCIM\100DUMMY");
        let destination = PathBuf::from(r"F:\dummy\1\.media-ingest-hardware-tests")
            .join(format!("copy-1-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&destination).expect("hardware-test destination");
        let prepared = prepare_verified_ingest(
            VerifiedIngestRequest {
                operation_id: Some("10000000-0000-4000-8000-000000000001".into()),
                source_root: source.to_string_lossy().into(),
                destination_root: destination.to_string_lossy().into(),
                source_medium_key: "hardware-probe-only".into(),
                source_identity_confidence: IdentityConfidence::Unresolved,
                source_generation: 1,
                max_workers: 2,
                sort_mode: IngestSortMode::OriginalTree,
                interval_minutes: None,
                custom_directory_fields: Vec::new(),
                destination_depth_order: None,
                auto_ingest_triggered: false,
            },
            "10000000-0000-4000-8000-000000000001".into(),
        )
        .expect("live card plan");
        assert_eq!(
            prepared.files.len(),
            3,
            "only the controlled fixture is expected"
        );
        let result = execute_prepared_ingest(
            prepared,
            Arc::new(std::sync::atomic::AtomicBool::new(false)),
            None,
        )
        .expect("live card verified ingest");
        assert_eq!(result.summary.copied_files, 3);
        assert_eq!(result.summary.copied_bytes, 1_310_762);
        assert!(destination
            .join(".media-ingest-receipts")
            .join(result.summary.receipt_name)
            .exists());
    }

    #[test]
    #[ignore = "requires the connected sacrificial SD card and F:\\dummy\\2"]
    fn hardware_sd_card_second_destination_probe() {
        let source = PathBuf::from(r"D:\DCIM\100DUMMY");
        let destination = PathBuf::from(r"F:\dummy\2\.media-ingest-hardware-tests")
            .join(format!("copy-2-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&destination).expect("hardware-test destination");
        let prepared = prepare_verified_ingest(
            VerifiedIngestRequest {
                operation_id: Some("10000000-0000-4000-8000-000000000002".into()),
                source_root: source.to_string_lossy().into(),
                destination_root: destination.to_string_lossy().into(),
                source_medium_key: "hardware-probe-only".into(),
                source_identity_confidence: IdentityConfidence::Unresolved,
                source_generation: 1,
                max_workers: 2,
                sort_mode: IngestSortMode::OriginalTree,
                interval_minutes: None,
                custom_directory_fields: Vec::new(),
                destination_depth_order: None,
                auto_ingest_triggered: false,
            },
            "10000000-0000-4000-8000-000000000002".into(),
        )
        .expect("second live card plan");
        let result = execute_prepared_ingest(
            prepared,
            Arc::new(std::sync::atomic::AtomicBool::new(false)),
            None,
        )
        .expect("second live card verified ingest");
        assert_eq!(
            (result.summary.copied_files, result.summary.copied_bytes),
            (3, 1_310_762)
        );
    }

    #[test]
    #[ignore = "requires both connected sacrificial cards and F:\\media-ingest-hardware-tests"]
    fn hardware_two_card_concurrent_verified_ingest_probe() {
        let root = PathBuf::from(r"F:\media-ingest-hardware-tests")
            .join(format!("two-card-concurrent-{}", uuid::Uuid::new_v4()));
        let sd_destination = root.join("sd");
        let microsd_destination = root.join("microsd");
        std::fs::create_dir_all(&sd_destination).expect("SD destination");
        std::fs::create_dir_all(&microsd_destination).expect("microSD destination");

        let sd = std::thread::spawn(move || {
            let request = VerifiedIngestRequest {
                operation_id: Some("10000000-0000-4000-8000-000000000003".into()),
                source_root: r"D:\DCIM\100DUMMY".into(),
                destination_root: sd_destination.to_string_lossy().into(),
                source_medium_key: "hardware-sd-concurrent-probe".into(),
                source_identity_confidence: IdentityConfidence::Unresolved,
                source_generation: 1,
                max_workers: 2,
                sort_mode: IngestSortMode::OriginalTree,
                interval_minutes: None,
                custom_directory_fields: Vec::new(),
                destination_depth_order: None,
                auto_ingest_triggered: false,
            };
            let prepared = prepare_verified_ingest(
                request.clone(),
                request.operation_id.clone().expect("operation id"),
            )
            .expect("SD plan");
            let result = execute_prepared_ingest(
                prepared,
                Arc::new(std::sync::atomic::AtomicBool::new(false)),
                None,
            )
            .expect("SD verified ingest");
            assert_eq!(
                (result.summary.copied_files, result.summary.copied_bytes),
                (3, 1_310_762)
            );
            result
        });
        let microsd = std::thread::spawn(move || {
            let request = VerifiedIngestRequest {
                operation_id: Some("10000000-0000-4000-8000-000000000004".into()),
                source_root: r"M:\DCIM\100CERT".into(),
                destination_root: microsd_destination.to_string_lossy().into(),
                source_medium_key: "hardware-microsd-concurrent-probe".into(),
                source_identity_confidence: IdentityConfidence::Unresolved,
                source_generation: 1,
                max_workers: 2,
                sort_mode: IngestSortMode::OriginalTree,
                interval_minutes: None,
                custom_directory_fields: Vec::new(),
                destination_depth_order: None,
                auto_ingest_triggered: false,
            };
            let prepared = prepare_verified_ingest(
                request.clone(),
                request.operation_id.clone().expect("operation id"),
            )
            .expect("microSD plan");
            let result = execute_prepared_ingest(
                prepared,
                Arc::new(std::sync::atomic::AtomicBool::new(false)),
                None,
            )
            .expect("microSD verified ingest");
            assert_eq!(
                (result.summary.copied_files, result.summary.copied_bytes),
                (3, 3_670_016)
            );
            result
        });

        let sd = sd.join().expect("SD worker");
        let microsd = microsd.join().expect("microSD worker");
        assert!(root
            .join("sd")
            .join(".media-ingest-receipts")
            .join(sd.summary.receipt_name)
            .exists());
        assert!(root
            .join("microsd")
            .join(".media-ingest-receipts")
            .join(microsd.summary.receipt_name)
            .exists());
    }

    #[test]
    fn prepared_ingest_honors_a_preexisting_cancellation_without_a_receipt() {
        let root = std::env::temp_dir().join(format!("cancelled-ingest-{}", uuid::Uuid::new_v4()));
        let source = root.join("source");
        let destination = root.join("destination");
        std::fs::create_dir_all(&source).expect("source");
        std::fs::write(source.join("clip.mov"), b"camera bytes").expect("fixture");
        let prepared = prepare_verified_ingest(
            VerifiedIngestRequest {
                operation_id: None,
                source_root: source.to_string_lossy().into(),
                destination_root: destination.to_string_lossy().into(),
                source_medium_key: "session:fixture".into(),
                source_identity_confidence: IdentityConfidence::SessionOnly,
                source_generation: 1,
                max_workers: 1,
                sort_mode: IngestSortMode::OriginalTree,
                interval_minutes: None,
                custom_directory_fields: Vec::new(),
                destination_depth_order: None,
                auto_ingest_triggered: false,
            },
            "cancelled-operation".into(),
        )
        .expect("prepare");
        let cancellation = Arc::new(std::sync::atomic::AtomicBool::new(true));
        assert!(matches!(
            execute_prepared_ingest(prepared, cancellation, None),
            Err(IngestError::Cancelled)
        ));
        assert!(!destination.join(".media-ingest-receipts").exists());
        std::fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn camera_day_sort_uses_a_deterministic_non_overwriting_suffix() {
        let root = std::env::temp_dir().join(format!("sort-plan-{}", uuid::Uuid::new_v4()));
        let source = root.join("source");
        std::fs::create_dir_all(source.join("first")).expect("first");
        std::fs::create_dir_all(source.join("second")).expect("second");
        std::fs::write(source.join("first/clip.mov"), b"first").expect("first fixture");
        std::fs::write(source.join("second/clip.mov"), b"second").expect("second fixture");
        let request = VerifiedIngestRequest {
            operation_id: None,
            source_root: source.to_string_lossy().into(),
            destination_root: root.join("destination").to_string_lossy().into(),
            source_medium_key: "session:fixture".into(),
            source_identity_confidence: IdentityConfidence::Unresolved,
            source_generation: 1,
            max_workers: 2,
            sort_mode: IngestSortMode::CameraDay,
            interval_minutes: None,
            custom_directory_fields: vec![
                CustomDirectoryField::new("Photographer", "Ari").expect("field")
            ],
            destination_depth_order: Some(vec![
                DestinationDepthSegment::CameraModel,
                DestinationDepthSegment::CustomField { index: 0 },
                DestinationDepthSegment::CaptureDay,
            ]),
            auto_ingest_triggered: false,
        };
        let plan = plan_copy_files(
            enumerate_regular_files(&source).expect("enumerate"),
            &source,
            &request,
            "run-1",
        )
        .expect("plan");
        assert_eq!(plan.len(), 2);
        assert_ne!(
            plan[0].destination_relative_path,
            plan[1].destination_relative_path
        );
        assert!(plan.iter().any(|file| {
            file.destination_relative_path
                .file_name()
                .and_then(OsStr::to_str)
                .is_some_and(|name| name.starts_with("clip__"))
        }));
        assert!(plan.iter().all(|file| {
            let components = file
                .destination_relative_path
                .components()
                .map(|component| component.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>();
            let camera = components
                .iter()
                .position(|component| component.contains("__"));
            let photographer = components.iter().position(|component| component == "Ari");
            camera.is_some_and(|camera| photographer.is_some_and(|field| camera < field))
        }));
        std::fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn reordered_custom_organization_is_copied_and_verified() {
        let root = std::env::temp_dir().join(format!("organization-copy-{}", uuid::Uuid::new_v4()));
        let source = root.join("source");
        let destination = root.join("destination");
        std::fs::create_dir_all(source.join("DCIM")).expect("source");
        std::fs::write(source.join("DCIM/clip.mov"), b"organized camera bytes").expect("fixture");
        let prepared = prepare_verified_ingest(
            VerifiedIngestRequest {
                operation_id: None,
                source_root: source.to_string_lossy().into(),
                destination_root: destination.to_string_lossy().into(),
                source_medium_key: "session:organization-fixture".into(),
                source_identity_confidence: IdentityConfidence::Unresolved,
                source_generation: 1,
                max_workers: 1,
                sort_mode: IngestSortMode::CameraInterval,
                interval_minutes: Some(30),
                custom_directory_fields: vec![
                    CustomDirectoryField::new("Photographer", "Ari").expect("field")
                ],
                destination_depth_order: Some(vec![
                    DestinationDepthSegment::CustomField { index: 0 },
                    DestinationDepthSegment::CaptureDay,
                    DestinationDepthSegment::CameraModel,
                    DestinationDepthSegment::CaptureInterval,
                ]),
                auto_ingest_triggered: false,
            },
            "organization-copy-operation".into(),
        )
        .expect("prepare");
        let planned_path = prepared.files[0].destination_relative_path.clone();
        let components = planned_path
            .components()
            .map(|component| component.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(components[0], "Ari");
        assert!(components[1].contains('T'));
        assert!(matches!(
            components[1].as_bytes().get(15),
            Some(b'+' | b'-')
        ));
        assert!(components[2].contains("__"));
        assert!(components[3].contains('T'));
        assert!(matches!(
            components[3].as_bytes().get(15),
            Some(b'+' | b'-')
        ));
        assert_eq!(components[4], "clip.mov");

        let completed = execute_prepared_ingest(
            prepared,
            Arc::new(std::sync::atomic::AtomicBool::new(false)),
            None,
        )
        .expect("copy and verify");
        assert_eq!(completed.summary.copied_files, 1);
        assert_eq!(completed.copies[0].destination_relative_path, planned_path);
        assert_eq!(
            std::fs::read(destination.join(&planned_path)).expect("copied media"),
            b"organized camera bytes"
        );
        assert!(destination
            .join(".media-ingest-receipts")
            .join(completed.summary.receipt_name)
            .exists());
        std::fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    #[ignore = "set MEDIA_INGEST_ORGANIZATION_HW_ROOT to a controlled fixture card root"]
    fn hardware_organization_fixture_uses_exif_and_verifies_copies() {
        let source = PathBuf::from(
            std::env::var("MEDIA_INGEST_ORGANIZATION_HW_ROOT")
                .expect("controlled hardware fixture root"),
        );
        let fixture = source.join("DCIM").join("A001.JPG");
        let metadata = inspect(&fixture);
        assert_eq!(metadata.make, "Sony");
        assert_eq!(metadata.model, "FX3");
        assert_eq!(
            metadata.capture_time_source,
            crate::metadata::CaptureTimeSource::ExifOriginalWithOffset
        );
        assert_eq!(
            metadata.capture_time.offset().local_minus_utc(),
            8 * 60 * 60
        );

        let destination = std::env::temp_dir().join(format!(
            "media-ingest-organization-hardware-{}",
            uuid::Uuid::new_v4()
        ));
        let prepared = prepare_verified_ingest(
            VerifiedIngestRequest {
                operation_id: None,
                source_root: source.to_string_lossy().into(),
                destination_root: destination.to_string_lossy().into(),
                source_medium_key: "hardware:organization-fixture".into(),
                source_identity_confidence: IdentityConfidence::HardwareStable,
                source_generation: 1,
                max_workers: 1,
                sort_mode: IngestSortMode::CameraInterval,
                interval_minutes: Some(30),
                custom_directory_fields: vec![
                    CustomDirectoryField::new("Photographer", "Ari").expect("field")
                ],
                destination_depth_order: Some(vec![
                    DestinationDepthSegment::CustomField { index: 0 },
                    DestinationDepthSegment::CameraModel,
                    DestinationDepthSegment::CaptureDay,
                    DestinationDepthSegment::CaptureInterval,
                ]),
                auto_ingest_triggered: false,
            },
            "hardware-organization-fixture".into(),
        )
        .expect("plan");
        assert_eq!(prepared.files.len(), 3);
        let completed = execute_prepared_ingest(
            prepared,
            Arc::new(std::sync::atomic::AtomicBool::new(false)),
            None,
        )
        .expect("verified copy");
        assert_eq!(completed.summary.copied_files, 3);
        for copy in &completed.copies {
            let components = copy
                .destination_relative_path
                .components()
                .map(|component| component.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>();
            assert_eq!(components[0], "Ari");
            assert!(components[1].starts_with("FX3__"));
            assert_eq!(components[2], "20260828T000000+0800");
            assert!(matches!(
                components[3].as_str(),
                "20260828T100000+0800" | "20260828T103000+0800" | "20260828T110000+0800"
            ));
            assert!(copy.final_path.is_file());
        }
        assert!(destination
            .join(".media-ingest-receipts")
            .join(completed.summary.receipt_name)
            .exists());
        std::fs::remove_dir_all(destination).expect("cleanup destination");
    }

    #[test]
    fn prepared_ingest_rejects_incomplete_organization_settings() {
        let root =
            std::env::temp_dir().join(format!("organization-invalid-{}", uuid::Uuid::new_v4()));
        let source = root.join("source");
        std::fs::create_dir_all(&source).expect("source");
        std::fs::write(source.join("clip.mov"), b"fixture").expect("fixture");
        let invalid_interval = VerifiedIngestRequest {
            operation_id: None,
            source_root: source.to_string_lossy().into(),
            destination_root: root.join("destination").to_string_lossy().into(),
            source_medium_key: "session:organization-fixture".into(),
            source_identity_confidence: IdentityConfidence::Unresolved,
            source_generation: 1,
            max_workers: 1,
            sort_mode: IngestSortMode::CameraInterval,
            interval_minutes: None,
            custom_directory_fields: Vec::new(),
            destination_depth_order: None,
            auto_ingest_triggered: false,
        };
        assert!(matches!(
            prepare_verified_ingest(invalid_interval, "invalid-interval".into()),
            Err(IngestError::InvalidPath)
        ));

        let incomplete_order = VerifiedIngestRequest {
            sort_mode: IngestSortMode::CameraDay,
            interval_minutes: None,
            destination_depth_order: Some(Vec::new()),
            ..VerifiedIngestRequest {
                operation_id: None,
                source_root: source.to_string_lossy().into(),
                destination_root: root.join("destination").to_string_lossy().into(),
                source_medium_key: "session:organization-fixture".into(),
                source_identity_confidence: IdentityConfidence::Unresolved,
                source_generation: 1,
                max_workers: 1,
                sort_mode: IngestSortMode::OriginalTree,
                interval_minutes: None,
                custom_directory_fields: Vec::new(),
                destination_depth_order: None,
                auto_ingest_triggered: false,
            }
        };
        assert!(matches!(
            prepare_verified_ingest(incomplete_order, "incomplete-order".into()),
            Err(IngestError::InvalidPath)
        ));
        std::fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn destination_collision_keys_are_casefolded_and_unicode_normalized() {
        let mut used = HashSet::new();
        let first = unique_destination_path(
            PathBuf::from("Camera/CLIP.MOV"),
            std::path::Path::new("DCIM/CLIP.MOV"),
            &mut used,
        )
        .expect("first");
        let case_variant = unique_destination_path(
            PathBuf::from("camera/clip.mov"),
            std::path::Path::new("DCIM/clip.mov"),
            &mut used,
        )
        .expect("case collision resolved");
        let unicode_variant = unique_destination_path(
            PathBuf::from("Camera/cafe\u{301}.mov"),
            std::path::Path::new("DCIM/cafe\u{301}.mov"),
            &mut used,
        )
        .expect("first unicode path");
        let unicode_equivalent = unique_destination_path(
            PathBuf::from("camera/café.mov"),
            std::path::Path::new("DCIM/café.mov"),
            &mut used,
        )
        .expect("unicode collision resolved");
        assert_eq!(first, PathBuf::from("Camera/CLIP.MOV"));
        assert_ne!(case_variant, PathBuf::from("camera/clip.mov"));
        assert_ne!(unicode_equivalent, PathBuf::from("camera/café.mov"));
        assert_ne!(unicode_variant, unicode_equivalent);
    }

    #[test]
    fn plan_preview_operation_id_keeps_unknown_camera_paths_stable() {
        let root = std::env::temp_dir().join(format!("preview-plan-{}", uuid::Uuid::new_v4()));
        let source = root.join("source");
        std::fs::create_dir_all(&source).expect("source");
        std::fs::write(source.join("clip.mov"), b"fixture media").expect("fixture");
        let request = VerifiedIngestRequest {
            operation_id: Some("10000000-0000-4000-8000-000000000003".into()),
            source_root: source.to_string_lossy().into(),
            destination_root: root.join("destination").to_string_lossy().into(),
            source_medium_key: "session:fixture".into(),
            source_identity_confidence: IdentityConfidence::Unresolved,
            source_generation: 1,
            max_workers: 2,
            sort_mode: IngestSortMode::CameraDay,
            interval_minutes: None,
            custom_directory_fields: Vec::new(),
            destination_depth_order: None,
            auto_ingest_triggered: false,
        };
        let first = prepare_verified_ingest(
            request.clone(),
            "10000000-0000-4000-8000-000000000003".into(),
        )
        .expect("first plan");
        let second =
            prepare_verified_ingest(request, "10000000-0000-4000-8000-000000000003".into())
                .expect("second plan");
        assert_eq!(
            first.files[0].destination_relative_path,
            second.files[0].destination_relative_path
        );
        std::fs::remove_dir_all(root).expect("cleanup");
    }
}
