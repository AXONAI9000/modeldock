export default function StatusPill({ up }: { up: boolean }) {
  return (
    <span
      className={
        "inline-flex items-center gap-1.5 text-[11px] px-2 py-0.5 rounded-full border " +
        (up
          ? "border-emerald-500/40 bg-emerald-500/10 text-emerald-700"
          : "border-zinc-300 bg-zinc-100 text-zinc-500")
      }
    >
      <span
        className={
          "w-1.5 h-1.5 rounded-full " +
          (up ? "bg-emerald-500 animate-pulse" : "bg-zinc-400")
        }
      />
      {up ? "运行中" : "离线"}
    </span>
  );
}
