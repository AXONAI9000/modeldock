#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

use std::sync::Arc;

use dock_core::{Bionic, GpuPoller, Hub, Ninfer, TelemetryEvent};
use tauri::{Emitter, Manager, State};

struct AppState {
    rt: tokio::runtime::Handle,
    hub: Arc<Hub>,
    ninfer: Arc<Ninfer>,
    bionic: Arc<Bionic>,
}

#[tauri::command]
fn engine_status(state: State<'_, AppState>) -> dock_core::Snapshot {
    state.hub.snapshot()
}

#[tauri::command]
async fn engine_start(state: State<'_, AppState>, engine: String) -> Result<(), String> {
    let rt = state.rt.clone();
    let ninfer = state.ninfer.clone();
    let bionic = state.bionic.clone();
    rt.spawn(async move {
        match engine.as_str() {
            "ninfer" => {
                ninfer.start().await?;
                if !ninfer.wait_ready(90).await {
                    return Err("NInfer 未在 90s 内就绪".into());
                }
            }
            "bionic" => bionic.start().await?,
            _ => return Err(format!("unknown engine: {engine}")),
        }
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn engine_stop(state: State<'_, AppState>, engine: String) -> Result<(), String> {
    let rt = state.rt.clone();
    let ninfer = state.ninfer.clone();
    let bionic = state.bionic.clone();
    rt.spawn(async move {
        match engine.as_str() {
            "ninfer" => ninfer.stop().await?,
            "bionic" => bionic.stop().await?,
            _ => return Err(format!("unknown engine: {engine}")),
        }
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

fn main() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("tokio runtime");

    let ninfer = Arc::new(Ninfer::default());
    let bionic = Arc::new(Bionic::default());
    let hub = Hub::new();

    // Adapter spawn helpers use tokio::spawn, which needs a runtime context.
    let _rt_guard = rt.enter();

    ninfer.spawn_telemetry(hub.tx().clone());
    bionic.spawn_telemetry(hub.tx().clone());
    GpuPoller::default().spawn(hub.tx().clone());
    hub.clone().run_forever();

    let hub_for_setup = hub.clone();
    let state = AppState {
        rt: rt.handle().clone(),
        hub: hub.clone(),
        ninfer: ninfer.clone(),
        bionic: bionic.clone(),
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
                let _ = w.set_focus();
            }
        }))
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            engine_status,
            engine_start,
            engine_stop
        ])
        .setup(move |app| {
            let handle = app.handle().clone();
            let hub = hub_for_setup.clone();
            // Coalesce telemetry batches and push to the webview at <=5 Hz.
            std::thread::spawn(move || {
                let mut rx = hub.subscribe();
                let mut batch: Vec<TelemetryEvent> = Vec::new();
                let mut total_emitted: u64 = 0;
                loop {
                    let mut drained = false;
                    loop {
                        match rx.try_recv() {
                            Ok(ev) => {
                                batch.push(ev);
                                drained = true;
                                if batch.len() >= 200 {
                                    break;
                                }
                            }
                            Err(tokio::sync::broadcast::error::TryRecvError::Empty) => break,
                            Err(tokio::sync::broadcast::error::TryRecvError::Lagged(n)) => {
                                tracing::warn!(lagged = n, "telemetry broadcast lagged");
                            }
                            Err(tokio::sync::broadcast::error::TryRecvError::Closed) => break,
                        }
                    }
                    if drained {
                        if batch.len() > 200 {
                            batch.truncate(200);
                        }
                        let n = batch.len();
                        match handle.emit("telemetry", std::mem::take(&mut batch)) {
                            Ok(()) => {
                                total_emitted += n as u64;
                                if total_emitted == 1 || total_emitted % 100 < (n as u64) {
                                    tracing::info!(
                                        events = n,
                                        total = total_emitted,
                                        "telemetry emitted to webview"
                                    );
                                }
                            }
                            Err(e) => tracing::error!(error = %e, "telemetry emit failed"),
                        }
                    }
                    std::thread::sleep(std::time::Duration::from_millis(200));
                }
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("run tauri application");
}
