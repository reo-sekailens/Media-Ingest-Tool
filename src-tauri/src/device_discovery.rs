//! Native, read-only removable-volume discovery.
//!
//! This is deliberately a volume adapter, not a claim that a mount location is
//! a medium identity. Platform-specific physical-device/notification adapters
//! enrich this snapshot in later steps.

use crate::identity::{derive_key, IdentityCandidate, IdentityScope, IdentityStrength};
use serde::{Deserialize, Serialize};

#[cfg(windows)]
use std::sync::mpsc::{Receiver, SyncSender};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiscoveryState {
    Ready,
    PermissionDenied,
    Unavailable,
}

/// A product-family recognition hint for presentation and calibration routing.
/// It is not an individual-reader key, card identity, or destructive-action
/// authorization.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReaderFamily {
    SandiskProReader,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredVolume {
    pub session_key: String,
    pub volume_key: Option<String>,
    pub display_name: String,
    pub filesystem: Option<String>,
    pub total_bytes: Option<u64>,
    pub available_bytes: Option<u64>,
    pub mount_locations: Vec<String>,
    /// App-owned mutable continuity marker. Never a hardware identity.
    pub marker_token: Option<String>,
    pub identity_candidates: Vec<IdentityCandidate>,
    pub reader_topology: Option<ReaderTopology>,
    pub state: DiscoveryState,
    pub error_code: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReaderTopology {
    /// Manufacturer/product/serial describe the reader transport, not a card.
    pub vendor: Option<String>,
    pub product: Option<String>,
    pub reader_serial: Option<String>,
    pub physical_device_number: Option<u32>,
    pub logical_unit: Option<u8>,
    /// VPD page 0x83 values reported for the current logical unit. These are
    /// retained for diagnosis only: a reader can synthesize them, so they are
    /// never promoted to an immutable-medium identity without hardware proof.
    pub reported_vpd_identifiers: Vec<String>,
    /// A CID read through Windows' native SD stack (CMD2). This is available
    /// only for direct SD host-controller stacks; USB mass-storage readers
    /// generally cannot forward the command. Raw bytes never cross IPC.
    pub reported_sd_cid: Option<String>,
}

impl ReaderTopology {
    pub fn recognized_family(&self) -> Option<ReaderFamily> {
        let vendor = self.vendor.as_deref()?.trim();
        let product = self.product.as_deref()?.trim();
        (vendor.eq_ignore_ascii_case("sandisk") && product.eq_ignore_ascii_case("pro-reader"))
            .then_some(ReaderFamily::SandiskProReader)
    }
}

pub trait DeviceDiscovery: Send + Sync {
    fn enumerate_removable_volumes(&self) -> Vec<DiscoveredVolume>;
}

struct PlatformIdentitySources<'a> {
    filesystem: &'a str,
    session: &'a str,
}

/// Event-driven Windows storage-interface subscription. The callback does no
/// discovery or I/O: it only coalesces arrival/removal into one bounded wakeup
/// for a normal worker to reconcile against a fresh snapshot.
#[cfg(windows)]
pub struct DeviceChangeSubscription {
    notification: windows::Win32::Devices::DeviceAndDriverInstallation::HCMNOTIFICATION,
    callback_context: Box<SyncSender<()>>,
    events: Receiver<()>,
}

#[cfg(windows)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeviceChangeRegistrationError {
    ConfigurationManagerRejected,
}

#[cfg(windows)]
impl DeviceChangeSubscription {
    pub fn register_disk_interfaces() -> Result<Self, DeviceChangeRegistrationError> {
        use std::sync::mpsc::sync_channel;
        use windows::Win32::Devices::DeviceAndDriverInstallation::{
            CM_Register_Notification, CM_NOTIFY_FILTER, CM_NOTIFY_FILTER_0, CM_NOTIFY_FILTER_0_0,
            CM_NOTIFY_FILTER_TYPE_DEVICEINTERFACE, CR_SUCCESS, HCMNOTIFICATION,
        };
        use windows::Win32::System::Ioctl::GUID_DEVINTERFACE_DISK;

        let (sender, events) = sync_channel(1);
        let callback_context = Box::new(sender);
        let filter = CM_NOTIFY_FILTER {
            cbSize: std::mem::size_of::<CM_NOTIFY_FILTER>() as u32,
            Flags: 0,
            FilterType: CM_NOTIFY_FILTER_TYPE_DEVICEINTERFACE,
            Reserved: 0,
            u: CM_NOTIFY_FILTER_0 {
                DeviceInterface: CM_NOTIFY_FILTER_0_0 {
                    ClassGuid: GUID_DEVINTERFACE_DISK,
                },
            },
        };
        let mut notification = HCMNOTIFICATION::default();
        let result = unsafe {
            CM_Register_Notification(
                &filter,
                Some((&*callback_context as *const SyncSender<()>).cast()),
                Some(device_interface_notification),
                &mut notification,
            )
        };
        if result != CR_SUCCESS || notification.is_invalid() {
            return Err(DeviceChangeRegistrationError::ConfigurationManagerRejected);
        }
        Ok(Self {
            notification,
            callback_context,
            events,
        })
    }

    pub fn recv(&self) -> Result<(), std::sync::mpsc::RecvError> {
        self.events.recv()
    }
}

#[cfg(windows)]
impl Drop for DeviceChangeSubscription {
    fn drop(&mut self) {
        use windows::Win32::Devices::DeviceAndDriverInstallation::CM_Unregister_Notification;
        let _ = unsafe { CM_Unregister_Notification(self.notification) };
        // Configuration Manager has stopped delivering to the callback context
        // before this boxed sender is dropped.
        let _ = &self.callback_context;
    }
}

#[cfg(windows)]
unsafe extern "system" fn device_interface_notification(
    _notification: windows::Win32::Devices::DeviceAndDriverInstallation::HCMNOTIFICATION,
    context: *const core::ffi::c_void,
    action: windows::Win32::Devices::DeviceAndDriverInstallation::CM_NOTIFY_ACTION,
    _event_data: *const windows::Win32::Devices::DeviceAndDriverInstallation::CM_NOTIFY_EVENT_DATA,
    _event_data_size: u32,
) -> u32 {
    use windows::Win32::Devices::DeviceAndDriverInstallation::{
        CM_NOTIFY_ACTION_DEVICEINTERFACEARRIVAL, CM_NOTIFY_ACTION_DEVICEINTERFACEREMOVAL,
    };
    if context.is_null()
        || (action != CM_NOTIFY_ACTION_DEVICEINTERFACEARRIVAL
            && action != CM_NOTIFY_ACTION_DEVICEINTERFACEREMOVAL)
    {
        return 0;
    }
    // SAFETY: register_disk_interfaces retains the boxed SyncSender until after
    // Configuration Manager unregisters this callback.
    let sender = unsafe { &*context.cast::<SyncSender<()>>() };
    let _ = sender.try_send(());
    0
}

#[derive(Default)]
pub struct NativeDeviceDiscovery;

impl DeviceDiscovery for NativeDeviceDiscovery {
    fn enumerate_removable_volumes(&self) -> Vec<DiscoveredVolume> {
        platform::enumerate_removable_volumes()
    }
}

pub fn volume_from_parts(
    mount_location: String,
    label: String,
    filesystem: Option<String>,
    serial: Option<String>,
    total_bytes: Option<u64>,
    available_bytes: Option<u64>,
) -> DiscoveredVolume {
    volume_from_platform_parts(
        PlatformIdentitySources {
            filesystem: "windows.volume-serial",
            session: "windows.mount-location",
        },
        mount_location,
        label,
        filesystem,
        serial,
        total_bytes,
        available_bytes,
    )
}

fn volume_from_platform_parts(
    identity_sources: PlatformIdentitySources<'_>,
    mount_location: String,
    label: String,
    filesystem: Option<String>,
    serial: Option<String>,
    total_bytes: Option<u64>,
    available_bytes: Option<u64>,
) -> DiscoveredVolume {
    let mut identity_candidates = Vec::new();
    if let Some(serial) = serial {
        if let Some(candidate) = IdentityCandidate::new(
            identity_sources.filesystem,
            IdentityScope::Filesystem,
            serial,
            IdentityStrength::Filesystem,
        ) {
            identity_candidates.push(candidate);
        }
    }
    let session_candidate = IdentityCandidate::new(
        identity_sources.session,
        IdentityScope::Session,
        mount_location.clone(),
        IdentityStrength::Session,
    )
    .expect("a native mount location is non-empty");
    let session_key = derive_key(IdentityScope::Session, &session_candidate);
    let volume_key = identity_candidates
        .first()
        .map(|candidate| derive_key(IdentityScope::Filesystem, candidate));

    DiscoveredVolume {
        session_key,
        volume_key,
        display_name: if label.is_empty() {
            mount_location.clone()
        } else {
            label
        },
        filesystem,
        total_bytes,
        available_bytes,
        mount_locations: vec![mount_location],
        marker_token: None,
        identity_candidates,
        reader_topology: None,
        state: DiscoveryState::Ready,
        error_code: None,
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, GetDiskFreeSpaceExW, GetDriveTypeW, GetLogicalDrives, GetVolumeInformationW,
        FILE_ATTRIBUTE_NORMAL, FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_SHARE_READ,
        FILE_SHARE_WRITE, OPEN_EXISTING,
    };
    use windows::Win32::Storage::IscsiDisc::{IOCTL_SCSI_GET_ADDRESS, SCSI_ADDRESS};
    use windows::Win32::System::Ioctl::{
        PropertyStandardQuery, StorageDeviceIdProperty, StorageDeviceProperty,
        IOCTL_STORAGE_GET_DEVICE_NUMBER, IOCTL_STORAGE_QUERY_PROPERTY, STORAGE_DEVICE_DESCRIPTOR,
        STORAGE_DEVICE_NUMBER, STORAGE_IDENTIFIER, STORAGE_PROPERTY_QUERY,
    };
    use windows::Win32::System::IO::DeviceIoControl;

    const DRIVE_REMOVABLE: u32 = 2;

    pub fn enumerate_removable_volumes() -> Vec<DiscoveredVolume> {
        let bitmask = unsafe { GetLogicalDrives() };
        (0..26)
            .filter(|index| bitmask & (1 << index) != 0)
            .filter_map(|index| enumerate_letter(index as u8))
            .collect()
    }

    fn enumerate_letter(index: u8) -> Option<DiscoveredVolume> {
        let mount = format!("{}:\\", char::from(b'A' + index));
        let wide = wide_nul(&mount);
        if unsafe { GetDriveTypeW(PCWSTR(wide.as_ptr())) } != DRIVE_REMOVABLE {
            return None;
        }

        let mut available = 0_u64;
        let mut total = 0_u64;
        let space_result = unsafe {
            GetDiskFreeSpaceExW(
                PCWSTR(wide.as_ptr()),
                Some(&mut available),
                Some(&mut total),
                None,
            )
        };

        let mut label = [0_u16; 256];
        let mut filesystem = [0_u16; 256];
        let mut serial = 0_u32;
        let info_result = unsafe {
            GetVolumeInformationW(
                PCWSTR(wide.as_ptr()),
                Some(&mut label),
                Some(&mut serial),
                None,
                None,
                Some(&mut filesystem),
            )
        };

        let topology = query_reader_topology(index);
        if info_result.is_err() && space_result.is_err() {
            return Some(DiscoveredVolume {
                session_key: format!("windows:unavailable:{mount}"),
                volume_key: None,
                display_name: mount.clone(),
                filesystem: None,
                total_bytes: None,
                available_bytes: None,
                mount_locations: vec![mount],
                marker_token: None,
                identity_candidates: Vec::new(),
                reader_topology: topology,
                state: DiscoveryState::Unavailable,
                error_code: Some("WINDOWS_VOLUME_QUERY_FAILED".into()),
            });
        }

        let mut volume = volume_from_parts(
            mount.clone(),
            from_wide(&label),
            info_result.as_ref().ok().map(|_| from_wide(&filesystem)),
            info_result.as_ref().ok().map(|_| format!("{serial:08X}")),
            space_result.as_ref().ok().map(|_| total),
            space_result.as_ref().ok().map(|_| available),
        );
        volume.marker_token = crate::storage_marker::read_marker(std::path::Path::new(&mount))
            .ok()
            .flatten();
        volume.reader_topology = topology;
        Some(volume)
    }

    fn query_reader_topology(index: u8) -> Option<ReaderTopology> {
        let device_path = format!(r"\\.\{}:", char::from(b'A' + index));
        let wide = wide_nul(&device_path);
        let handle = unsafe {
            CreateFileW(
                PCWSTR(wide.as_ptr()),
                FILE_GENERIC_READ.0 | FILE_GENERIC_WRITE.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                None,
            )
        }
        .or_else(|_| unsafe {
            // A locked SD card may reject write access even though the
            // read-only descriptor queries remain useful. CID probing will
            // then conservatively be unavailable.
            CreateFileW(
                PCWSTR(wide.as_ptr()),
                FILE_GENERIC_READ.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                None,
            )
        })
        .ok()?;
        let topology = query_handle_topology(handle);
        let _ = unsafe { CloseHandle(handle) };
        topology
    }

    fn query_handle_topology(handle: windows::Win32::Foundation::HANDLE) -> Option<ReaderTopology> {
        let mut query = STORAGE_PROPERTY_QUERY {
            PropertyId: StorageDeviceProperty,
            QueryType: PropertyStandardQuery,
            AdditionalParameters: [0],
        };
        let mut descriptor_bytes = vec![0_u8; 1024];
        let mut bytes_returned = 0_u32;
        unsafe {
            DeviceIoControl(
                handle,
                IOCTL_STORAGE_QUERY_PROPERTY,
                Some((&mut query as *mut STORAGE_PROPERTY_QUERY).cast()),
                std::mem::size_of::<STORAGE_PROPERTY_QUERY>() as u32,
                Some(descriptor_bytes.as_mut_ptr().cast()),
                descriptor_bytes.len() as u32,
                Some(&mut bytes_returned),
                None,
            )
        }
        .ok()?;
        if bytes_returned < std::mem::size_of::<STORAGE_DEVICE_DESCRIPTOR>() as u32 {
            return None;
        }
        let descriptor = unsafe {
            std::ptr::read_unaligned(
                descriptor_bytes
                    .as_ptr()
                    .cast::<STORAGE_DEVICE_DESCRIPTOR>(),
            )
        };
        let mut device_number = STORAGE_DEVICE_NUMBER::default();
        let physical_device_number = unsafe {
            DeviceIoControl(
                handle,
                IOCTL_STORAGE_GET_DEVICE_NUMBER,
                None,
                0,
                Some((&mut device_number as *mut STORAGE_DEVICE_NUMBER).cast()),
                std::mem::size_of::<STORAGE_DEVICE_NUMBER>() as u32,
                Some(&mut bytes_returned),
                None,
            )
        }
        .ok()
        .map(|_| device_number.DeviceNumber);
        let mut address = SCSI_ADDRESS::default();
        let logical_unit = unsafe {
            DeviceIoControl(
                handle,
                IOCTL_SCSI_GET_ADDRESS,
                None,
                0,
                Some((&mut address as *mut SCSI_ADDRESS).cast()),
                std::mem::size_of::<SCSI_ADDRESS>() as u32,
                Some(&mut bytes_returned),
                None,
            )
        }
        .ok()
        .map(|_| address.Lun);
        let reported_vpd_identifiers = query_vpd_identifiers(handle);
        let reported_sd_cid = query_direct_sd_cid(handle);
        Some(ReaderTopology {
            vendor: c_string_at(&descriptor_bytes, descriptor.VendorIdOffset),
            product: c_string_at(&descriptor_bytes, descriptor.ProductIdOffset),
            reader_serial: c_string_at(&descriptor_bytes, descriptor.SerialNumberOffset),
            physical_device_number,
            logical_unit,
            reported_vpd_identifiers,
            reported_sd_cid,
        })
    }

    /// Read CMD2 (ALL_SEND_CID) only through Windows' native SD stack. The
    /// buffer layout is the documented SFFDISK header followed by the
    /// `SDCMD_DESCRIPTOR` and its 16-byte response area. Unsupported reader
    /// transports simply return no CID; no fallback identifier is invented.
    fn query_direct_sd_cid(handle: windows::Win32::Foundation::HANDLE) -> Option<String> {
        const IOCTL_SFFDISK_DEVICE_COMMAND: u32 = 0x79E84;
        const SFFDISK_DC_DEVICE_COMMAND: u32 = 3;
        const CMD2_ALL_SEND_CID: u8 = 2;
        const SDTD_READ: u32 = 1;
        const SDTT_CMD_ONLY: u32 = 1;
        const SDRT_2: u32 = 4;
        const DESCRIPTOR_BYTES: usize = 20;
        const CID_BYTES: usize = 16;
        // Header fields before ULONG_PTR occupy 16 bytes on both supported
        // Windows ABIs; the trailing pointer-sized Information field follows.
        let header_bytes = 16 + std::mem::size_of::<usize>();
        let descriptor_offset = header_bytes;
        let response_offset = descriptor_offset + DESCRIPTOR_BYTES;
        let mut command = vec![0_u8; response_offset + CID_BYTES];
        command[0..2].copy_from_slice(&(header_bytes as u16).to_le_bytes());
        command[4..8].copy_from_slice(&SFFDISK_DC_DEVICE_COMMAND.to_le_bytes());
        command[8..10].copy_from_slice(&(DESCRIPTOR_BYTES as u16).to_le_bytes());
        command[12..16].copy_from_slice(&(CID_BYTES as u32).to_le_bytes());
        command[descriptor_offset] = CMD2_ALL_SEND_CID;
        command[descriptor_offset + 4..descriptor_offset + 8].copy_from_slice(&0_u32.to_le_bytes()); // SDCC_STANDARD
        command[descriptor_offset + 8..descriptor_offset + 12]
            .copy_from_slice(&SDTD_READ.to_le_bytes());
        command[descriptor_offset + 12..descriptor_offset + 16]
            .copy_from_slice(&SDTT_CMD_ONLY.to_le_bytes());
        command[descriptor_offset + 16..descriptor_offset + 20]
            .copy_from_slice(&SDRT_2.to_le_bytes());
        let mut bytes_returned = 0_u32;
        unsafe {
            DeviceIoControl(
                handle,
                IOCTL_SFFDISK_DEVICE_COMMAND,
                Some(command.as_mut_ptr().cast()),
                command.len() as u32,
                Some(command.as_mut_ptr().cast()),
                command.len() as u32,
                Some(&mut bytes_returned),
                None,
            )
        }
        .ok()?;
        (bytes_returned as usize >= response_offset + CID_BYTES)
            .then(|| parse_sd_cid(&command[response_offset..response_offset + CID_BYTES]))?
    }

    pub(super) fn parse_sd_cid(value: &[u8]) -> Option<String> {
        (value.len() == 16 && value.iter().any(|byte| *byte != 0)).then(|| hex::encode(value))
    }

    fn query_vpd_identifiers(handle: windows::Win32::Foundation::HANDLE) -> Vec<String> {
        let mut query = STORAGE_PROPERTY_QUERY {
            PropertyId: StorageDeviceIdProperty,
            QueryType: PropertyStandardQuery,
            AdditionalParameters: [0],
        };
        let mut header = [0_u8; 8];
        let mut bytes_returned = 0_u32;
        if unsafe {
            DeviceIoControl(
                handle,
                IOCTL_STORAGE_QUERY_PROPERTY,
                Some((&mut query as *mut STORAGE_PROPERTY_QUERY).cast()),
                std::mem::size_of::<STORAGE_PROPERTY_QUERY>() as u32,
                Some(header.as_mut_ptr().cast()),
                header.len() as u32,
                Some(&mut bytes_returned),
                None,
            )
        }
        .is_err()
            || bytes_returned < header.len() as u32
        {
            return Vec::new();
        }
        let declared_size = u32::from_le_bytes(header[4..8].try_into().expect("header length"));
        let size = usize::try_from(declared_size)
            .ok()
            .filter(|size| *size >= 12 && *size <= 64 * 1024)
            .unwrap_or_default();
        if size == 0 {
            return Vec::new();
        }
        let mut descriptor = vec![0_u8; size];
        if unsafe {
            DeviceIoControl(
                handle,
                IOCTL_STORAGE_QUERY_PROPERTY,
                Some((&mut query as *mut STORAGE_PROPERTY_QUERY).cast()),
                std::mem::size_of::<STORAGE_PROPERTY_QUERY>() as u32,
                Some(descriptor.as_mut_ptr().cast()),
                descriptor.len() as u32,
                Some(&mut bytes_returned),
                None,
            )
        }
        .is_err()
        {
            return Vec::new();
        }
        descriptor.truncate(bytes_returned as usize);
        parse_vpd_identifiers(&descriptor)
    }

    pub(super) fn parse_vpd_identifiers(descriptor: &[u8]) -> Vec<String> {
        const DESCRIPTOR_HEADER_BYTES: usize = 12;
        const IDENTIFIER_HEADER_BYTES: usize = std::mem::offset_of!(STORAGE_IDENTIFIER, Identifier);
        if descriptor.len() < DESCRIPTOR_HEADER_BYTES {
            return Vec::new();
        }
        let declared_size = u32::from_le_bytes(
            descriptor[4..8]
                .try_into()
                .expect("descriptor header length"),
        ) as usize;
        let bounded = declared_size.min(descriptor.len());
        let identifier_count = u32::from_le_bytes(
            descriptor[8..12]
                .try_into()
                .expect("descriptor header length"),
        );
        let mut offset = DESCRIPTOR_HEADER_BYTES;
        let mut identifiers = Vec::new();
        for _ in 0..identifier_count {
            let Some(header_end) = offset.checked_add(IDENTIFIER_HEADER_BYTES) else {
                break;
            };
            if header_end > bounded {
                break;
            }
            let code_set = i32::from_le_bytes(
                descriptor[offset..offset + 4]
                    .try_into()
                    .expect("identifier code set length"),
            );
            let identifier_type = i32::from_le_bytes(
                descriptor[offset + 4..offset + 8]
                    .try_into()
                    .expect("identifier type length"),
            );
            let identifier_size = u16::from_le_bytes(
                descriptor[offset + 8..offset + 10]
                    .try_into()
                    .expect("identifier size length"),
            ) as usize;
            let next_offset = u16::from_le_bytes(
                descriptor[offset + 10..offset + 12]
                    .try_into()
                    .expect("next offset length"),
            ) as usize;
            let association = i32::from_le_bytes(
                descriptor[offset + 12..offset + 16]
                    .try_into()
                    .expect("association length"),
            );
            let Some(identifier_end) = header_end.checked_add(identifier_size) else {
                break;
            };
            if identifier_end > bounded {
                break;
            }
            identifiers.push(format!(
                "vpd83:{code_set}:{identifier_type}:{association}:{}",
                hex::encode(&descriptor[header_end..identifier_end])
            ));
            if next_offset == 0 {
                break;
            }
            let Some(next) = offset.checked_add(next_offset) else {
                break;
            };
            if next <= offset || next > bounded {
                break;
            }
            offset = next;
        }
        identifiers
    }

    fn c_string_at(bytes: &[u8], offset: u32) -> Option<String> {
        let start = usize::try_from(offset).ok()?;
        let value = bytes.get(start..)?.split(|byte| *byte == 0).next()?;
        let value = String::from_utf8_lossy(value).trim().to_string();
        (!value.is_empty()).then_some(value)
    }

    fn wide_nul(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn from_wide(value: &[u16]) -> String {
        let length = value
            .iter()
            .position(|character| *character == 0)
            .unwrap_or(value.len());
        String::from_utf16_lossy(&value[..length])
    }
}

#[cfg(any(target_os = "linux", test))]
fn parse_linux_mountinfo_line(line: &str) -> Option<(String, String, String)> {
    let (before_separator, after_separator) = line.split_once(" - ")?;
    let mount_location = unescape_mountinfo(before_separator.split_whitespace().nth(4)?);
    let mut filesystem_fields = after_separator.split_whitespace();
    let filesystem = filesystem_fields.next()?.to_string();
    let source = filesystem_fields.next()?.to_string();
    Some((mount_location, filesystem, source))
}

#[cfg(any(target_os = "linux", test))]
fn unescape_mountinfo(value: &str) -> String {
    value
        .replace("\\040", " ")
        .replace("\\011", "\t")
        .replace("\\012", "\n")
        .replace("\\134", "\\")
}

#[cfg(target_os = "linux")]
mod platform {
    use super::*;
    use fs2::{available_space, total_space};
    use std::collections::HashSet;
    use std::path::Path;

    pub fn enumerate_removable_volumes() -> Vec<DiscoveredVolume> {
        let mounts = match std::fs::read_to_string("/proc/self/mountinfo") {
            Ok(mounts) => mounts,
            Err(_) => return Vec::new(),
        };
        let mut seen_mounts = HashSet::new();
        mounts
            .lines()
            .filter_map(parse_linux_mountinfo_line)
            .filter(|(mount, _filesystem, source)| {
                source.starts_with("/dev/") && seen_mounts.insert(mount.clone())
            })
            .filter(|(_mount, _filesystem, source)| removable_block_source(source))
            .map(|(mount, filesystem, source)| {
                let mount_path = Path::new(&mount);
                let mut volume = volume_from_platform_parts(
                    PlatformIdentitySources {
                        filesystem: "linux.filesystem-uuid-unavailable",
                        session: "linux.mount-location",
                    },
                    mount.clone(),
                    source.rsplit('/').next().unwrap_or(&source).to_string(),
                    Some(filesystem),
                    None,
                    total_space(mount_path).ok(),
                    available_space(mount_path).ok(),
                );
                volume.marker_token = crate::storage_marker::read_marker(mount_path)
                    .ok()
                    .flatten();
                volume
            })
            .collect()
    }

    fn removable_block_source(source: &str) -> bool {
        let Some(name) = Path::new(source)
            .file_name()
            .and_then(|value| value.to_str())
        else {
            return false;
        };
        let mut sysfs_path = match std::fs::canonicalize(format!("/sys/class/block/{name}")) {
            Ok(path) => path,
            Err(_) => return false,
        };
        loop {
            let removable = sysfs_path.join("removable");
            if let Ok(value) = std::fs::read_to_string(&removable) {
                return value.trim() == "1";
            }
            let Some(parent) = sysfs_path.parent() else {
                return false;
            };
            if parent == sysfs_path {
                return false;
            }
            sysfs_path = parent.to_path_buf();
        }
    }
}

#[cfg(all(not(windows), not(target_os = "linux")))]
mod platform {
    use super::*;

    pub fn enumerate_removable_volumes() -> Vec<DiscoveredVolume> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_serial_is_filesystem_evidence_not_medium_identity() {
        let volume = volume_from_parts(
            "F:\\".into(),
            "CARD".into(),
            Some("exFAT".into()),
            Some("ABCD-1234".into()),
            Some(32),
            Some(12),
        );
        assert!(volume.volume_key.is_some());
        assert_eq!(
            volume.identity_candidates[0].scope,
            IdentityScope::Filesystem
        );
        assert_eq!(
            volume.identity_candidates[0].strength,
            IdentityStrength::Filesystem
        );
    }

    #[test]
    fn linux_mountinfo_parser_preserves_escaped_mounts_without_cli_parsing() {
        let parsed = parse_linux_mountinfo_line(
            "42 29 8:1 / /media/Camera\\040Card rw,nosuid - exfat /dev/sdb1 rw",
        )
        .expect("mount line");
        assert_eq!(parsed.0, "/media/Camera Card");
        assert_eq!(parsed.1, "exfat");
        assert_eq!(parsed.2, "/dev/sdb1");
    }

    #[test]
    fn pro_reader_recognition_is_exact_and_non_authoritative() {
        let topology = ReaderTopology {
            vendor: Some("SanDisk".into()),
            product: Some("PRO-READER".into()),
            reader_serial: Some("revision-like-value".into()),
            physical_device_number: Some(4),
            logical_unit: Some(0),
            reported_vpd_identifiers: Vec::new(),
            reported_sd_cid: None,
        };
        assert_eq!(
            topology.recognized_family(),
            Some(ReaderFamily::SandiskProReader)
        );
        assert_eq!(
            ReaderTopology {
                product: Some("PRO-READER MULTI-CARD".into()),
                ..topology
            }
            .recognized_family(),
            None
        );
    }

    #[cfg(windows)]
    #[test]
    fn vpd_parser_binds_only_well_formed_identifier_bytes() {
        let mut descriptor = vec![0_u8; 35];
        let descriptor_size = descriptor.len() as u32;
        descriptor[4..8].copy_from_slice(&descriptor_size.to_le_bytes());
        descriptor[8..12].copy_from_slice(&1_u32.to_le_bytes());
        // One 16-byte STORAGE_IDENTIFIER header followed by a seven-byte value.
        descriptor[12..16].copy_from_slice(&1_i32.to_le_bytes());
        descriptor[16..20].copy_from_slice(&2_i32.to_le_bytes());
        descriptor[20..22].copy_from_slice(&7_u16.to_le_bytes());
        descriptor[22..24].copy_from_slice(&0_u16.to_le_bytes());
        descriptor[24..28].copy_from_slice(&0_i32.to_le_bytes());
        descriptor[28..35].copy_from_slice(b"CARD-ID");
        assert_eq!(
            platform::parse_vpd_identifiers(&descriptor),
            vec!["vpd83:1:2:0:434152442d4944"]
        );
    }

    #[cfg(windows)]
    #[test]
    fn direct_sd_cid_parser_requires_exact_nonzero_register_bytes() {
        assert_eq!(
            platform::parse_sd_cid(&[0x03; 16]),
            Some("03030303030303030303030303030303".into())
        );
        assert_eq!(platform::parse_sd_cid(&[0; 16]), None);
        assert_eq!(platform::parse_sd_cid(&[0x03; 15]), None);
    }
}
