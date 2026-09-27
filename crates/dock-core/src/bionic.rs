use std::process::Command;

use tokio::sync::mpsc;

use crate::{now_ms, EventKind, ModelInfo, TelemetryEvent, BIONIC_ENGINE};

pub struct Bionic {
    base: String,
    app_exe: String,
    lms_exe: String,
    client: reqwest::Client,
}

impl Default for Bionic {
    fn default() -> Self {
        Self {
            base: "http://127.0.0.1:1234".to_string(),
            app_exe: r"D:\setup\bionic\Bionic.exe".to_string(),
            lms_exe: r"D:\setup\bionic\resources\app\.webpack-bionic\lms.exe"
                .to_string(),
            client: reqwest::Client::new(),
        }
    }
}

impl Bionic {
    async fn api_models(&self) -> Option<serde_json::Value> {
        let resp = self
            .client
            .get(format!("{}/api/v0/models", self.base))
            .send()
            .await
            .ok()?;
        resp.json::<serde_json::Value>().await.ok()
    }

    pub async fn models(&self) -> Vec<ModelInfo> {
        let Some(v) = self.api_models().await else {
            return Vec::new();
        };
        let Some(data) = v.get("data").and_then(|d| d.as_array()) else {
            return Vec::new();
        };
        data.iter()
            .filter_map(|m| {
                Some(ModelInfo {
                    id: m.get("id")?.as_str()?.to_string(),
                    engine: BIONIC_ENGINE.to_string(),
                    quant: m
                        .get("quantization")
                        .and_then(|q| q.as_str())
                        .unwrap_or("?")
                        .to_string(),
                    context: m
                        .get("max_context_length")
                        .and_then(|x| x.as_u64())
                        .unwrap_or(0),
                    file: m
                        .get("containingDirAbsolutePath")
                        .and_then(|x| x.as_str())
                        .unwrap_or("")
                        .to_string(),
                    size_gib: None,
                    loaded: m.get("state").and_then(|s| s.as_str()) == Some("loaded"),
                })
            })
            .collect()
    }

    /// Launch the Bionic app (its embedded LM Studio engine comes with it).
    pub async fn start(&self) -> Result<(), String> {
        let exe = self.app_exe.clone();
        tokio::task::spawn_blocking(move || Command::new(&exe).spawn().map(|_| ()))
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| format!("spawn Bionic.exe failed: {e}"))
    }

    /// Kill the Bionic process tree (M0-level control; M1 may use a gentler path).
    pub async fn stop(&self) -> Result<(), String> {
        let out = tokio::task::spawn_blocking(|| {
            let mut cmd = Command::new("taskkill");
            cmd.args(["/F", "/IM", "Bionic.exe", "/T"]);
            crate::no_window(cmd).output()
        })
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        if out.status.success() || text.to_lowercase().contains("success") {
            Ok(())
        } else {
            Err(text.trim().to_string())
        }
    }

    /// `lms.exe <subcmd>...` helper (M1: load/unload/ls/ps).
    pub async fn lms(&self, subcmd: &[&str]) -> Result<String, String> {
        let exe = self.lms_exe.clone();
        let args: Vec<String> = subcmd.iter().map(|s| s.to_string()).collect();
        let out = tokio::task::spawn_blocking(move || {
            let mut cmd = Command::new(&exe);
            cmd.args(&args);
            crate::no_window(cmd).output()
        })
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
        let text = String::from_utf8_lossy(&out.stdout).to_string();
        if out.status.success() {
            Ok(text)
        } else {
            Err(format!(
                "lms {} failed: {}",
                subcmd.join(" "),
                String::from_utf8_lossy(&out.stderr)
            ))
        }
    }

    /// Poll /api/v0/models: engine up/down + loaded-model state changes.
    pub fn spawn_telemetry(&self, tx: mpsc::Sender<TelemetryEvent>) {
        let base = self.base.clone();
        let client = self.client.clone();
        tokio::spawn(async move {
            let mut last_up: Option<bool> = None;
            let mut last_loaded: Vec<String> = Vec::new();
            let mut last_models_len: usize = 0;
            loop {
                let v: Option<serde_json::Value> = async {
                    let resp = client
                        .get(format!("{base}/api/v0/models"))
                        .send()
                        .await
                        .ok()?;
                    resp.json::<serde_json::Value>().await.ok()
                }
                .await;
                let up = v.is_some();

                if last_up != Some(up) {
                    last_up = Some(up);
                    let ev = TelemetryEvent {
                        engine: BIONIC_ENGINE.to_string(),
                        ts: now_ms(),
                        kind: if up {
                            EventKind::EngineUp {
                                engine: BIONIC_ENGINE.to_string(),
                            }
                        } else {
                            EventKind::EngineDown {
                                engine: BIONIC_ENGINE.to_string(),
                            }
                        },
                    };
                    if tx.send(ev).await.is_err() {
                        return;
                    }
                }

                if up {
                    let (loaded, models) = match &v {
                        Some(vv) => {
                            let data = vv
                                .get("data")
                                .and_then(|d| d.as_array())
                                .cloned()
                                .unwrap_or_default();
                            let loaded: Vec<String> = data
                                .iter()
                                .filter(|m| {
                                    m.get("state").and_then(|s| s.as_str())
                                        == Some("loaded")
                                })
                                .filter_map(|m| {
                                    m.get("id")?.as_str().map(|s| s.to_string())
                                })
                                .collect();
                            let models: Vec<ModelInfo> = data
                                .iter()
                                .filter_map(|m| {
                                    Some(ModelInfo {
                                        id: m.get("id")?.as_str()?.to_string(),
                                        engine: BIONIC_ENGINE.to_string(),
                                        quant: m
                                            .get("quantization")
                                            .and_then(|q| q.as_str())
                                            .unwrap_or("?")
                                            .to_string(),
                                        context: m
                                            .get("max_context_length")
                                            .and_then(|x| x.as_u64())
                                            .unwrap_or(0),
                                        file: m
                                            .get("containingDirAbsolutePath")
                                            .and_then(|x| x.as_str())
                                            .unwrap_or("")
                                            .to_string(),
                                        size_gib: None,
                                        loaded: m.get("state").and_then(
                                            |s| s.as_str(),
                                        ) == Some("loaded"),
                                    })
                                })
                                .collect();
                            (loaded, models)
                        }
                        None => (Vec::new(), Vec::new()),
                    };
                    if loaded != last_loaded || models.len() != last_models_len {
                        last_loaded = loaded.clone();
                        last_models_len = models.len();
                        let ev = TelemetryEvent {
                            engine: BIONIC_ENGINE.to_string(),
                            ts: now_ms(),
                            kind: EventKind::ModelStates { loaded, models },
                        };
                        if tx.send(ev).await.is_err() {
                            return;
                        }
                    }
                }

                tokio::time::sleep(tokio::time::Duration::from_millis(2000)).await;
            }
        });
    }
}
