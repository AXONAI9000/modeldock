use std::path::PathBuf;
use std::process::Command;

use tokio::sync::mpsc;

use crate::{
    jsonl, tailer, now_ms, EventKind, ModelInfo, TelemetryEvent, NINFER_ENGINE,
};

pub struct Ninfer {
    base: String,
    distro: String,
    jsonl_path: PathBuf,
    client: reqwest::Client,
}

impl Default for Ninfer {
    fn default() -> Self {
        Self {
            base: "http://127.0.0.1:8080".to_string(),
            distro: "Ubuntu-24.04".to_string(),
            jsonl_path: PathBuf::from(
                r"\\wsl.localhost\Ubuntu-24.04\var\log\ninfer\request-log.jsonl",
            ),
            client: reqwest::Client::new(),
        }
    }
}

impl Ninfer {
    pub async fn health(&self) -> bool {
        self.client
            .get(format!("{}/health", self.base))
            .send()
            .await
            .map(|r| r.status().as_u16() == 200)
            .unwrap_or(false)
    }

    /// Static artifact info, `loaded`/context patched from /v1/models.
    pub async fn models(&self) -> Vec<ModelInfo> {
        let mut out = vec![ModelInfo {
            id: "qwen3.8-27b".to_string(),
            engine: NINFER_ENGINE.to_string(),
            quant: "NVFP4".to_string(),
            context: 240_000,
            file: r"D:\models\qwen3_8_27b_nvfp4.ninfer".to_string(),
            size_gib: Some(22.09),
            loaded: false,
        }];
        let Ok(resp) = self
            .client
            .get(format!("{}/v1/models", self.base))
            .timeout(std::time::Duration::from_secs(2))
            .send()
            .await
        else {
            return out;
        };
        let Ok(v) = resp.json::<serde_json::Value>().await else {
            return out;
        };
        if let Some(data) = v.get("data").and_then(|d| d.as_array()) {
            for m in data {
                let id = m.get("id").and_then(|x| x.as_str()).unwrap_or("");
                if id.is_empty() {
                    continue;
                }
                let mlen = m.get("max_model_len").and_then(|x| x.as_u64());
                for o in out.iter_mut() {
                    if o.id == id {
                        o.loaded = true;
                        if let Some(ml) = mlen {
                            o.context = ml;
                        }
                    }
                }
            }
        }
        out
    }

    async fn systemctl(&self, verb: &str) -> Result<(), String> {
        let distro = self.distro.clone();
        let verb = verb.to_string();
        let verb_arg = verb.clone();
        let out = tokio::task::spawn_blocking(move || {
            let mut cmd = Command::new("wsl");
            cmd.args([
                "-d",
                distro.as_str(),
                "-e",
                "systemctl",
                verb_arg.as_str(),
                "ninfer",
            ]);
            crate::no_window(cmd).output()
        })
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
        let stderr = String::from_utf8_lossy(&out.stderr).to_string();
        if out.status.success() {
            Ok(())
        } else {
            Err(format!("systemctl {} failed: {}", verb, stderr))
        }
    }

    /// True if a persistent keepalive wsl session already exists.
    async fn keepalive_present(&self) -> bool {
        let res = tokio::task::spawn_blocking(|| {
            let mut cmd = Command::new("powershell");
            cmd.args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Get-CimInstance Win32_Process -Filter \"Name='wsl.exe'\" | Where-Object { $_.CommandLine -like '*tail -f /dev/null*' } | Measure-Object | Select-Object -ExpandProperty Count",
            ]);
            crate::no_window(cmd).output()
        })
        .await;
        match res {
            Ok(Ok(out)) if out.status.success() => {
                let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
                s.parse::<u64>().unwrap_or(0) > 0
            }
            // Unknown (query failed): assume present to avoid session pile-up.
            _ => true,
        }
    }

    /// Best-effort: kill persistent keepalive sessions so the distro can
    /// power off (WSL shuts the distro down when the last session ends).
    async fn kill_keepalive(&self) {
        let res = tokio::task::spawn_blocking(|| {
            let mut cmd = Command::new("powershell");
            cmd.args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Get-CimInstance Win32_Process -Filter \"Name='wsl.exe'\" | Where-Object { $_.CommandLine -like '*tail -f /dev/null*' } | ForEach-Object { Stop-Process -Id $_.ProcessId -Force }",
            ]);
            crate::no_window(cmd).output()
        })
        .await;
        if let Ok(Ok(out)) = res {
            if !out.status.success() {
                tracing::debug!("kill_keepalive powershell exited nonzero");
            }
        }
    }

    /// Launches the engine: boots WSL if needed (leaving a persistent
    /// session so the distro cannot power off mid-model-load) +
    /// `systemctl start ninfer`. Idempotent.
    pub async fn start(&self) -> Result<(), String> {
        if !self.keepalive_present().await {
            let distro = self.distro.clone();
            tokio::task::spawn_blocking(move || {
                let mut cmd = Command::new("wsl");
                cmd.args([
                    "-d",
                    distro.as_str(),
                    "-u", "root",
                    "--",
                    "bash", "-lc",
                    "systemctl start ninfer; exec tail -f /dev/null",
                ]);
                crate::no_window_detached(cmd).spawn()
            })
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| format!("spawn WSL keepalive failed: {e}"))?;
        }
        self.systemctl("start").await
    }

    /// Stops the engine, then releases the WSL distro (kills keepalive
    /// sessions so the distro powers off — clean state, no auto-start).
    pub async fn stop(&self) -> Result<(), String> {
        let r = self.systemctl("stop").await;
        self.kill_keepalive().await;
        r
    }

    pub async fn wait_ready(&self, timeout_s: u64) -> bool {
        let mut t = tokio::time::interval(tokio::time::Duration::from_secs(2));
        t.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        for _ in 0..timeout_s / 2 {
            t.tick().await;
            if self.health().await {
                return true;
            }
        }
        false
    }

    /// Health polling + JSONL tailing + /v1/models state (throttled),
    /// emitting telemetry events.
    pub fn spawn_telemetry(&self, tx: mpsc::Sender<TelemetryEvent>) {
        let base = self.base.clone();
        let jsonl_path = self.jsonl_path.clone();
        let client = self.client.clone();
        // Owned copy so the task can reuse models() (GET /v1/models).
        let ninfer = Self {
            base: self.base.clone(),
            distro: self.distro.clone(),
            jsonl_path: self.jsonl_path.clone(),
            client: self.client.clone(),
        };
        let models_interval = std::time::Duration::from_secs(2);
        tokio::spawn(async move {
            let mut last_up: Option<bool> = None;
            let mut tail: Option<tailer::Tailer> = None;
            let mut last_loaded: Vec<String> = Vec::new();
            let mut last_models_len: usize = 0;
            let mut last_models_poll: Option<tokio::time::Instant> = None;
            loop {
                let up = client
                    .get(format!("{base}/health"))
                    .send()
                    .await
                    .map(|r| r.status().as_u16() == 200)
                    .unwrap_or(false);

                if last_up != Some(up) {
                    last_up = Some(up);
                    // Up/down flip: forget stale model state and force a
                    // fresh /v1/models poll on the next up tick.
                    last_loaded.clear();
                    last_models_len = 0;
                    last_models_poll = None;
                    let ev = TelemetryEvent {
                        engine: NINFER_ENGINE.to_string(),
                        ts: now_ms(),
                        kind: if up {
                            EventKind::EngineUp {
                                engine: NINFER_ENGINE.to_string(),
                            }
                        } else {
                            EventKind::EngineDown {
                                engine: NINFER_ENGINE.to_string(),
                            }
                        },
                    };
                    if tx.send(ev).await.is_err() {
                        return;
                    }
                }

                if up {
                    // Loaded-model state: poll /v1/models at most every ~2s
                    // and emit ModelStates on change (mirrors Bionic).
                    let due = matches!(
                        last_models_poll,
                        Some(t) if t.elapsed() >= models_interval
                    );
                    if last_models_poll.is_none() || due {
                        last_models_poll = Some(tokio::time::Instant::now());
                        let models = ninfer.models().await;
                        let loaded: Vec<String> = models
                            .iter()
                            .filter(|m| m.loaded)
                            .map(|m| m.id.clone())
                            .collect();
                        if loaded != last_loaded || models.len() != last_models_len {
                            last_loaded = loaded.clone();
                            last_models_len = models.len();
                            let ev = TelemetryEvent {
                                engine: NINFER_ENGINE.to_string(),
                                ts: now_ms(),
                                kind: EventKind::ModelStates { loaded, models },
                            };
                            if tx.send(ev).await.is_err() {
                                return;
                            }
                        }
                    }

                    match &mut tail {
                        None => match tailer::Tailer::follow(&jsonl_path).await {
                            Ok(t) => tail = Some(t),
                            Err(e) => {
                                tracing::debug!(error = %e, "jsonl open failed, will retry")
                            }
                        },
                        Some(t) => {
                            // Truncation/rotation: re-read from start.
                            if let Ok(sz) = t.size().await {
                                if sz < t.pos() {
                                    match tailer::Tailer::from_start(&jsonl_path).await
                                    {
                                        Ok(nt) => tail = Some(nt),
                                        Err(_) => tail = None,
                                    }
                                    continue;
                                }
                            }
                            if let Some(t) = &mut tail {
                                match t.poll().await {
                                    Ok(lines) => {
                                        for l in lines {
                                            if let Some(ev) =
                                                jsonl::parse_ninfer_line(&l)
                                            {
                                                if tx.send(ev).await.is_err() {
                                                    return;
                                                }
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        tracing::debug!(
                                            error = %e,
                                            "jsonl read failed, will reopen"
                                        );
                                        tail = None;
                                    }
                                }
                            }
                        }
                    }
                } else {
                    tail = None;
                }

                tokio::time::sleep(tokio::time::Duration::from_millis(400)).await;
            }
        });
    }
}
