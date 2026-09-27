pub mod bionic;
pub mod gpu;
pub mod hub;
pub mod jsonl;
pub mod ninfer;
pub mod tailer;

pub use bionic::Bionic;
pub use gpu::GpuPoller;
pub use hub::{Hub, Snapshot};
pub use ninfer::Ninfer;

use serde::Serialize;

pub const NINFER_ENGINE: &str = "ninfer";
pub const BIONIC_ENGINE: &str = "bionic";
pub const GPU_ENGINE: &str = "gpu";

/// Console-child helper: a console subprocess of a GUI app (windows
/// subsystem) would flash a console window on every spawn. On Windows,
/// suppress the window with CREATE_NO_WINDOW.
#[cfg(windows)]
pub fn no_window(mut cmd: std::process::Command) -> std::process::Command {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
}

#[cfg(not(windows))]
pub fn no_window(cmd: std::process::Command) -> std::process::Command {
    cmd
}

/// Like `no_window`, but also DETACHED_PROCESS: the child fully outlives
/// the parent (used for the persistent WSL keepalive session).
#[cfg(windows)]
pub fn no_window_detached(mut cmd: std::process::Command) -> std::process::Command {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    const DETACHED_PROCESS: u32 = 0x00000008;
    cmd.creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS);
    cmd
}

#[cfg(not(windows))]
pub fn no_window_detached(cmd: std::process::Command) -> std::process::Command {
    cmd
}

#[derive(Debug, Clone, Copy, Default, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct GpuSample {
    pub mem_used_mb: u64,
    pub mem_total_mb: u64,
    pub util_pct: u32,
    pub power_w: f64,
    pub temp_c: u32,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct MtpStats {
    pub rounds: u64,
    pub drafted: u64,
    pub accepted: u64,
    pub accept_rate: f64,
}

/// Telemetry event. Wire format: internally-tagged (`kind` discriminator,
/// payload fields flat) — the frontend types in src/lib/telemetry.ts match.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EventKind {
    Throughput {
        decode_tps: f64,
        prefill_tps: f64,
        decode_tokens: u64,
        prefill_tokens: u64,
        interval_s: f64,
        running: u32,
        waiting: u32,
        avg_batch: f64,
        mtp_accept_rate: Option<f64>,
    },
    RequestStarted {
        protocol: String,
        stream: bool,
        tools: bool,
        reasoning_effort: Option<String>,
    },
    RequestFinished {
        prompt_tokens: u64,
        completion_tokens: u64,
        ttft_s: f64,
        decode_s: f64,
        total_s: f64,
        decode_tps: f64,
        mtp: Option<MtpStats>,
        finish_reason: String,
    },
    ModelStates {
        loaded: Vec<String>,
        models: Vec<ModelInfo>,
    },
    Gpu {
        gpu: GpuSample,
    },
    EngineUp {
        engine: String,
    },
    EngineDown {
        engine: String,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct TelemetryEvent {
    pub engine: String,
    pub ts: i64,
    pub kind: EventKind,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelInfo {
    pub id: String,
    pub engine: String,
    pub quant: String,
    pub context: u64,
    pub file: String,
    pub size_gib: Option<f64>,
    pub loaded: bool,
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
