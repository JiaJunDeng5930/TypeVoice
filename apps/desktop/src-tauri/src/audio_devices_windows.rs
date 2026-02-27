#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DefaultCaptureRole {
    Communications,
    Console,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioEndpointInfo {
    pub endpoint_id: String,
    pub friendly_name: String,
}

#[cfg(windows)]
mod imp {
    use super::{AudioEndpointInfo, DefaultCaptureRole};
    use windows::core::{Interface, HRESULT, HSTRING, PWSTR};
    use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
    use windows::Win32::Foundation::RPC_E_CHANGED_MODE;
    use windows::Win32::Media::Audio::{
        eCapture, eCommunications, eConsole, ERole, IMMDevice, IMMDeviceEnumerator,
        MMDeviceEnumerator, DEVICE_STATE_ACTIVE,
    };
    use windows::Win32::System::Com::StructuredStorage::PropVariantToStringAlloc;
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_ALL,
        COINIT_MULTITHREADED, STGM_READ,
    };

    struct ComInitGuard {
        should_uninit: bool,
    }

    impl Drop for ComInitGuard {
        fn drop(&mut self) {
            if self.should_uninit {
                unsafe {
                    CoUninitialize();
                }
            }
        }
    }

    fn ensure_com_initialized() -> Result<ComInitGuard, String> {
        let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        if hr.is_ok() {
            return Ok(ComInitGuard {
                should_uninit: true,
            });
        }
        if hr == RPC_E_CHANGED_MODE {
            return Ok(ComInitGuard {
                should_uninit: false,
            });
        }
        Err(format!(
            "E_RECORD_INPUT_COM_INIT_FAILED: CoInitializeEx failed: {}",
            format_hresult(hr)
        ))
    }

    fn format_hresult(hr: HRESULT) -> String {
        format!("0x{:08X}", hr.0 as u32)
    }

    fn with_enumerator<T>(
        f: impl FnOnce(&IMMDeviceEnumerator) -> Result<T, String>,
    ) -> Result<T, String> {
        let _com_guard = ensure_com_initialized()?;
        let enumerator: IMMDeviceEnumerator = unsafe {
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(|e| {
                format!("E_RECORD_INPUT_ENUMERATOR_CREATE_FAILED: CoCreateInstance failed: {e}")
            })?
        };
        f(&enumerator)
    }

    unsafe fn pwstr_to_string(ptr: PWSTR) -> String {
        if ptr.is_null() {
            return String::new();
        }
        let mut len = 0usize;
        while *ptr.0.add(len) != 0 {
            len += 1;
        }
        let slice = std::slice::from_raw_parts(ptr.0, len);
        String::from_utf16_lossy(slice)
    }

    fn endpoint_from_device(device: &IMMDevice) -> Result<AudioEndpointInfo, String> {
        let endpoint_id = unsafe {
            let id_ptr = device.GetId().map_err(|e| {
                format!("E_RECORD_INPUT_ENDPOINT_ID_FAILED: IMMDevice::GetId failed: {e}")
            })?;
            let text = pwstr_to_string(id_ptr);
            CoTaskMemFree(Some(id_ptr.0.cast()));
            text
        };
        if endpoint_id.trim().is_empty() {
            return Err("E_RECORD_INPUT_ENDPOINT_ID_FAILED: endpoint id is empty".to_string());
        }

        let friendly_name = unsafe {
            let store = device.OpenPropertyStore(STGM_READ).map_err(|e| {
                format!(
                    "E_RECORD_INPUT_PROPERTY_STORE_FAILED: IMMDevice::OpenPropertyStore failed: {e}"
                )
            })?;
            let value = store.GetValue(&PKEY_Device_FriendlyName).map_err(|e| {
                format!("E_RECORD_INPUT_FRIENDLY_NAME_FAILED: IPropertyStore::GetValue failed: {e}")
            })?;
            let name_ptr = PropVariantToStringAlloc(&value).map_err(|e| {
                format!("E_RECORD_INPUT_FRIENDLY_NAME_FAILED: PropVariantToStringAlloc failed: {e}")
            })?;
            let text = pwstr_to_string(name_ptr);
            CoTaskMemFree(Some(name_ptr.0.cast()));
            text
        };
        if friendly_name.trim().is_empty() {
            return Err("E_RECORD_INPUT_FRIENDLY_NAME_FAILED: friendly name is empty".to_string());
        }
        Ok(AudioEndpointInfo {
            endpoint_id,
            friendly_name,
        })
    }

    fn role_to_erole(role: DefaultCaptureRole) -> ERole {
        match role {
            DefaultCaptureRole::Communications => eCommunications,
            DefaultCaptureRole::Console => eConsole,
        }
    }

    pub fn get_default_capture_endpoint(
        role: DefaultCaptureRole,
    ) -> Result<AudioEndpointInfo, String> {
        with_enumerator(|enumerator| {
            let device = unsafe {
                enumerator
                    .GetDefaultAudioEndpoint(eCapture, role_to_erole(role))
                    .map_err(|e| {
                        format!(
                            "E_RECORD_INPUT_DEFAULT_FAILED: IMMDeviceEnumerator::GetDefaultAudioEndpoint failed: {e}"
                        )
                    })?
            };
            endpoint_from_device(&device)
        })
    }

    pub fn get_capture_endpoint_by_id(endpoint_id: &str) -> Result<AudioEndpointInfo, String> {
        let trimmed = endpoint_id.trim();
        if trimmed.is_empty() {
            return Err("E_RECORD_INPUT_FIXED_MISSING: fixed endpoint id is empty".to_string());
        }
        with_enumerator(|enumerator| {
            let target = HSTRING::from(trimmed);
            let device = unsafe {
                enumerator.GetDevice(&target).map_err(|e| {
                    format!("E_RECORD_INPUT_FIXED_NOT_FOUND: IMMDeviceEnumerator::GetDevice failed: {e}")
                })?
            };
            endpoint_from_device(&device)
        })
    }

    pub fn list_active_capture_endpoints() -> Result<Vec<AudioEndpointInfo>, String> {
        with_enumerator(|enumerator| {
            let collection = unsafe {
                enumerator
                    .EnumAudioEndpoints(eCapture, DEVICE_STATE_ACTIVE)
                    .map_err(|e| {
                        format!(
                            "E_RECORD_INPUT_ENUM_FAILED: IMMDeviceEnumerator::EnumAudioEndpoints failed: {e}"
                        )
                    })?
            };
            let count = unsafe {
                collection.GetCount().map_err(|e| {
                    format!("E_RECORD_INPUT_ENUM_FAILED: IMMDeviceCollection::GetCount failed: {e}")
                })?
            };
            let mut out = Vec::with_capacity(count as usize);
            for idx in 0..count {
                let device = unsafe {
                    collection.Item(idx).map_err(|e| {
                        format!("E_RECORD_INPUT_ENUM_FAILED: IMMDeviceCollection::Item failed: {e}")
                    })?
                };
                out.push(endpoint_from_device(&device)?);
            }
            Ok(out)
        })
    }
}

#[cfg(windows)]
pub use imp::{
    get_capture_endpoint_by_id, get_default_capture_endpoint, list_active_capture_endpoints,
};

#[cfg(not(windows))]
pub fn get_default_capture_endpoint(
    _role: DefaultCaptureRole,
) -> Result<AudioEndpointInfo, String> {
    Err("E_RECORD_UNSUPPORTED: backend recording is only supported on Windows".to_string())
}

#[cfg(not(windows))]
pub fn get_capture_endpoint_by_id(_endpoint_id: &str) -> Result<AudioEndpointInfo, String> {
    Err("E_RECORD_UNSUPPORTED: backend recording is only supported on Windows".to_string())
}

#[cfg(not(windows))]
pub fn list_active_capture_endpoints() -> Result<Vec<AudioEndpointInfo>, String> {
    Err("E_RECORD_UNSUPPORTED: backend recording is only supported on Windows".to_string())
}
