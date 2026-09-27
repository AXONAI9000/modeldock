//! Headless telemetry probe: runs for $PROBE_SECS (default 30) and prints one
//! summary line per second. Used to verify the pipeline without the GUI.

use dock_core::{Bionic, GpuPoller, Hub, Ninfer};
use std::time::Instant;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_target(false)
        .init();

    let secs: u64 = std::env::var("PROBE_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(30);

    let hub = Hub::new();
    Ninfer::default().spawn_telemetry(hub.tx().clone());
    Bionic::default().spawn_telemetry(hub.tx().clone());
    GpuPoller::default().spawn(hub.tx().clone());
    hub.clone().run_forever();

    let mut rx = hub.subscribe();
    let started = Instant::now();
    let mut last_print = Instant::now();

    loop {
        if started.elapsed().as_secs() >= secs {
            break;
        }
        let _ = tokio::time::timeout(
            std::time::Duration::from_millis(200),
            rx.recv(),
        )
        .await;
        if last_print.elapsed() >= std::time::Duration::from_secs(1) {
            last_print = Instant::now();
            let s = hub.snapshot();
            let el = started.elapsed();
            let mut parts: Vec<String> = Vec::new();
            for e in &s.engines {
                if e.id == "ninfer" {
                    let mut line = format!("ninfer up={} loaded={:?}", e.up, e.loaded);
                    if let Some(t) = &e.last_throughput {
                        line.push_str(&format!(
                            " | {} tok/s ({} tok/{}s) run={} wait={} batch={:.2}{}",
                            t.decode_tps,
                            t.decode_tokens,
                            t.interval_s,
                            t.running,
                            t.waiting,
                            t.avg_batch,
                            t.mtp_accept_rate
                                .map(|r| format!(" mtp={:.1}%", r * 100.0))
                                .unwrap_or_default(),
                        ));
                    }
                    parts.push(line);
                } else {
                    parts.push(format!(
                        "bionic up={} loaded={:?}",
                        e.up, e.loaded
                    ));
                }
            }
            if let Some(g) = &s.gpu {
                parts.push(format!(
                    "gpu {}/{}MB {}% {}W {}C",
                    g.mem_used_mb, g.mem_total_mb, g.util_pct, g.power_w, g.temp_c
                ));
            }
            println!("[{:02}:{:02}] {}", el.as_secs() / 60, el.as_secs() % 60, parts.join(" | "));
        }
    }
    println!("probe done");
}
