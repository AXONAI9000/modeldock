import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import StatusPill from "./StatusPill";
import { isTauri, type EngineSnap } from "../lib/telemetry";

function Row({ k, v }: { k: string; v: string }) {
  return (
    <div className="flex justify-between gap-4">
      <dt className="text-zinc-500">{k}</dt>
      <dd className="font-num text-zinc-700">{v}</dd>
    </div>
  );
}

export default function EngineCard({
  title,
  id,
  engine,
  hint,
}: {
  title: string;
  id: string;
  engine: EngineSnap | undefined;
  hint: string;
}) {
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const up = engine?.up ?? false;
  const t = engine?.last_throughput ?? null;
  const loaded = engine?.loaded ?? [];

  const act = async (verb: "start" | "stop") => {
    if (!isTauri()) {
      setErr("仅 Tauri 环境可用（请通过 exe 运行）");
      return;
    }
    setBusy(true);
    setErr(null);
    try {
      await invoke(verb === "start" ? "engine_start" : "engine_stop", {
        engine: id,
      });
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(false);
    }
  };

  const btn =
    "text-[11px] px-2.5 py-1 rounded-md border transition-colors disabled:opacity-40 disabled:cursor-not-allowed";

  return (
    <div className="rounded-xl border border-zinc-200 bg-white p-4 space-y-3 shadow-sm">
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-2">
          <span className="text-sm font-semibold text-zinc-900">{title}</span>
          <StatusPill up={up} />
        </div>
        <span className="text-[11px] text-zinc-400 font-num">{hint}</span>
      </div>

      <div className="flex items-end gap-6">
        <div>
          <div className="text-[11px] text-zinc-500 mb-0.5">decode</div>
          <div className="font-num text-3xl leading-none text-emerald-600">
            {t ? t.decode_tps.toFixed(1) : "—"}
          </div>
          <div className="text-[10px] text-zinc-400 mt-1">tok/s（5s 窗口）</div>
        </div>
        {t && (
          <dl className="text-[12px] space-y-1 text-zinc-600">
            <Row k="active" v={String(t.running)} />
            <Row k="waiting" v={String(t.waiting)} />
            <Row k="batch" v={t.avg_batch.toFixed(2)} />
            {t.mtp_accept_rate != null && (
              <Row
                k="MTP 接受"
                v={(t.mtp_accept_rate * 100).toFixed(1) + "%"}
              />
            )}
          </dl>
        )}
      </div>

      <div className="flex gap-2 flex-wrap min-h-[22px]">
        {loaded.length === 0 ? (
          <span className="text-[11px] text-zinc-400">未加载模型</span>
        ) : (
          loaded.map((m) => (
            <span
              key={m}
              className="text-[11px] px-2 py-0.5 rounded-full bg-emerald-500/10 border border-emerald-500/30 text-emerald-700 font-num"
            >
              {m}
            </span>
          ))
        )}
      </div>

      <div className="flex items-center gap-2 pt-2 border-t border-zinc-200">
        <button
          onClick={() => act("start")}
          disabled={up || busy}
          className={
            btn +
            " border-emerald-500/50 bg-emerald-500/5 text-emerald-700 hover:bg-emerald-500/15"
          }
        >
          {busy && !up ? "启动中…" : "启动"}
        </button>
        <button
          onClick={() => act("stop")}
          disabled={!up || busy}
          className={
            btn +
            " border-red-400/60 bg-red-500/5 text-red-600 hover:bg-red-500/15"
          }
        >
          {busy && up ? "停止中…" : "停止"}
        </button>
        {err && (
          <span className="text-[11px] text-red-500 font-num truncate">
            {err}
          </span>
        )}
      </div>
    </div>
  );
}
