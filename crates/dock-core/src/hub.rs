use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tokio::sync::{broadcast, mpsc};

use crate::{
    BIONIC_ENGINE, EventKind, GpuSample, ModelInfo, TelemetryEvent, NINFER_ENGINE,
};

#[derive(Debug, Clone, Default, Serialize)]
pub struct ThroughputSnap {
    pub decode_tps: f64,
    pub prefill_tps: f64,
    pub decode_tokens: u64,
    pub prefill_tokens: u64,
    pub interval_s: f64,
    pub running: u32,
    pub waiting: u32,
    pub avg_batch: f64,
    pub mtp_accept_rate: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct RequestSnap {
    pub ts: i64,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub ttft_s: f64,
    pub decode_s: f64,
    pub decode_tps: f64,
    pub mtp_accept_rate: Option<f64>,
    pub finish_reason: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct EngineSnap {
    pub id: String,
    pub up: bool,
    pub loaded: Vec<String>,
    pub models: Vec<ModelInfo>,
    pub last_throughput: Option<ThroughputSnap>,
    pub last_request: Option<RequestSnap>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Snapshot {
    pub gpu: Option<GpuSample>,
    pub engines: Vec<EngineSnap>,
    pub recent: Vec<TelemetryEvent>,
    pub tps_history: Vec<(i64, f64)>,
}

impl Default for Snapshot {
    fn default() -> Self {
        Self {
            gpu: None,
            engines: vec![
                EngineSnap {
                    id: NINFER_ENGINE.into(),
                    up: false,
                    loaded: vec![],
                    models: vec![],
                    last_throughput: None,
                    last_request: None,
                },
                EngineSnap {
                    id: BIONIC_ENGINE.into(),
                    up: false,
                    loaded: vec![],
                    models: vec![],
                    last_throughput: None,
                    last_request: None,
                },
            ],
            recent: vec![],
            tps_history: vec![],
        }
    }
}

impl Snapshot {
    fn apply(&mut self, ev: &TelemetryEvent) {
        match &ev.kind {
            EventKind::Gpu { gpu } => self.gpu = Some(*gpu),
            EventKind::EngineUp { engine } => {
                if let Some(e) = self.engines.iter_mut().find(|x| x.id == *engine) {
                    e.up = true;
                }
            }
            EventKind::EngineDown { engine } => {
                if let Some(e) = self.engines.iter_mut().find(|x| x.id == *engine) {
                    e.up = false;
                    // Don't let the last loaded-model list outlive the engine.
                    e.loaded.clear();
                    e.models.clear();
                }
            }
            EventKind::ModelStates { loaded, models } => {
                if let Some(e) = self.engines.iter_mut().find(|x| x.id == ev.engine) {
                    e.loaded = loaded.clone();
                    e.models = models.clone();
                }
            }
            EventKind::Throughput {
                decode_tps,
                prefill_tps,
                decode_tokens,
                prefill_tokens,
                interval_s,
                running,
                waiting,
                avg_batch,
                mtp_accept_rate,
            } => {
                if let Some(e) = self.engines.iter_mut().find(|x| x.id == ev.engine) {
                    e.last_throughput = Some(ThroughputSnap {
                        decode_tps: *decode_tps,
                        prefill_tps: *prefill_tps,
                        decode_tokens: *decode_tokens,
                        prefill_tokens: *prefill_tokens,
                        interval_s: *interval_s,
                        running: *running,
                        waiting: *waiting,
                        avg_batch: *avg_batch,
                        mtp_accept_rate: *mtp_accept_rate,
                    });
                    if e.id == NINFER_ENGINE {
                        self.tps_history.push((ev.ts, *decode_tps));
                        if self.tps_history.len() > 1200 {
                            self.tps_history
                                .drain(..self.tps_history.len() - 1200);
                        }
                    }
                }
            }
            EventKind::RequestFinished {
                prompt_tokens,
                completion_tokens,
                ttft_s,
                decode_s,
                decode_tps,
                mtp,
                finish_reason,
                total_s: _,
            } => {
                if let Some(e) = self.engines.iter_mut().find(|x| x.id == ev.engine) {
                    e.last_request = Some(RequestSnap {
                        ts: ev.ts,
                        prompt_tokens: *prompt_tokens,
                        completion_tokens: *completion_tokens,
                        ttft_s: *ttft_s,
                        decode_s: *decode_s,
                        decode_tps: *decode_tps,
                        mtp_accept_rate: mtp.as_ref().map(|m| m.accept_rate),
                        finish_reason: finish_reason.clone(),
                    });
                }
            }
            EventKind::RequestStarted { .. } => {}
        }
    }
}

/// Merges all adapter streams: ring buffer + snapshot + broadcast to subscribers.
pub struct Hub {
    tx: mpsc::Sender<TelemetryEvent>,
    rx: Mutex<Option<mpsc::Receiver<TelemetryEvent>>>,
    broadcast: broadcast::Sender<TelemetryEvent>,
    ring: Mutex<VecDeque<TelemetryEvent>>,
    snap: Mutex<Snapshot>,
}

impl Hub {
    pub fn new() -> Arc<Self> {
        let (tx, rx) = mpsc::channel(1024);
        let (bc, _) = broadcast::channel(1024);
        Arc::new(Self {
            tx,
            rx: Mutex::new(Some(rx)),
            broadcast: bc,
            ring: Mutex::new(VecDeque::with_capacity(512)),
            snap: Mutex::new(Snapshot::default()),
        })
    }

    pub fn tx(&self) -> mpsc::Sender<TelemetryEvent> {
        self.tx.clone()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<TelemetryEvent> {
        self.broadcast.subscribe()
    }

    pub fn snapshot(&self) -> Snapshot {
        self.snap.lock().unwrap().clone()
    }

    /// Consume the merged stream forever (must be spawned once).
    pub fn run_forever(self: Arc<Self>) {
        let me = self.clone();
        tokio::spawn(async move {
            let mut rx = me
                .rx
                .lock()
                .unwrap()
                .take()
                .expect("run_forever called twice");
            while let Some(ev) = rx.recv().await {
                {
                    let mut r = me.ring.lock().unwrap();
                    r.push_back(ev.clone());
                    if r.len() > 500 {
                        r.pop_front();
                    }
                }
                {
                    let mut s = me.snap.lock().unwrap();
                    s.apply(&ev);
                    s.recent = me
                        .ring
                        .lock()
                        .unwrap()
                        .iter()
                        .rev()
                        .take(60)
                        .cloned()
                        .collect();
                }
                let _ = me.broadcast.send(ev);
            }
        });
    }
}
