//! Native safe-eject boundary.
//!
//! A caller may provide only a mount root that was just revalidated by the
//! Rust-owned device snapshot. This module never accepts a shell command,
//! drive number, or reader name from the webview.

use std::path::Path;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SafeEjectError {
    UnsupportedPlatform,
    /// Windows rejected removal because something still owns the medium.
    DeviceBusy(Option<String>),
    DeviceNotEjectable,
    EjectFailed,
}

/// Requests an OS-confirmed safe removal after the caller has closed its own
/// handles and revalidated the selected medium. On Windows this targets the
/// exact disk device behind the selected volume, never a USB-reader parent;
/// a multi-slot reader can therefore not be removed accidentally.
pub fn safe_eject(mount_root: &Path) -> Result<(), SafeEjectError> {
    platform::safe_eject(mount_root)
}

#[cfg(windows)]
mod platform {
    use super::SafeEjectError;
    use std::{mem::size_of, path::Path, slice};
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, FlushFileBuffers, FILE_ATTRIBUTE_NORMAL, FILE_GENERIC_READ,
        FILE_GENERIC_WRITE, FILE_SHARE_MODE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };
    use windows::Win32::System::Ioctl::{
        FSCTL_DISMOUNT_VOLUME, FSCTL_LOCK_VOLUME, FSCTL_UNLOCK_VOLUME, GUID_DEVINTERFACE_DISK,
        IOCTL_STORAGE_GET_DEVICE_NUMBER, STORAGE_DEVICE_NUMBER,
    };
    use windows::Win32::System::IO::DeviceIoControl;
    use windows::Win32::{
        Devices::DeviceAndDriverInstallation::{
            CM_Request_Device_EjectW, SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInterfaces,
            SetupDiGetClassDevsW, SetupDiGetDeviceInterfaceDetailW, CONFIGRET, CR_REMOVE_VETOED,
            CR_SUCCESS, DIGCF_DEVICEINTERFACE, DIGCF_PRESENT, SP_DEVICE_INTERFACE_DATA,
            SP_DEVICE_INTERFACE_DETAIL_DATA_W, SP_DEVINFO_DATA,
        },
        Foundation::CloseHandle,
    };

    pub(super) fn safe_eject(mount_root: &Path) -> Result<(), SafeEjectError> {
        let device_path = volume_device_path(mount_root).ok_or(SafeEjectError::EjectFailed)?;
        let wide = wide_nul(&device_path);
        // Exclusive access intentionally fails when another process still has
        // a file open. That is the only safe outcome for an operator-facing
        // eject control.
        let handle = unsafe {
            CreateFileW(
                PCWSTR(wide.as_ptr()),
                FILE_GENERIC_READ.0 | FILE_GENERIC_WRITE.0,
                FILE_SHARE_MODE(0),
                None,
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                None,
            )
        }
        .map_err(|_| SafeEjectError::DeviceBusy(None))?;
        let disk_devinst = (|| unsafe {
            FlushFileBuffers(handle).map_err(|_| SafeEjectError::EjectFailed)?;
            DeviceIoControl(handle, FSCTL_LOCK_VOLUME, None, 0, None, 0, None, None)
                .map_err(|_| SafeEjectError::DeviceBusy(None))?;
            if DeviceIoControl(handle, FSCTL_DISMOUNT_VOLUME, None, 0, None, 0, None, None).is_err()
            {
                let _ = DeviceIoControl(handle, FSCTL_UNLOCK_VOLUME, None, 0, None, 0, None, None);
                return Err(SafeEjectError::EjectFailed);
            }
            let device_number = storage_device_number(handle)?;
            disk_devinst_for_number(&device_number).ok_or(SafeEjectError::DeviceNotEjectable)
        })();
        // Closing our own exclusive volume handle is required before asking
        // Plug and Play to remove the matched disk. Retaining it would let
        // this process veto its own `CM_Request_Device_EjectW` request.
        let _ = unsafe { CloseHandle(handle) };
        disk_devinst.and_then(request_safe_removal)
    }

    unsafe fn storage_device_number(
        handle: windows::Win32::Foundation::HANDLE,
    ) -> Result<STORAGE_DEVICE_NUMBER, SafeEjectError> {
        let mut device_number = STORAGE_DEVICE_NUMBER::default();
        DeviceIoControl(
            handle,
            IOCTL_STORAGE_GET_DEVICE_NUMBER,
            None,
            0,
            Some((&mut device_number as *mut STORAGE_DEVICE_NUMBER).cast()),
            size_of::<STORAGE_DEVICE_NUMBER>() as u32,
            None,
            None,
        )
        .map_err(|_| SafeEjectError::EjectFailed)?;
        Ok(device_number)
    }

    fn disk_devinst_for_number(target: &STORAGE_DEVICE_NUMBER) -> Option<u32> {
        let device_set = unsafe {
            SetupDiGetClassDevsW(
                Some(&GUID_DEVINTERFACE_DISK),
                PCWSTR::null(),
                None,
                DIGCF_PRESENT | DIGCF_DEVICEINTERFACE,
            )
        }
        .ok()?;
        let result = (|| {
            for index in 0.. {
                let mut interface = SP_DEVICE_INTERFACE_DATA {
                    cbSize: size_of::<SP_DEVICE_INTERFACE_DATA>() as u32,
                    ..Default::default()
                };
                if unsafe {
                    SetupDiEnumDeviceInterfaces(
                        device_set,
                        None,
                        &GUID_DEVINTERFACE_DISK,
                        index,
                        &mut interface,
                    )
                }
                .is_err()
                {
                    break;
                }
                let mut detail_size = 0_u32;
                let _ = unsafe {
                    SetupDiGetDeviceInterfaceDetailW(
                        device_set,
                        &interface,
                        None,
                        0,
                        Some(&mut detail_size),
                        None,
                    )
                };
                if detail_size < size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32 {
                    continue;
                }
                let mut detail_words =
                    vec![0_usize; (detail_size as usize).div_ceil(size_of::<usize>())];
                let detail = detail_words
                    .as_mut_ptr()
                    .cast::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>();
                unsafe { (*detail).cbSize = size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32 };
                let mut info = SP_DEVINFO_DATA {
                    cbSize: size_of::<SP_DEVINFO_DATA>() as u32,
                    ..Default::default()
                };
                if unsafe {
                    SetupDiGetDeviceInterfaceDetailW(
                        device_set,
                        &interface,
                        Some(detail),
                        detail_size,
                        None,
                        Some(&mut info),
                    )
                }
                .is_err()
                {
                    continue;
                }
                let Some(path) = (unsafe { interface_path(detail, detail_size as usize) }) else {
                    continue;
                };
                let Ok(handle) = open_readonly_device(&path) else {
                    continue;
                };
                let observed = unsafe { storage_device_number(handle) };
                let _ = unsafe { CloseHandle(handle) };
                if matches!(observed, Ok(number) if same_device_number(target, &number)) {
                    return Some(info.DevInst);
                }
            }
            None
        })();
        let _ = unsafe { SetupDiDestroyDeviceInfoList(device_set) };
        result
    }

    fn open_readonly_device(
        path: &str,
    ) -> Result<windows::Win32::Foundation::HANDLE, SafeEjectError> {
        let wide = wide_nul(path);
        unsafe {
            CreateFileW(
                PCWSTR(wide.as_ptr()),
                FILE_GENERIC_READ.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                None,
            )
        }
        .map_err(|_| SafeEjectError::EjectFailed)
    }

    unsafe fn interface_path(
        detail: *const SP_DEVICE_INTERFACE_DETAIL_DATA_W,
        detail_size: usize,
    ) -> Option<String> {
        let offset = std::mem::offset_of!(SP_DEVICE_INTERFACE_DETAIL_DATA_W, DevicePath);
        let length = detail_size.checked_sub(offset)? / size_of::<u16>();
        let units = slice::from_raw_parts((*detail).DevicePath.as_ptr(), length);
        let end = units.iter().position(|unit| *unit == 0)?;
        String::from_utf16(&units[..end]).ok()
    }

    fn same_device_number(left: &STORAGE_DEVICE_NUMBER, right: &STORAGE_DEVICE_NUMBER) -> bool {
        left.DeviceType == right.DeviceType && left.DeviceNumber == right.DeviceNumber
    }

    fn request_safe_removal(devinst: u32) -> Result<(), SafeEjectError> {
        let mut veto_name = [0_u16; 260];
        let mut veto_type = Default::default();
        let result: CONFIGRET = unsafe {
            CM_Request_Device_EjectW(devinst, Some(&mut veto_type), Some(&mut veto_name), 0)
        };
        if result == CR_SUCCESS {
            Ok(())
        } else if result == CR_REMOVE_VETOED {
            Err(SafeEjectError::DeviceBusy(read_veto_name(&veto_name)))
        } else {
            Err(SafeEjectError::DeviceNotEjectable)
        }
    }

    fn read_veto_name(buffer: &[u16]) -> Option<String> {
        let end = buffer.iter().position(|unit| *unit == 0)?;
        let value = String::from_utf16_lossy(&buffer[..end]);
        (!value.trim().is_empty()).then_some(value)
    }

    fn volume_device_path(mount_root: &Path) -> Option<String> {
        let value = mount_root.to_string_lossy();
        let bytes = value.as_bytes();
        (bytes.len() == 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && (bytes[2] == b'\\' || bytes[2] == b'/'))
            .then(|| format!(r"\\.\{}:", char::from(bytes[0].to_ascii_uppercase())))
    }

    fn wide_nul(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    #[cfg(test)]
    mod tests {
        use super::{same_device_number, volume_device_path};
        use std::path::Path;
        use windows::Win32::System::Ioctl::STORAGE_DEVICE_NUMBER;

        #[test]
        fn accepts_only_a_windows_drive_root() {
            assert_eq!(
                volume_device_path(Path::new("f:\\")),
                Some(r"\\.\F:".into())
            );
            assert_eq!(volume_device_path(Path::new("F:\\DCIM")), None);
            assert_eq!(volume_device_path(Path::new("\\\\server\\share")), None);
        }

        #[test]
        fn disk_lookup_never_accepts_a_different_disk_number() {
            let target = STORAGE_DEVICE_NUMBER {
                DeviceType: 7,
                DeviceNumber: 4,
                PartitionNumber: 1,
            };
            assert!(same_device_number(
                &target,
                &STORAGE_DEVICE_NUMBER {
                    DeviceType: 7,
                    DeviceNumber: 4,
                    PartitionNumber: 0,
                }
            ));
            assert!(!same_device_number(
                &target,
                &STORAGE_DEVICE_NUMBER {
                    DeviceType: 7,
                    DeviceNumber: 5,
                    PartitionNumber: 1,
                }
            ));
        }
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::SafeEjectError;
    use std::path::Path;
    use std::process::Command;

    const DISKUTIL: &str = "/usr/sbin/diskutil";

    pub(super) fn safe_eject(mount_root: &Path) -> Result<(), SafeEjectError> {
        let canonical = mount_root
            .canonicalize()
            .map_err(|_| SafeEjectError::EjectFailed)?;
        if !canonical.is_dir() || !canonical.starts_with("/Volumes/") {
            return Err(SafeEjectError::DeviceNotEjectable);
        }
        let output = Command::new(DISKUTIL)
            .args(["eject", "-plist"])
            .arg(&canonical)
            .output()
            .map_err(|_| SafeEjectError::EjectFailed)?;
        if output.status.success() {
            Ok(())
        } else {
            Err(SafeEjectError::DeviceBusy(None))
        }
    }
}

#[cfg(all(not(windows), not(target_os = "macos")))]
mod platform {
    use super::SafeEjectError;
    use std::path::Path;

    pub(super) fn safe_eject(_mount_root: &Path) -> Result<(), SafeEjectError> {
        Err(SafeEjectError::UnsupportedPlatform)
    }
}
