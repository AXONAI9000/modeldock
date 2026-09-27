//! Defensive parser for `ninfer-serve --request-log-jsonl` (schema v21).
//!
//! Exact field names are resolved via candidate paths; the first raw line of
//! each event type is logged (truncated) so the real schema can be verified
//! against the probe output and candidates trimmed later.

use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::Value;

use crate::{now_ms, EventKind, MtpStats, TelemetryEvent, NINFER_ENGINE};

fn s(v: &Value, keys: &[&str]) -> Option<String> {
    for k in keys {
        if let Some(x) = v.get(k).and_then(|x| x.as_str()) {
            return Some(x.to_string());
        }
    }
    None
}

/// Walk a dotted path through a JSON object; return the leaf as f64.
fn dig(v: &Value, path: &[&str]) -> Option<f64> {
    let mut cur = v;
    for k in path {
        cur = cur.get(k)?;
    }
    cur.as_f64().or_else(|| cur.as_u64().map(|x| x as f64))
}

fn f64d(v: &Value, paths: &[&[&str]]) -> Option<f64> {
    for p in paths {
        if let Some(x) = dig(v, p) {
            return Some(x);
        }
    }
    None
}

fn trunc(s: &str) -> String {
    let mut out: String = s.chars().take(1500).collect();
    if s.chars().count() > 1500 {
        out.push_str("…[truncated]");
    }
    out
}

fn event_type(v: &Value) -> Option<String> {
    s(v, &["type", "event", "kind"])
}

const DECODE_TOKENS: &[&[&str]] = &[
    &["tokens", "committed_decode"],
    &["decode_tokens"],
    &["decode", "tokens"],
    &["decode_committed_tokens"],
    &["decode_committed"],
    &["decode"],
];
const PREFILL_TOKENS: &[&[&str]] = &[
    &["tokens", "computed_prefill"],
    &["prefill_tokens"],
    &["computed_prefill_tokens"],
    &["prefill", "tokens"],
    &["prefill_computed"],
    &["prefill"],
];
const TPS_DECODE: &[&[&str]] = &[&["throughput_tokens_per_second", "decode"]];
const TPS_PREFILL: &[&[&str]] = &[&["throughput_tokens_per_second", "prefill"]];
const PROMPT_TOKENS: &[&[&str]] = &[
    &["result", "prompt_tokens"],
    &["prompt_tokens"],
];
const COMPLETION_TOKENS: &[&[&str]] = &[
    &["result", "completion_tokens"],
    &["completion_tokens"],
];
const INTERVAL: &[&[&str]] = &[
    &["interval_seconds"],
    &["elapsed_seconds"],
    &["interval"],
    &["window_seconds"],
];
const RUNNING: &[&[&str]] = &[&["running"], &["scheduler", "running"]];
const WAITING: &[&[&str]] = &[&["waiting"], &["scheduler", "waiting"]];
const AVG_BATCH: &[&[&str]] = &[
    &["decode_batch", "average_size"],
    &["average_size"],
    &["avg_batch"],
    &["average_batch"],
];
const MTP_RATE: &[&[&str]] = &[
    &["mtp_acceptance"],
    &["mtp_accept_rate"],
    &["acceptance"],
    &["speculative", "acceptance"],
    &["speculative", "accepted_rate"],
    &["speculative", "accept_rate"],
];
const TIMINGS_TTFT: &[&[&str]] = &[
    &["timings_seconds", "ttft"],
    &["ttft_seconds"],
    &["ttft"],
];
const TIMINGS_DECODE: &[&[&str]] = &[
    &["timings_seconds", "decode"],
    &["decode_seconds"],
    &["decode"],
];
const TIMINGS_TOTAL: &[&[&str]] = &[
    &["timings_seconds", "total"],
    &["total_seconds"],
    &["total"],
];
const MTP_ROUNDS: &[&[&str]] = &[&["speculative", "rounds"], &["rounds"]];
const MTP_DRAFTED: &[&[&str]] = &[
    &["speculative", "drafted_tokens"],
    &["drafted_tokens"],
];

static LOGGED_THROUGHPUT: AtomicBool = AtomicBool::new(false);
static LOGGED_DONE: AtomicBool = AtomicBool::new(false);
static LOGGED_START: AtomicBool = AtomicBool::new(false);

/// Parse one JSONL line; None if not a recognized/necessary event.
pub fn parse_ninfer_line(line: &str) -> Option<TelemetryEvent> {
    let v: Value = serde_json::from_str(line).ok()?;
    let et = event_type(&v)?;
    let ts = f64d(&v, &[&["timestamp_unix_ms"], &["ts"], &["time_unix_ms"]])
        .map(|x| x as i64)
        .unwrap_or_else(now_ms);

    match et.as_str() {
        "throughput" => {
            if !LOGGED_THROUGHPUT.swap(true, Ordering::Relaxed) {
                tracing::info!(raw = %trunc(line), "first throughput event raw");
            }
            let interval_s = f64d(&v, INTERVAL).unwrap_or(5.0).max(0.5);
            let decode_tokens = f64d(&v, DECODE_TOKENS).unwrap_or(0.0) as u64;
            let prefill_tokens = f64d(&v, PREFILL_TOKENS).unwrap_or(0.0) as u64;
            let running = f64d(&v, RUNNING).unwrap_or(0.0) as u32;
            let waiting = f64d(&v, WAITING).unwrap_or(0.0) as u32;
            let avg_batch = f64d(&v, AVG_BATCH).unwrap_or(0.0);
            let mtp_accept_rate = match f64d(&v, MTP_RATE) {
                Some(r) if r <= 1.0 => Some(r),
                Some(r) => Some(r / 100.0),
                None => {
                    let accepted =
                        f64d(&v, &[&["speculative", "accepted_tokens"]]);
                    let drafted =
                        f64d(&v, &[&["speculative", "drafted_tokens"]]);
                    match (accepted, drafted) {
                        (Some(a), Some(d)) if d > 0.0 => Some(a / d),
                        _ => None,
                    }
                }
            };
            let (decode_tps, prefill_tps) =
                match (f64d(&v, TPS_DECODE), f64d(&v, TPS_PREFILL)) {
                    (Some(d), Some(p)) => (d, p),
                    _ => (
                        decode_tokens as f64 / interval_s,
                        prefill_tokens as f64 / interval_s,
                    ),
                };
            Some(TelemetryEvent {
                engine: NINFER_ENGINE.to_string(),
                ts,
                kind: EventKind::Throughput {
                    decode_tps,
                    prefill_tps,
                    decode_tokens,
                    prefill_tokens,
                    interval_s,
                    running,
                    waiting,
                    avg_batch,
                    mtp_accept_rate,
                },
            })
        }
        "request_done" => {
            if !LOGGED_DONE.swap(true, Ordering::Relaxed) {
                tracing::info!(raw = %trunc(line), "first request_done event raw");
            }
            let prompt_tokens = f64d(&v, PROMPT_TOKENS).unwrap_or(0.0) as u64;
            let completion_tokens = f64d(&v, COMPLETION_TOKENS).unwrap_or(0.0) as u64;
            let ttft_s = f64d(&v, TIMINGS_TTFT).unwrap_or(0.0);
            let decode_s = f64d(&v, TIMINGS_DECODE).unwrap_or(0.0);
            let total_s = f64d(&v, TIMINGS_TOTAL).unwrap_or(0.0);
            let result = v.get("result");
            let finish_reason = match result {
                Some(r) => {
                    s(r, &["finish_reason", "finish"]).unwrap_or_default()
                }
                None => s(&v, &["finish_reason", "finish"]).unwrap_or_default(),
            };
            let rounds = f64d(&v, MTP_ROUNDS).unwrap_or(0.0) as u64;
            let drafted = f64d(&v, MTP_DRAFTED).unwrap_or(0.0) as u64;
            let accepted =
                f64d(&v, &[&["speculative", "accepted_tokens"]])
                    .unwrap_or(0.0) as u64;
            let mtp = if rounds > 0 || drafted > 0 || accepted > 0 {
                Some(MtpStats {
                    rounds,
                    drafted,
                    accepted,
                    accept_rate: if drafted > 0 {
                        accepted as f64 / drafted as f64
                    } else {
                        0.0
                    },
                })
            } else {
                None
            };
            Some(TelemetryEvent {
                engine: NINFER_ENGINE.to_string(),
                ts,
                kind: EventKind::RequestFinished {
                    prompt_tokens,
                    completion_tokens,
                    ttft_s,
                    decode_s,
                    total_s,
                    decode_tps: if decode_s > 0.0 {
                        completion_tokens as f64 / decode_s
                    } else {
                        0.0
                    },
                    mtp,
                    finish_reason,
                },
            })
        }
        "request_start" => {
            if !LOGGED_START.swap(true, Ordering::Relaxed) {
                tracing::info!(raw = %trunc(line), "first request_start event raw");
            }
            let req = v.get("request");
            let tools = req
                .and_then(|r| r.get("tool_count"))
                .and_then(|x| x.as_f64())
                .unwrap_or(0.0)
                > 0.0
                || v
                    .get("tools")
                    .map(|t| match t {
                        Value::Array(a) => !a.is_empty(),
                        Value::Object(_) => true,
                        _ => false,
                    })
                    .unwrap_or(false);
            Some(TelemetryEvent {
                engine: NINFER_ENGINE.to_string(),
                ts,
                kind: EventKind::RequestStarted {
                    protocol: req
                        .and_then(|r| s(r, &["protocol"]))
                        .or_else(|| s(&v, &["protocol", "api"]))
                        .unwrap_or_default(),
                    stream: req
                        .and_then(|r| r.get("stream"))
                        .and_then(|x| x.as_bool())
                        .or_else(|| v.get("stream").and_then(|x| x.as_bool()))
                        .unwrap_or(false),
                    tools,
                    reasoning_effort: req
                        .and_then(|r| s(r, &["requested_reasoning_effort"]))
                        .or_else(|| {
                            s(&v, &["requested_reasoning_effort", "reasoning_effort"])
                        }),
                },
            })
        }
        // server_start / request_rejected / request_error: not needed for M0.
        _ => None,
    }
}
