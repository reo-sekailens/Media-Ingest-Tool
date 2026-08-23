//! Windows Storage Management provider.
//!
//! The provider talks directly to `root\\Microsoft\\Windows\\Storage` through
//! WMI COM. It never starts PowerShell, `format.exe`, or a shell command. The
//! mount root is a native discovery locator only; WMI returns an opaque object
//! path that is re-opened immediately before `MSFT_Volume.Format` executes.

use super::{
    ExpectedFormatTarget, FormatProviderError, PlatformFormatProvider, ResolvedFormatTarget,
    ValidatedMount,
};
use crate::format_profiles::{FormatFilesystem, FormatProfile};
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};
use windows::core::{BSTR, GUID, PCWSTR};
use windows::Win32::Foundation::{RPC_E_TOO_LATE, VARIANT_FALSE, VARIANT_TRUE};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoInitializeSecurity, CoUninitialize, CLSCTX_INPROC_SERVER,
    COINIT_MULTITHREADED, EOAC_NONE, RPC_C_AUTHN_LEVEL_CALL, RPC_C_IMP_LEVEL_IMPERSONATE,
};
use windows::Win32::System::Rpc::{RPC_C_AUTHN_WINNT, RPC_C_AUTHZ_NONE};
use windows::Win32::System::Variant::{
    VariantClear, VARENUM, VARIANT, VARIANT_0_0, VT_BOOL, VT_BSTR, VT_UI4, VT_UI8,
};
use windows::Win32::System::Wmi::{
    IWbemClassObject, IWbemLocator, IWbemServices, WBEM_FLAG_FORWARD_ONLY,
    WBEM_FLAG_RETURN_IMMEDIATELY, WBEM_INFINITE,
};

const STORAGE_NAMESPACE: &str = r"ROOT\Microsoft\Windows\Storage";
const WQL: &str = "WQL";
const VOLUME_CLASS: &str = "MSFT_Volume";
const FORMAT_METHOD: &str = "Format";
const FORMAT_TIMEOUT: Duration = Duration::from_secs(45);

pub(super) struct WindowsStorageProvider;

impl PlatformFormatProvider for WindowsStorageProvider {
    fn resolve_exact_target(
        &self,
        expected: &ExpectedFormatTarget,
    ) -> Result<ResolvedFormatTarget, FormatProviderError> {
        let drive = drive_designator(&expected.current_mount_root)
            .ok_or(FormatProviderError::TargetUnavailable)?;
        let _apartment =
            ComApartment::initialize().map_err(|_| FormatProviderError::TargetUnavailable)?;
        let services = storage_services().map_err(|_| FormatProviderError::TargetUnavailable)?;
        let object = volume_for_drive(&services, &drive)
            .map_err(|_| FormatProviderError::TargetUnavailable)?;
        let capacity =
            property_u64(&object, "Size").map_err(|_| FormatProviderError::TargetUnavailable)?;
        if capacity != expected.expected_capacity_bytes {
            return Err(FormatProviderError::TargetChanged);
        }
        let provider_key = property_string(&object, "__RELPATH")
            .map_err(|_| FormatProviderError::TargetUnavailable)?;
        if provider_key.is_empty() {
            return Err(FormatProviderError::TargetUnavailable);
        }
        Ok(ResolvedFormatTarget {
            provider_key,
            medium_key: expected.medium_key.clone(),
            connection_generation: expected.connection_generation,
            capacity_bytes: capacity,
        })
    }

    fn quick_format(
        &self,
        target: &ResolvedFormatTarget,
        profile: &FormatProfile,
    ) -> Result<(), FormatProviderError> {
        let _apartment =
            ComApartment::initialize().map_err(|_| FormatProviderError::FormatFailed)?;
        let services = storage_services().map_err(|_| FormatProviderError::FormatFailed)?;
        let volume = object_at(&services, &target.provider_key)
            .map_err(|_| FormatProviderError::TargetChanged)?;
        if property_u64(&volume, "Size").map_err(|_| FormatProviderError::TargetChanged)?
            != target.capacity_bytes
        {
            return Err(FormatProviderError::TargetChanged);
        }
        let input =
            format_input(&services, profile).map_err(|_| FormatProviderError::FormatFailed)?;
        let mut output = None;
        unsafe {
            services
                .ExecMethod(
                    &BSTR::from(&target.provider_key),
                    &BSTR::from(FORMAT_METHOD),
                    Default::default(),
                    None,
                    &input,
                    Some(&mut output),
                    None,
                )
                .map_err(|_| FormatProviderError::FormatFailed)?;
        }
        let output = output.ok_or(FormatProviderError::FormatFailed)?;
        let return_value =
            property_u64(&output, "ReturnValue").map_err(|_| FormatProviderError::FormatFailed)?;
        if return_value == 0 {
            Ok(())
        } else if return_value == 42014 || return_value == 42015 {
            Err(FormatProviderError::Busy)
        } else if return_value == 5 {
            Err(FormatProviderError::WriteProtected)
        } else {
            Err(FormatProviderError::FormatFailed)
        }
    }

    fn wait_for_validated_mount(
        &self,
        expected: &ExpectedFormatTarget,
        profile: &FormatProfile,
    ) -> Result<ValidatedMount, FormatProviderError> {
        let drive = drive_designator(&expected.current_mount_root)
            .ok_or(FormatProviderError::RemountFailed)?;
        let deadline = Instant::now() + FORMAT_TIMEOUT;
        loop {
            let attempt = (|| {
                let _apartment =
                    ComApartment::initialize().map_err(|_| FormatProviderError::RemountFailed)?;
                let services =
                    storage_services().map_err(|_| FormatProviderError::RemountFailed)?;
                let volume = volume_for_drive(&services, &drive)
                    .map_err(|_| FormatProviderError::RemountFailed)?;
                let capacity = property_u64(&volume, "Size")
                    .map_err(|_| FormatProviderError::RemountFailed)?;
                if capacity != expected.expected_capacity_bytes {
                    return Err(FormatProviderError::TargetChanged);
                }
                let filesystem = property_string(&volume, "FileSystem")
                    .map_err(|_| FormatProviderError::ValidationFailed)?;
                if !filesystem.eq_ignore_ascii_case(filesystem_name(profile.filesystem)) {
                    return Err(FormatProviderError::ValidationFailed);
                }
                Ok(ValidatedMount {
                    root: expected.current_mount_root.clone(),
                    filesystem,
                    capacity_bytes: capacity,
                })
            })();
            match attempt {
                Ok(mount) => return Ok(mount),
                Err(FormatProviderError::TargetChanged)
                | Err(FormatProviderError::ValidationFailed)
                    if Instant::now() >= deadline =>
                {
                    return Err(FormatProviderError::RemountFailed)
                }
                Err(_) if Instant::now() >= deadline => {
                    return Err(FormatProviderError::RemountFailed)
                }
                Err(_) => thread::sleep(Duration::from_millis(500)),
            }
        }
    }
}

fn format_input(
    services: &IWbemServices,
    profile: &FormatProfile,
) -> windows::core::Result<IWbemClassObject> {
    // `GetMethod` is defined on the class object, not the selected volume
    // instance. The instance path remains the sole target of `ExecMethod`.
    let class = object_at(services, VOLUME_CLASS)?;
    let mut input_signature = None;
    unsafe {
        class.GetMethod(
            PCWSTR(wide_nul(FORMAT_METHOD).as_ptr()),
            0,
            &mut input_signature,
            std::ptr::null_mut(),
        )?;
    }
    let input = unsafe {
        input_signature
            .ok_or_else(windows::core::Error::empty)?
            .SpawnInstance(0)?
    };
    let filesystem = match profile.filesystem {
        FormatFilesystem::Fat => "FAT",
        FormatFilesystem::Fat32 => "FAT32",
        FormatFilesystem::Exfat => "exFAT",
    };
    put_bstr(&input, "FileSystem", filesystem)?;
    put_bool(&input, "Full", false)?;
    // Never override open-handle or write-protection checks. The provider
    // reports a failed/busy format instead of asking Windows to force it.
    put_bool(&input, "Force", false)?;
    Ok(input)
}

fn filesystem_name(kind: FormatFilesystem) -> &'static str {
    match kind {
        FormatFilesystem::Fat => "FAT",
        FormatFilesystem::Fat32 => "FAT32",
        FormatFilesystem::Exfat => "exFAT",
    }
}

fn drive_designator(root: &Path) -> Option<String> {
    let root = root.to_string_lossy();
    let bytes = root.as_bytes();
    (bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':')
        // `MSFT_Volume.DriveLetter` is the letter only (`D`), whereas the
        // native mount root is `D:\\` and the Win32 volume handle uses `D:`.
        .then(|| root[..1].to_ascii_uppercase())
}

fn volume_for_drive(
    services: &IWbemServices,
    drive: &str,
) -> windows::core::Result<IWbemClassObject> {
    let query = format!("SELECT * FROM {VOLUME_CLASS} WHERE DriveLetter = '{drive}'");
    let enumerator = unsafe {
        services.ExecQuery(
            &BSTR::from(WQL),
            &BSTR::from(query),
            WBEM_FLAG_RETURN_IMMEDIATELY | WBEM_FLAG_FORWARD_ONLY,
            None,
        )?
    };
    let mut result = [None];
    let mut returned = 0;
    unsafe {
        enumerator
            .Next(WBEM_INFINITE, &mut result, &mut returned)
            .ok()?;
    }
    if returned == 1 {
        result[0].take().ok_or_else(windows::core::Error::empty)
    } else {
        Err(windows::core::Error::empty())
    }
}

fn object_at(services: &IWbemServices, path: &str) -> windows::core::Result<IWbemClassObject> {
    let mut object = None;
    unsafe {
        services.GetObject(
            &BSTR::from(path),
            Default::default(),
            None,
            Some(&mut object),
            None,
        )?;
    }
    object.ok_or_else(windows::core::Error::empty)
}

fn storage_services() -> windows::core::Result<IWbemServices> {
    let locator: IWbemLocator = unsafe {
        CoCreateInstance(
            &GUID::from_u128(0x4590f811_1d3a_11d0_891f_00aa004b2e24),
            None,
            CLSCTX_INPROC_SERVER,
        )?
    };
    unsafe {
        let services = locator.ConnectServer(
            &BSTR::from(STORAGE_NAMESPACE),
            &BSTR::new(),
            &BSTR::new(),
            &BSTR::new(),
            0,
            &BSTR::new(),
            None,
        )?;
        windows::Win32::System::Com::CoSetProxyBlanket(
            &services,
            RPC_C_AUTHN_WINNT,
            RPC_C_AUTHZ_NONE,
            None,
            RPC_C_AUTHN_LEVEL_CALL,
            RPC_C_IMP_LEVEL_IMPERSONATE,
            None,
            EOAC_NONE,
        )?;
        Ok(services)
    }
}

fn property_string(object: &IWbemClassObject, name: &str) -> windows::core::Result<String> {
    let mut value = VARIANT::default();
    unsafe {
        object.Get(PCWSTR(wide_nul(name).as_ptr()), 0, &mut value, None, None)?;
        let value_type = variant_type(&value);
        if value_type != VT_BSTR {
            let _ = VariantClear(&mut value);
            return Err(windows::core::Error::empty());
        }
        let bstr = variant_bstr(&value).to_string();
        VariantClear(&mut value)?;
        Ok(bstr)
    }
}

fn property_u64(object: &IWbemClassObject, name: &str) -> windows::core::Result<u64> {
    let mut value = VARIANT::default();
    unsafe {
        object.Get(PCWSTR(wide_nul(name).as_ptr()), 0, &mut value, None, None)?;
        let result = match variant_type(&value) {
            VT_UI8 => variant_u64(&value),
            VT_UI4 => u64::from(variant_u32(&value)),
            // Classic WMI marshals CIM_UINT64 properties as strings to avoid
            // automation's historical 64-bit numeric limitation.
            VT_BSTR => match variant_bstr(&value).to_string().parse::<u64>() {
                Ok(value) => value,
                Err(_) => {
                    let _ = VariantClear(&mut value);
                    return Err(windows::core::Error::empty());
                }
            },
            _ => {
                let _ = VariantClear(&mut value);
                return Err(windows::core::Error::empty());
            }
        };
        VariantClear(&mut value)?;
        Ok(result)
    }
}

fn put_bool(object: &IWbemClassObject, name: &str, value: bool) -> windows::core::Result<()> {
    let mut variant = VARIANT::default();
    unsafe {
        variant_header(&mut variant).vt = VT_BOOL;
        variant_header(&mut variant).Anonymous.boolVal =
            if value { VARIANT_TRUE } else { VARIANT_FALSE };
        object.Put(PCWSTR(wide_nul(name).as_ptr()), 0, &variant, 0)
    }
}

fn put_bstr(object: &IWbemClassObject, name: &str, value: &str) -> windows::core::Result<()> {
    let mut variant = VARIANT::default();
    unsafe {
        variant_header(&mut variant).vt = VT_BSTR;
        variant_header(&mut variant).Anonymous.bstrVal =
            std::mem::ManuallyDrop::new(BSTR::from(value));
        let result = object.Put(PCWSTR(wide_nul(name).as_ptr()), 0, &variant, 0);
        VariantClear(&mut variant)?;
        result
    }
}

unsafe fn variant_header(value: &mut VARIANT) -> &mut VARIANT_0_0 {
    unsafe { &mut *std::ptr::addr_of_mut!(value.Anonymous.Anonymous).cast::<VARIANT_0_0>() }
}

unsafe fn variant_header_ref(value: &VARIANT) -> &VARIANT_0_0 {
    unsafe { &*std::ptr::addr_of!(value.Anonymous.Anonymous).cast::<VARIANT_0_0>() }
}

unsafe fn variant_type(value: &VARIANT) -> VARENUM {
    unsafe { variant_header_ref(value).vt }
}

unsafe fn variant_bstr(value: &VARIANT) -> &BSTR {
    unsafe { &*std::ptr::addr_of!(variant_header_ref(value).Anonymous.bstrVal).cast::<BSTR>() }
}

unsafe fn variant_u64(value: &VARIANT) -> u64 {
    unsafe { variant_header_ref(value).Anonymous.ullVal }
}

unsafe fn variant_u32(value: &VARIANT) -> u32 {
    unsafe { variant_header_ref(value).Anonymous.ulVal }
}

fn wide_nul(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

struct ComApartment;

impl ComApartment {
    fn initialize() -> windows::core::Result<Self> {
        unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
            match CoInitializeSecurity(
                None,
                -1,
                None,
                None,
                RPC_C_AUTHN_LEVEL_CALL,
                RPC_C_IMP_LEVEL_IMPERSONATE,
                None,
                EOAC_NONE,
                None,
            ) {
                Ok(()) => Ok(Self),
                Err(error) if error.code() == RPC_E_TOO_LATE => Ok(Self),
                Err(error) => Err(error),
            }
        }
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        unsafe { CoUninitialize() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

    #[test]
    fn drive_designator_accepts_only_a_windows_drive_root() {
        assert_eq!(drive_designator(Path::new(r"D:\")), Some("D".into()));
        assert_eq!(drive_designator(Path::new(r"D:\DCIM")), Some("D".into()));
        assert_eq!(drive_designator(Path::new(r"\\server\share")), None);
    }

    /// Read-only hardware probe for a deliberately supplied sacrificial card.
    /// It does not call `quick_format`; it only proves the provider can bind
    /// the freshly observed native root to exactly one WMI volume object.
    #[test]
    #[ignore = "set MEDIA_INGEST_HW_DRIVE to a sacrificial mounted card"]
    fn hardware_volume_resolution_probe() {
        let root = std::env::var("MEDIA_INGEST_HW_DRIVE")
            .expect("set MEDIA_INGEST_HW_DRIVE, for example D:\\");
        let mut capacity = 0_u64;
        let root_wide = wide_nul(&root);
        unsafe {
            GetDiskFreeSpaceExW(PCWSTR(root_wide.as_ptr()), None, Some(&mut capacity), None)
                .expect("read removable volume capacity");
        }
        let drive = drive_designator(Path::new(&root)).expect("drive root");
        let _apartment = ComApartment::initialize().expect("initialize COM");
        let services = storage_services().expect("connect Storage Management WMI");
        let object = volume_for_drive(&services, &drive).expect("query WMI volume for drive");
        assert_eq!(
            property_u64(&object, "Size").expect("read WMI volume size"),
            capacity
        );
        let object_path = property_string(&object, "__RELPATH").expect("read WMI object path");
        assert!(!object_path.is_empty());
        let reopened = object_at(&services, &object_path).expect("reopen opaque WMI object path");
        assert_eq!(
            property_u64(&reopened, "Size").expect("read reopened WMI volume size"),
            capacity
        );
        let input = format_input(
            &services,
            &FormatProfile {
                id: "sdxc-default",
                filesystem: FormatFilesystem::Exfat,
                inferred_from_capacity: true,
            },
        )
        .expect("construct documented WMI Format input without formatting");
        assert_eq!(
            property_string(&input, "FileSystem").expect("read input filesystem"),
            "exFAT"
        );
        let resolved = WindowsStorageProvider
            .resolve_exact_target(&ExpectedFormatTarget {
                medium_key: "hardware-probe-only".into(),
                connection_generation: 1,
                expected_capacity_bytes: capacity,
                current_mount_root: Path::new(&root).to_path_buf(),
            })
            .expect("resolve exact WMI volume");
        assert_eq!(resolved.capacity_bytes, capacity);
        assert!(!resolved.provider_key.is_empty());
    }
}
