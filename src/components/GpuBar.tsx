import type { GpuSample } from "../lib/telemetry";

export default function GpuBar({ gpu }: { gpu: GpuSample | null }) {
  const pct = gpu
    ? Math.round((gpu.mem_used_mb / Math.max(1, gpu.mem_total_mb)) * 100)
    : 0;
  return (
    <div className="rounded-xl border border-zinc-200 bg-white p-4 shadow-sm">
      <div className="flex items-center justify-between text-xs text-zinc-500 mb-2">
        <span>RTX 5090 · nvidia-smi @1Hz</span>
        <span className="font-num text-zinc-700">
          {gpu
            ? `${(gpu.mem_used_mb / 1024).toFixed(1)} / ${(
                gpu.mem_total_mb / 1024
              ).toFixed(1)} GB · ${gpu.util_pct}% util · ${gpu.power_w.toFixed(
                0,
              )} W · ${gpu.temp_c}°C`
            : "等待采样…"}
        </span>
      </div>
      <div className="h-2 rounded-full bg-zinc-200 overflow-hidden">
        <div
          className="h-full rounded-full bg-emerald-500 transition-all duration-500"
          style={{ width: `${pct}%` }}
        />
      </div>
    </div>
  );
}
