pub use typevoice_core::{context_pack, ports};
pub use typevoice_observability::obs;
#[cfg(windows)]
pub use typevoice_platform::context_capture_windows;
pub use typevoice_platform::{
    audio_device_notifications_windows, audio_devices_windows, context_capture, export, insertion,
    pipeline, record_input, record_input_cache, subprocess, toolchain,
};
pub use typevoice_providers::{doubao_asr, llm, remote_asr};
pub use typevoice_storage::{data_dir, history, settings};

pub mod audio_capture;
pub mod rewrite;
pub mod task_manager;
pub mod transcription;
pub mod transcription_actor;
pub mod ui_events;
pub mod voice_tasks;
pub mod voice_workflow;

pub struct RuntimeState {
    toolchain: std::sync::Mutex<toolchain::ToolchainStatus>,
}

impl RuntimeState {
    pub fn new() -> Self {
        Self {
            toolchain: std::sync::Mutex::new(toolchain::ToolchainStatus::pending()),
        }
    }

    pub fn set_toolchain(&self, st: toolchain::ToolchainStatus) {
        let mut g = self.toolchain.lock().unwrap();
        *g = st;
    }

    pub fn get_toolchain(&self) -> toolchain::ToolchainStatus {
        self.toolchain.lock().unwrap().clone()
    }
}
