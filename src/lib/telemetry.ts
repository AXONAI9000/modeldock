import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

/**
 * Telemetry wire format — must match dock_core::EventKind
 * (serde internally-tagged: `kind` discriminator + flat payload fields).
 */
export interface ThroughputSnap {
  decode_tps: number;
  prefill_tps: number;
  decode_tokens: number;
  prefill_tokens: number;
  interval_s: number;
  running: number;
  waiting: number;
  avg_batch: number;
  mtp_accept_rate: number | null;
}

export interface MtpStats {
  rounds: number;
  drafted: number;
  accepted: number;
  accept_rate: number;
}

export interface GpuSample {
  mem_used_mb: number;
  mem_total_mb: number;
  util_pct: number;
  power_w: number;
  temp_c: number;
}

export interface ModelInfo {
  id: string;
  engine: string;
  quant: string;
  context: number;
  file: string;
  size_gib: number | null;
  loaded: boolean;
}

export interface RequestFinished {
  prompt_tokens: number;
  completion_tokens: number;
  ttft_s: number;
  decode_s: number;
  total_s: number;
  decode_tps: number;
  mtp: MtpStats | null;
  finish_reason: string;
}

export type EventKind =
  | ({ kind: "throughput" } & ThroughputSnap)
  | {
      kind: "request_started";
      protocol: string;
      stream: boolean;
      tools: boolean;
      reasoning_effort: string | null;
    }
  | ({ kind: "request_finished" } & RequestFinished)
  | { kind: "model_states"; loaded: string[]; models: ModelInfo[] }
  | { kind: "gpu"; gpu: GpuSample }
  | { kind: "engine_up"; engine: string }
  | { kind: "engine_down"; engine: string };

export interface TelemetryEvent {
  engine: string;
  ts: number;
  kind: EventKind;
}

export interface RequestSnap extends RequestFinished {
  ts: number;
}

export interface EngineSnap {
  id: string;
  up: boolean;
  loaded: string[];
  models: ModelInfo[];
  last_throughput: ThroughputSnap | null;
  last_request: RequestSnap | null;
}

export interface Snapshot {
  gpu: GpuSample | null;
  engines: EngineSnap[];
  recent: TelemetryEvent[];
  tps_history: [number, number][];
}

export function isTauri(): boolean {
  return (
    !!(window as any).__TAURI_INTERNALS__ || !!(window as any).__TAURI__
  );
}

function blankEngine(id: string): EngineSnap {
  return {
    id,
    up: false,
    loaded: [],
    models: [],
    last_throughput: null,
    last_request: null,
  };
}

interface Store {
  gpu: GpuSample | null;
  engines: Map<string, EngineSnap>;
  tps: [number, number][];
  recent: TelemetryEvent[];
}

/** Applies one wire event to the store (shared by live batches and hydration). */
function applyEvent(s: Store, e: TelemetryEvent): void {
  const eng = s.engines.get(e.engine);
  switch (e.kind.kind) {
    case "gpu":
      s.gpu = e.kind.gpu;
      break;
    case "engine_up":
      if (eng) eng.up = true;
      break;
    case "engine_down":
      if (eng) {
        eng.up = false;
        // Don't let the last loaded-model list outlive the engine.
        eng.loaded = [];
        eng.models = [];
      }
      break;
    case "model_states":
      if (eng) {
        eng.loaded = e.kind.loaded;
        eng.models = e.kind.models;
      }
      break;
    case "throughput":
      if (eng) eng.last_throughput = e.kind;
      if (e.engine === "ninfer") {
        s.tps.push([e.ts, e.kind.decode_tps]);
        if (s.tps.length > 1200) {
          s.tps.splice(0, s.tps.length - 1200);
        }
      }
      break;
    case "request_finished":
      if (eng) {
        eng.last_request = { ...e.kind, ts: e.ts };
      }
      break;
    case "request_started":
      break;
  }
  s.recent.push(e);
  if (s.recent.length > 300) {
    s.recent.splice(0, s.recent.length - 300);
  }
}

/**
 * Subscribes to the "telemetry" Tauri event (batches from the Rust core)
 * and re-renders at a fixed 4 Hz cadence regardless of event rate.
 */
export function useTelemetry(): { snapshot: Snapshot | null; live: boolean } {
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null);
  const [live, setLive] = useState(false);
  const store = useRef<Store>({
    gpu: null,
    engines: new Map([
      ["ninfer", blankEngine("ninfer")],
      ["bionic", blankEngine("bionic")],
    ]),
    tps: [],
    recent: [],
  });

  useEffect(() => {
    if (!isTauri()) return;
    let unlisten: (() => void) | undefined;
    let disposed = false;

    import("@tauri-apps/api/event").then(({ listen }) => {
      listen<TelemetryEvent[]>("telemetry", (ev) => {
        for (const e of ev.payload) {
          applyEvent(store.current, e);
        }
      }).then((u) => {
        if (disposed) {
          u();
        } else {
          unlisten = u;
          setLive(true);
        }
      }).catch((err) => {
        // Without a .catch this rejects silently and the pill hangs on
        // "遥测连接中…" forever. Surface it so ACL/IPC issues are visible.
        console.error("telemetry listen failed:", err);
      });
    });

    // Hydrate current state on mount: one-shot state-change events
    // (engine_up / model_states) can fire before the listener above
    // registers, so pull the hub's current snapshot once.
    invoke<Snapshot>("engine_status")
      .then((snap) => {
        if (disposed) return;
        const s = store.current;
        for (const e of snap.engines ?? []) {
          const eng = s.engines.get(e.id);
          if (!eng) continue;
          eng.up = e.up;
          eng.loaded = e.loaded ?? [];
          eng.models = e.models ?? [];
          eng.last_throughput = e.last_throughput ?? null;
          eng.last_request = e.last_request ?? null;
        }
        if (snap.gpu) s.gpu = snap.gpu;
        if (Array.isArray(snap.tps_history)) {
          s.tps = (snap.tps_history as [number, number][]).slice(-1200);
        }
        for (const ev of snap.recent ?? []) applyEvent(s, ev);
      })
      .catch((err) => {
        console.error("engine_status hydration failed:", err);
      });

    const iv = window.setInterval(() => {
      const s = store.current;
      setSnapshot({
        gpu: s.gpu,
        engines: [...s.engines.values()],
        recent: s.recent.slice(-60),
        tps_history: s.tps,
      });
    }, 250);

    return () => {
      disposed = true;
      unlisten?.();
      window.clearInterval(iv);
    };
  }, []);

  return { snapshot, live };
}
