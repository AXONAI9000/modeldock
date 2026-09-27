import { useEffect, useRef } from "react";
import uplot from "uplot";

/** Live decode tok/s chart (uPlot), 10-minute rolling window. */
export default function TpsChart({
  history,
}: {
  history: [number, number][];
}) {
  const wrapRef = useRef<HTMLDivElement>(null);
  const upRef = useRef<uplot | null>(null);

  useEffect(() => {
    if (!wrapRef.current) return;
    const up = new uplot(
      {
        id: "tps",
        width: wrapRef.current.clientWidth || 600,
        height: 180,
        series: [
          { label: "decode tok/s" },
          {
            label: "tok/s",
            stroke: "#059669",
            width: 1.5,
            fill: "rgba(5,150,105,0.08)",
          },
        ],
        scales: {
          time: { time: true, distr: 2 },
          value: {
            range: (
              _self: uplot,
              _min: number,
              max: number,
              _key: string,
            ): [number, number] => [
              0,
              Math.max(50, Math.ceil((max ?? 100) * 1.15)),
            ],
          },
        },
        axes: [
          {
            stroke: "#d4d4d8",
            grid: { stroke: "rgba(228,228,231,0.7)" },
            font: "11px Inter, sans-serif",
            labelFont: "10px Inter, sans-serif",
          },
          {
            stroke: "#d4d4d8",
            grid: { stroke: "rgba(228,228,231,0.7)" },
            font: "11px 'JetBrains Mono', monospace",
            labelFont: "10px Inter, sans-serif",
          },
        ],
        cursor: { drag: { x: false, y: false } },
        legend: { show: true },
        padding: [8, 8, 0, 0],
      },
      [[], []],
      wrapRef.current,
    );
    upRef.current = up;
    const ro = new ResizeObserver(() => {
      if (wrapRef.current) {
        up.setSize({ width: wrapRef.current.clientWidth, height: 180 });
      }
    });
    ro.observe(wrapRef.current);
    return () => {
      ro.disconnect();
      up.destroy();
      upRef.current = null;
    };
  }, []);

  useEffect(() => {
    const up = upRef.current;
    if (!up) return;
    const now = Date.now();
    const data = history.filter(([t]) => now - t < 10 * 60 * 1000);
    if (data.length > 1) {
      up.setData([
        data.map((d) => d[0] / 1000),
        data.map((d) => d[1]),
      ]);
    }
  }, [history]);

  return (
    <div
      ref={wrapRef}
      className="rounded-xl border border-zinc-200 bg-white p-3 shadow-sm"
    />
  );
}
