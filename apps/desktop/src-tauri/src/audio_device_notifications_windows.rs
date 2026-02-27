use std::path::Path;
use std::sync::Mutex;

#[cfg(windows)]
use serde_json::json;

use crate::record_input_cache::RecordInputCacheState;

#[cfg_attr(not(windows), allow(dead_code))]
pub struct AudioDeviceNotificationState {
    guard: Mutex<Option<AudioDeviceNotificationGuard>>,
}

impl AudioDeviceNotificationState {
    pub fn new() -> Self {
        Self {
            guard: Mutex::new(None),
        }
    }

    pub fn start_best_effort(&self, data_dir: &Path, cache: RecordInputCacheState) {
        #[cfg(not(windows))]
        {
            let _ = data_dir;
            let _ = cache;
        }

        #[cfg(windows)]
        {
            let span = crate::trace::Span::start(
                data_dir,
                None,
                "App",
                "APP.audio_device_listener_start",
                None,
            );
            let mut g = self.guard.lock().unwrap();
            if g.is_some() {
                span.ok(Some(json!({ "already_running": true })));
                return;
            }
            match imp::start_listener(data_dir, cache) {
                Ok(listener_guard) => {
                    *g = Some(listener_guard);
                    span.ok(Some(json!({ "started": true })));
                }
                Err(e) => {
                    span.err("config", "E_AUDIO_DEVICE_LISTENER_START_FAILED", &e, None);
                }
            }
        }
    }
}

#[cfg(windows)]
struct AudioDeviceNotificationGuard {
    stop_tx: std::sync::mpsc::Sender<()>,
    join: Option<std::thread::JoinHandle<()>>,
}

#[cfg(windows)]
impl Drop for AudioDeviceNotificationGuard {
    fn drop(&mut self) {
        let _ = self.stop_tx.send(());
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

#[cfg(not(windows))]
struct AudioDeviceNotificationGuard;

#[cfg(windows)]
mod imp {
    use std::path::{Path, PathBuf};
    use std::sync::mpsc;
    use std::time::Duration;

    use serde_json::json;
    use windows::core::{implement, PCWSTR};
    use windows::Win32::Foundation::RPC_E_CHANGED_MODE;
    use windows::Win32::Media::Audio::{
        eAll, eCapture, eCommunications, eConsole, eMultimedia, eRender, EDataFlow, ERole,
        IMMDeviceEnumerator, IMMNotificationClient, IMMNotificationClient_Impl, MMDeviceEnumerator,
        DEVICE_STATE,
    };
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_MULTITHREADED,
    };
    use windows::Win32::UI::Shell::PropertiesSystem::PROPERTYKEY;

    use crate::audio_device_notifications_windows::AudioDeviceNotificationGuard;
    use crate::record_input_cache::RecordInputCacheState;

    pub fn start_listener(
        data_dir: &Path,
        cache: RecordInputCacheState,
    ) -> Result<AudioDeviceNotificationGuard, String> {
        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        let (init_tx, init_rx) = mpsc::channel::<Result<(), String>>();
        let data_dir_buf = data_dir.to_path_buf();
        let join = std::thread::spawn(move || {
            listener_thread(data_dir_buf, cache, stop_rx, init_tx);
        });

        match init_rx.recv_timeout(Duration::from_secs(3)) {
            Ok(Ok(())) => Ok(AudioDeviceNotificationGuard {
                stop_tx,
                join: Some(join),
            }),
            Ok(Err(e)) => {
                let _ = join.join();
                Err(e)
            }
            Err(e) => {
                let _ = stop_tx.send(());
                let _ = join.join();
                Err(format!(
                    "E_AUDIO_DEVICE_LISTENER_START_FAILED: listener init timeout: {e}"
                ))
            }
        }
    }

    fn listener_thread(
        data_dir: PathBuf,
        cache: RecordInputCacheState,
        stop_rx: mpsc::Receiver<()>,
        init_tx: mpsc::Sender<Result<(), String>>,
    ) {
        let _com = match ensure_com_initialized() {
            Ok(v) => v,
            Err(e) => {
                let _ = init_tx.send(Err(e));
                return;
            }
        };

        let enumerator: IMMDeviceEnumerator =
            match unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) } {
                Ok(v) => v,
                Err(e) => {
                    let _ = init_tx.send(Err(format!(
                        "E_AUDIO_DEVICE_LISTENER_START_FAILED: CoCreateInstance failed: {e}"
                    )));
                    return;
                }
            };

        let client_impl = DeviceNotificationClient {
            data_dir: data_dir.clone(),
            cache,
        };
        let client: IMMNotificationClient = client_impl.into();

        if let Err(e) = unsafe { enumerator.RegisterEndpointNotificationCallback(&client) } {
            let _ = init_tx.send(Err(format!(
                "E_AUDIO_DEVICE_LISTENER_START_FAILED: RegisterEndpointNotificationCallback failed: {e}"
            )));
            return;
        }

        let _ = init_tx.send(Ok(()));
        let _ = stop_rx.recv();
        let _ = unsafe { enumerator.UnregisterEndpointNotificationCallback(&client) };
    }

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
            "E_AUDIO_DEVICE_LISTENER_START_FAILED: CoInitializeEx failed: 0x{:08X}",
            hr.0 as u32
        ))
    }

    #[implement(windows::Win32::Media::Audio::IMMNotificationClient)]
    struct DeviceNotificationClient {
        data_dir: PathBuf,
        cache: RecordInputCacheState,
    }

    impl IMMNotificationClient_Impl for DeviceNotificationClient_Impl {
        fn OnDeviceStateChanged(
            &self,
            pwstrdeviceid: &PCWSTR,
            dwnewstate: DEVICE_STATE,
        ) -> windows::core::Result<()> {
            emit_event(
                &self.data_dir,
                &self.cache,
                "device_state_changed",
                None,
                None,
                pcwstr_to_string(pwstrdeviceid).as_str(),
                Some(dwnewstate.0),
                true,
            );
            Ok(())
        }

        fn OnDeviceAdded(&self, pwstrdeviceid: &PCWSTR) -> windows::core::Result<()> {
            emit_event(
                &self.data_dir,
                &self.cache,
                "device_added",
                None,
                None,
                pcwstr_to_string(pwstrdeviceid).as_str(),
                None,
                true,
            );
            Ok(())
        }

        fn OnDeviceRemoved(&self, pwstrdeviceid: &PCWSTR) -> windows::core::Result<()> {
            emit_event(
                &self.data_dir,
                &self.cache,
                "device_removed",
                None,
                None,
                pcwstr_to_string(pwstrdeviceid).as_str(),
                None,
                true,
            );
            Ok(())
        }

        fn OnDefaultDeviceChanged(
            &self,
            flow: EDataFlow,
            role: ERole,
            pwstrdefaultdeviceid: &PCWSTR,
        ) -> windows::core::Result<()> {
            let refresh = flow == eCapture;
            emit_event(
                &self.data_dir,
                &self.cache,
                "default_device_changed",
                Some(flow),
                Some(role),
                pcwstr_to_string(pwstrdefaultdeviceid).as_str(),
                None,
                refresh,
            );
            Ok(())
        }

        fn OnPropertyValueChanged(
            &self,
            pwstrdeviceid: &PCWSTR,
            _key: &PROPERTYKEY,
        ) -> windows::core::Result<()> {
            emit_event(
                &self.data_dir,
                &self.cache,
                "property_value_changed",
                None,
                None,
                pcwstr_to_string(pwstrdeviceid).as_str(),
                None,
                true,
            );
            Ok(())
        }
    }

    fn pcwstr_to_string(v: &PCWSTR) -> String {
        unsafe { v.to_string().unwrap_or_default() }
    }

    fn emit_event(
        data_dir: &Path,
        cache: &RecordInputCacheState,
        event_type: &str,
        flow: Option<EDataFlow>,
        role: Option<ERole>,
        endpoint_id: &str,
        state: Option<u32>,
        should_refresh: bool,
    ) {
        crate::trace::event(
            data_dir,
            None,
            "App",
            "APP.audio_device_event",
            "ok",
            Some(json!({
                "event_type": event_type,
                "flow": flow.map(flow_label),
                "role": role.map(role_label),
                "endpoint_id": endpoint_id,
                "state": state,
                "refresh_requested": should_refresh,
            })),
        );

        if should_refresh {
            cache.request_refresh(
                data_dir.to_path_buf(),
                format!(
                    "device_event:{}:{}:{}",
                    event_type,
                    flow.map(flow_label).unwrap_or_else(|| "none".to_string()),
                    role.map(role_label).unwrap_or_else(|| "none".to_string())
                ),
            );
        }
    }

    fn flow_label(flow: EDataFlow) -> String {
        if flow == eCapture {
            return "capture".to_string();
        }
        if flow == eRender {
            return "render".to_string();
        }
        if flow == eAll {
            return "all".to_string();
        }
        format!("raw_{}", flow.0)
    }

    fn role_label(role: ERole) -> String {
        if role == eCommunications {
            return "communications".to_string();
        }
        if role == eConsole {
            return "console".to_string();
        }
        if role == eMultimedia {
            return "multimedia".to_string();
        }
        format!("raw_{}", role.0)
    }
}
