# ModelDock

本地 LLM 推理引擎统一控制台 —— 一个窗口管所有本地引擎（NInfer + Bionic）。
Tauri 2 + Rust + React，Windows + WSL。

> ModelDock（模型坞）：所有本地引擎靠岸、统一调度。

## 功能

- **实时吞吐**：tail NInfer 的 request-log JSONL（v21 schema），5s 窗口 decode tok/s 大数字 + 走势图（uplot）
- **模型状态**：轮询 NInfer `/v1/models` 与 Bionic `/api/v0/models`，变化时发 `ModelStates` 事件，卡片实时显示已加载模型
- **一键启停**
  - NInfer：拉起 WSL 常驻会话 + `systemctl start ninfer`（含 90s 健康等待）；停止时释放 WSL 发行版、彻底腾出显存
  - Bionic：启动/杀进程树
- **GPU 监控**：显存 / 利用率 / 功率 / 温度
- **请求级统计**：每请求 tok/s、TTFT、MTP 接受率

## 架构

```
NInfer (WSL :8080)  ── JSONL tail + /health + /v1/models ─┐
Bionic (:1234)      ── /api/v0/models 轮询 ────────────────┼──> dock-core Hub ──> Tauri telemetry 事件 (≤5Hz 合批) ──> WebUI
GPU (nvidia-smi)    ── 轮询 ───────────────────────────────┘
```

- `crates/dock-core` — 遥测核心：适配器（ninfer / bionic / gpu）、JSONL 解析器、文件 tailer、事件 Hub
- `src-tauri` — Tauri 2 壳：`engine_start` / `engine_stop` / `engine_status` 命令 + 事件推送
- `src/` — React UI：引擎卡片（EngineCard）、走势图（TpsChart）、GPU 条（GpuBar）
- `ninfer-ops/` — WSL 侧 systemd 遥测/就绪探针 drop-in 与运维脚本

## 构建与运行（Windows）

前置：GNU 工具链（`stable-x86_64-pc-windows-gnu`）、mingw binutils（dlltool/windres）、Node 18+。

```powershell
npm run build                    # 前端 -> dist/
cargo build --release            # -> target/release/modeldock.exe（内嵌前端，无控制台）
.\target\release\modeldock.exe
```

无头验证（不起 GUI 看遥测链路）：

```powershell
cargo build -p dock-core --example probe
$env:PROBE_SECS = 30
.\target\debug\examples\probe.exe
```

## 依赖与限制

- NInfer 需 WSL 发行版 Ubuntu-24.04 + `ninfer` systemd 服务；JSONL 遥测需 drop-in 追加 `--request-log-jsonl`（见 `ninfer-ops/`）
- VRAM 硬约束：32GB 单卡同时只能放一个 27B 模型——加载第二个引擎前先停另一个
- 运维手册：[RUNBOOK.md](RUNBOOK.md)
