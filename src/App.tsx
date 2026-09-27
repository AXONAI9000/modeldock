import { useTelemetry, isTauri } from "./lib/telemetry";
import EngineCard from "./components/EngineCard";
import GpuBar from "./components/GpuBar";
import TpsChart from "./components/TpsChart";

const NAV = [
  { label: "Dashboard", icon: "◉" },
  { label: "Models", icon: "▤" },
  { label: "Activity", icon: "≣" },
  { label: "Settings", icon: "⚙" },
];

export default function App() {
  const { snapshot, live } = useTelemetry();
  const engines = snapshot?.engines ?? [];
  const ninfer = engines.find((e) => e.id === "ninfer");
  const bionic = engines.find((e) => e.id === "bionic");

  const reqFinished = (snapshot?.recent ?? [])
    .filter((e) => e.kind.kind === "request_finished")
    .slice()
    .reverse();

  return (
    <div className="flex h-full">
      <aside className="w-14 shrink-0 border-r border-zinc-200 bg-white flex flex-col items-center py-3">
        <div className="w-9 h-9 mb-4 grid place-items-center rounded-lg bg-emerald-500/10 border border-emerald-500/30 font-num text-emerald-700">
          M
        </div>
        {NAV.map((n, i) => (
          <button
            key={n.label}
            title={n.label}
            className={
              "w-10 h-10 mb-1 grid place-items-center rounded-lg text-base transition-colors " +
              (i === 0
                ? "bg-zinc-100 text-zinc-900"
                : "text-zinc-400 hover:text-zinc-700 hover:bg-zinc-100")
            }
          >
            {n.icon}
          </button>
        ))}
      </aside>

      <main className="flex-1 overflow-y-auto p-5 space-y-4">
        <header className="flex items-center justify-between">
          <div>
            <h1 className="text-lg font-semibold tracking-tight leading-none text-zinc-900">
              ModelDock
            </h1>
            <p className="text-xs text-zinc-500 mt-1">
              本地推理引擎控制台 · NInfer + Bionic · M0
            </p>
          </div>
          <span
            className={
              "text-[11px] px-2.5 py-1 rounded-full border " +
              (live
                ? "border-emerald-500/40 bg-emerald-500/10 text-emerald-700"
                : "border-amber-500/50 bg-amber-500/10 text-amber-700")
            }
          >
            {live
              ? "实时遥测已连接"
              : isTauri()
                ? "遥测连接中…"
                : "非 Tauri 环境（请 cargo tauri dev 运行）"}
          </span>
        </header>

        <div className="grid grid-cols-2 gap-4">
          <EngineCard
            title="NInfer"
            id="ninfer"
            hint="qwen3.8-27b · NVFP4+MTP3 · :8080"
            engine={ninfer}
          />
          <EngineCard
            title="Bionic"
            id="bionic"
            hint="LM Studio 引擎 · GGUF · :1234"
            engine={bionic}
          />
        </div>

        <TpsChart history={snapshot?.tps_history ?? []} />
        <GpuBar gpu={snapshot?.gpu ?? null} />

        <section className="rounded-xl border border-zinc-200 bg-white p-4 shadow-sm">
          <h2 className="text-sm font-semibold text-zinc-900 mb-3">
            最近请求（NInfer request_done）
          </h2>
          {reqFinished.length === 0 ? (
            <p className="text-xs text-zinc-400">
              尚无完成的请求。随便让 DSH 生成一段内容即可看到明细。
            </p>
          ) : (
            <div className="overflow-x-auto">
              <table className="w-full text-[12px] font-num">
                <thead>
                  <tr className="text-zinc-500 text-left">
                    <th className="py-1 pr-3 font-normal">时间</th>
                    <th className="pr-3 font-normal">prompt</th>
                    <th className="pr-3 font-normal">completion</th>
                    <th className="pr-3 font-normal">ttft</th>
                    <th className="pr-3 font-normal">decode</th>
                    <th className="pr-3 font-normal">tok/s</th>
                    <th className="pr-3 font-normal">MTP</th>
                    <th className="font-normal">finish</th>
                  </tr>
                </thead>
                <tbody>
                  {reqFinished.slice(0, 20).map((e) => {
                    const k = e.kind;
                    if (k.kind !== "request_finished") return null;
                    return (
                      <tr
                        key={`${e.ts}-${k.finish_reason}`}
                        className="border-t border-zinc-200 text-zinc-700"
                      >
                        <td className="py-1 pr-3 text-zinc-400">
                          {new Date(e.ts).toLocaleTimeString("zh-CN", {
                            hour12: false,
                          })}
                        </td>
                        <td className="pr-3">{k.prompt_tokens}</td>
                        <td className="pr-3">{k.completion_tokens}</td>
                        <td className="pr-3">{k.ttft_s.toFixed(2)}s</td>
                        <td className="pr-3">{k.decode_s.toFixed(2)}s</td>
                        <td className="pr-3 text-emerald-600">
                          {k.decode_tps.toFixed(1)}
                        </td>
                        <td className="pr-3">
                          {k.mtp
                            ? (k.mtp.accept_rate * 100).toFixed(1) + "%"
                            : "—"}
                        </td>
                        <td>{k.finish_reason || "—"}</td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
            </div>
          )}
        </section>
      </main>
    </div>
  );
}
