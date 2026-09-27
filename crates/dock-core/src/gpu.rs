use std::process::Command;

use tokio::sync::mpsc;

use crate::{now_ms, EventKind, GpuSample, TelemetryEvent, GPU_ENGINE};

/// Polls `nvidia-smi` inside WSL2 at 1 Hz (single-flight).
pub struct GpuPoller {
    distro: String,
}

impl Default for GpuPoller {
    fn default() -> Self {
        Self {
            distro: "Ubuntu-24.04".to_string(),
        }
    }
}

impl GpuPoller {
    pub fn spawn(self, tx: mpsc::Sender<TelemetryEvent>) {
        let distro = self.distro;
        tokio::spawn(async move {
            let mut pending: Option<tokio::task::JoinHandle<std::io::Result<String>>> =
                None;
            loop {
                if pending.is_none() {
                    let d = distro.clone();
                    pending = Some(tokio::task::spawn_blocking(move || {
                        let mut cmd = Command::new("wsl");
                        cmd.args([
                            "-d",
                            d.as_str(),
                            "-e",
                            "nvidia-smi",
                            "--query-gpu=memory.used,memory.total,utilization.gpu,power.draw,temperature.gpu",
                            "--format=csv,noheader,nounits",
                        ]);
                        crate::no_window(cmd)
                            .output()
                            .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
                    }));
                }
                let handle = pending.take().expect("pending is set");
                match handle.await {
                    Ok(Ok(out)) => {
                        let parts: Vec<&str> = out
                            .lines()
                            .next()
                            .unwrap_or("")
                            .split(',')
                            .map(|s| s.trim())
                            .collect();
                        if parts.len() >= 5 {
                            if let (Ok(used), Ok(total), Ok(util), Ok(power), Ok(temp)) = (
                                parts[0].parse::<u64>(),
                                parts[1].parse::<u64>(),
                                parts[2].parse::<u32>(),
                                parts[3].parse::<f64>(),
                                parts[4].parse::<u32>(),
                            ) {
                                let ev = TelemetryEvent {
                                    engine: GPU_ENGINE.to_string(),
                                    ts: now_ms(),
                                    kind: EventKind::Gpu {
                                        gpu: GpuSample {
                                            mem_used_mb: used,
                                            mem_total_mb: total,
                                            util_pct: util,
                                            power_w: power,
                                            temp_c: temp,
                                        },
                                    },
                                };
                                if tx.send(ev).await.is_err() {
                                    return;
                                }
                            }
                        }
                    }
                    Ok(Err(e)) => tracing::debug!(error = %e, "nvidia-smi failed"),
                    Err(e) => tracing::debug!(error = %e, "gpu poll task join failed"),
                }
                tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;
            }
        });
    }
}
