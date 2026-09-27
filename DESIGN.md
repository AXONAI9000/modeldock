# ModelDock — 本地 LLM 引擎统一控制台 · 设计文档

> 版本: v1.0 · 2026-09-27 · 目标机器: Windows 11 + RTX 5090 (32GB) + WSL2 (Ubuntu-24.04)
> 工作代号: **ModelDock**（模型坞：所有本地引擎靠岸、统一调度）

---

## 1. 背景与目标

本机有两个本地推理引擎，各自为政：

1. **NInfer**（WSL2 内的 systemd 服务，端口 8080）：Qwen3.8-27B NVFP4 + MTP3，DSH 的主力模型。
2. **Bionic**（Electron 应用，内置 LM Studio 引擎，端口 1234）：GGUF 模型集合（Q4_K_M / Q5_K_M / Q6_K），DSH 里的 "LM Studio (本地)" 路由实际指向它。

**目标**：一个 Windows 桌面软件，做到：

- **F1 统一视图**：读取两个引擎的模型清单（文件、量化、大小、上下文、状态、最近使用）。
- **F2 统一管理**：一键启动/停止引擎；加载/卸载模型（含显存够不够的预检告警）。
- **F3 实时遥测**：模型当前 tok/s（大数字 + 走势图）、正在做什么（active/prefill/decode/waiting 状态、MTP 接受率、每请求明细）。
- **F4 高性能**：应用自身开销可忽略（目标 <200MB RSS），遥测不拖慢引擎。

非目标（MVP 不做）：多机/多卡、云端引擎、代理模式（Stretch 再议）、训练/微调。

---

## 2. 现状盘点（已实测验证的事实）

### 2.1 引擎与端口

| 项 | NInfer | Bionic |
|---|---|---|
| 运行位置 | WSL2 `Ubuntu-24.04`，systemd 服务 `ninfer.service`（enabled 自启） | Windows 进程（Electron，多进程），主 PID 持有 1234 + 41343 |
| 进程 | `/root/ninfer/build/apps/ninfer-serve /mnt/d/models/qwen3_8_27b_nvfp4.ninfer --max-context 240000 --kv-capacity 240000 --max-concurrency 2 --kv-dtype fp8 --device-state-slots 2 --host-state-slots 8 --host-kv-mib 8192 --spec mtp --draft-tokens 3 --lm-head-draft --preserve-thinking` | `D:\setup\bionic\Bionic.exe`（内置 `lms.exe`、`liblmstudio`、`node`、`deno` 运行时） |
| 端口 | `127.0.0.1:8080` | `127.0.0.1:1234`（LM Studio REST API + OpenAI 兼容） |
| 模型文件 | `D:\models\qwen3_8_27b_nvfp4.ninfer`（22.09 GiB） | `D:\models\**\*.gguf`（downloadsFolder = `D:\models`） |
| 已知模型 | 单模型常驻 | `bartowski/Qwen3.8-27B-Q4_K_M`、`lmstudio-community/Qwen3.8-27B-Q6_K`、`ukisai/Swift-1.5-Qwen3.8-27B-Q5_K_M`（完整清单以 `lms ls` 为准） |

### 2.2 控制通道（已验证可用）

| 操作 | NInfer | Bionic |
|---|---|---|
| 启动/停止引擎 | `wsl -d Ubuntu-24.04 -e systemctl start\|stop\|restart ninfer` | 启动 Bionic 进程；引擎随 Bionic 生命周期（或 `lms.exe server` 子命令） |
| 健康检查 | `GET /health` → `{"status":"ok"\|"unavailable"}` | `GET /api/v0/models`（列表+state）+ 进程存活 |
| 加载模型 | 不适用（单 artifact 常驻；换模型 = 改 unit 的 artifact 路径后 restart） | `lms.exe load <model-id>` |
| 卸载模型 | 不适用 | `lms.exe unload <model-id>` |
| 已加载模型 | `GET /v1/models` | `lms.exe ps` |
| 磁盘模型清单 | 固定 1 个 | `lms.exe ls` / 读 `model-index-cache.json`（JSON，无锁，安全读取；**不要读 bionic.sqlite —— 运行中被 WAL 锁**） |

`lms.exe` 完整能力（已验证 `--help`）：`chat / get / load / unload / ls / ps / import / server / log / link / runtime / clone / push / dev / login / logout / whoami`。
Bionic 内置 CLI 路径：`D:\setup\bionic\resources\app\.webpack-bionic\lms.exe`。

### 2.3 遥测数据源（核心设计依据）

**NInfer —— `--request-log-jsonl FILE`（官方文档 "Structured request log"）**

- 追加式 JSONL（schema v21），每行一个事件，实时 flush。
- 事件类型：
  - `server_start`：artifact、容量、GPU 环境、arena/Host 配置。
  - `request_start` / `request_rejected` / `request_error`：协议、采样器、thinking 模式、预算。
  - `request_done`：finish reason、prompt/completion/cache/computed-prefill token 数、**`timings_seconds {prepare, ttft, vision, prefill, decode, total}`**、**`speculative {backend, draft_window, rounds, drafted_tokens, accepted_tokens, fallback_steps, accepted_per_position}`**（MTP 接受率原始计数！）。
  - **`throughput`（默认每 5 秒一次）**：`prefill`/`decode` token 增量（decode 为已提交的 accepted 输出）、调度器快照 `running / prefilling / decode_ready / waiting / materializing / capture_pending / terminal_pending`、decode batch 统计、`host_work`（各阶段 Host/Device-wait 秒数）、`context_cache` 压力计数。完全空闲的区间会被省略。
- **当前服务未开启此标志** → 前置改动：给 `ninfer.service` 加一行 `--request-log-jsonl /var/log/ninfer/request-log.jsonl`（目录需预建），见 §9。
- 读取方式：Windows 侧经 WSL  UNC 路径 `\\wsl.localhost\Ubuntu-24.04\var\log\ninfer\request-log.jsonl` 直接 tail，无需在 WSL 里跑任何代理。
- 补充：每次 chat 响应自带 llama.cpp 风格 `timings`（`prompt_ms/predict_ms` 等）；流式可加 `timings_per_token`、`return_progress` —— 给"内置快速测试"功能用。

**Bionic / LM Studio**

- `GET /api/v0/models`：模型列表 + `state: loaded|not-loaded` + `quantization` + `max_context_length`（已验证）。
  - ⚠️ 实测该版本**不支持** `POST /api/v0/models/load|stop`（返回 "Unexpected endpoint or method"）→ 加载/卸载**必须走 `lms.exe` CLI**。
- `POST /api/v0/chat/completions`：响应含 `stats {tokens_per_second, time_to_first_token, generation_time, stop_reason}` + `model_info` + `runtime`（请求级精确速度，用于自检/基准）。
- 引擎日志：`C:\Users\Administrator\.lmstudio\apps\bionic\server-logs\YYYY-MM\*.log`（DEBUG 级记录每个请求；可 tail 出活动流）。
- 模型元数据：`.lmstudio\apps\bionic\.internal\model-index-cache.json`（displayName、量化、上下文、路径、gguf metadata）+ `model-data.json`（lastLoadedTimestamp → "最近使用"列）。

**GPU（全局）**

- `wsl -d Ubuntu-24.04 -e nvidia-smi --query-gpu=memory.used,memory.total,utilization.gpu,power.draw,temperature.gpu --format=csv,noheader,nounits`，1Hz。
- WSL2 与 Windows 共享同一 GPU，两边进程显存都会计入。

### 2.4 显存共存约束（产品必须处理的硬约束）

- 5090 = 32GB。NInfer 常驻 ≈ 20GB+（arena 19.7GiB + KV）；Bionic 侧 27B GGUF Q4_K_M ≈ 17GB、Q6_K ≈ 22GB。
- **两个 27B 同时加载必然 OOM** → 加载前必须做"够不够放"预检（当前 VRAM 已用 + 目标模型体积 + KV 余量 < 总容量），并在 UI 上给出"加载 X 前需先卸载 Y"的建议。

---

## 3. 功能定义（MVP）

### P0（必须有）

1. **引擎卡片（Dashboard）**：每引擎一张卡 —— 状态点（运行中/停止）、加载的模型、VRAM 占用条、**当前 decode tok/s 大数字**（5s 分辨率）、active/waiting 请求数、MTP 接受率（NInfer）、最近 10 分钟 tok/s 走势图。
2. **模型总表**：跨引擎合并列 —— 名称 / 引擎 / 文件 / 量化 / 大小 / 上下文 / 状态（loaded·idle·引擎停）/ 最近使用；行内操作：加载、卸载（引擎停止时置灰并提示）。
3. **引擎开关**：启动 / 停止 / 重启（NInfer: systemd；Bionic: 进程 + 引擎）。停止前有确认弹窗，提示"DSH 会话会失联"之类的副作用（引擎 ↔ 客户端映射表：8080←DSH 主力；1234←DSH 备用路由）。
4. **GPU 条**：显存 used/total、利用率、功率、温度（1Hz）。
5. **实时 tok/s**：NInfer 由 `throughput` 事件计算（decode 增量/interval）；Bionic 由 server-log 活动 + 请求 stats 估算（无 JSONL 级遥测，精度较低，UI 上如实标注）。

### P1（应该有）

6. **活动流（Activity）**：统一时间线 —— NInfer 的 `request_start/done`（协议、tokens 进出、ttft、decode 秒数、MTP rounds/接受率、finish reason）+ Bionic 引擎日志的请求记录；点击展开明细；最近 500 条环形缓冲，虚拟化列表。
7. **显存预检**：加载前计算并警示（§2.4）。
8. **内置快速基准**：对已加载引擎发一条固定 1024 token 计数请求，用返回的 `timings`/`stats` 显示精确 tok/s 与 MTP 接受率（复用本轮会话里实测过的方法）。

### P2（Stretch）

9. 客户端归属启发式标注（NInfer 请求带 `reasoning_effort`+tools 的记为 DSH，否则 Bionic/未知）。
10. 托盘常驻 + 开机自启 + 引擎自动拉起（`ninfer.service` 已 enabled；Bionic 可选）。
11. 代理模式：所有客户端经 ModelDock 转发，实现精确的 per-client tok/s（MVP 不做，架构上留接口）。
12. 多引擎扩展位（Ollama / vLLM 适配器）。

---

## 4. 总体架构

```
┌──────────────────────────────────────────────────────────────┐
│  ModelDock (Tauri 2 桌面应用, Windows)                       │
│                                                              │
│  ┌──────────────────────────┐   ┌──────────────────────────┐ │
│  │ Frontend (WebView2)      │   │ Rust core (tokio)        │ │
│  │ React 18 + TypeScript    │   │ ┌──────────────────────┐ │ │
│  │ shadcn/ui + Tailwind v4  │   │ │ EngineAdapter trait  │ │ │
│  │ uPlot 实时图表           │◄──┤ │  · NinferAdapter     │ │ │
│  │ TanStack Query + Zustand │Tauri│  · BionicAdapter     │ │ │
│  │ react-virtual 活动流     │ IPC │  · (OllamaAdapter…)  │ │ │
│  └──────────────────────────┘    │ └──────────────────────┘ │ │
│         渲染 ≤2-4Hz              │ TelemetryHub: 归一事件   │ │
│                                  │  + 环形缓冲(500) + 合流  │ │
│                                  │ 限流 ≤10Hz → emit       │ │
│                                  └──────────────────────────┘ │
└───────┬──────────────────────────┬───────────────────────────┘
        │ HTTP (reqwest, 轮询)      │ std::process
        │                          │  (wsl.exe / lms.exe)
   ┌────┴─────┐             ┌──────┴───────────────────────────┐
   │ NInfer   │  tail JSONL │ Windows 侧直读 WSL 文件系统:      │
   │ :8080    │◄────────────│ \\wsl.localhost\Ubuntu-24.04\    │
   └──────────┘             │   var\log\ninfer\request-log.*   │
                            │ lms.exe: ps/ls/load/unload/server│
                            │ Bionic: server-logs/*.log tail   │
                            │ nvidia-smi (1Hz)                 │
                            └──────────────────────────────────┘
```

### 4.1 核心抽象（Rust）

```rust
pub enum EngineId { Ninfer, Bionic }

#[derive(serde::Serialize)]
pub struct TelemetryEvent {
    pub engine: EngineId,
    pub ts: i64,                  // unix ms
    pub kind: EventKind,
}

pub enum EventKind {
    Throughput { decode_tps: f64, prefill_tps: f64,
                 running: u32, waiting: u32, avg_batch: f64,
                 mtp_accept_rate: Option<f64> },   // NInfer 5s 事件
    RequestStarted { protocol: String, stream: bool, tools: bool,
                     reasoning_effort: Option<String> },
    RequestFinished { prompt_tokens: u32, completion_tokens: u32,
                      ttft_s: f64, decode_s: f64,
                      mtp: Option<MtpStats>, finish_reason: String },
    ModelState { model: String, loaded: bool },
    Gpu { mem_used: u64, mem_total: u64, util: u32,
          power_w: f64, temp_c: u32 },
    EngineUp { engine: EngineId } / EngineDown { engine: EngineId } { ... },
}

#[async_trait]
pub trait EngineAdapter {
    async fn status(&self) -> Result<EngineStatus>;
    async fn list_models(&self) -> Result<Vec<ModelInfo>>;
    async fn start(&self) -> Result<()>;
    async fn stop(&self) -> Result<()>;
    async fn load_model(&self, id: &str) -> Result<()>;
    async fn unload_model(&self, id: &str) -> Result<()>;
    /// 后台任务: 持续产出 TelemetryEvent（JSONL tail / lms ps 轮询 / log tail）
    fn spawn_telemetry(&self, tx: tokio::sync::mpsc::Sender<TelemetryEvent>);
}
```

### 4.2 各遥测通道实现要点

| 通道 | 实现 | 频率/机制 |
|---|---|---|
| NInfer 吞吐 | tail `\\wsl.localhost\Ubuntu-24.04\var\log\ninfer\request-log.jsonl`（先 seek 到文件尾，之后只读增量；断行按 `\n` 切、半行缓存；`serde_json` 逐行解析） | 事件驱动（引擎 5s 一次） |
| NInfer 健康 | `GET :8080/health` | 1Hz（仅 200/503 翻转时 emit） |
| NInfer 启停 | `wsl -d Ubuntu-24.04 -e systemctl <verb> ninfer`（`std::process::Command`，超时 30s；启动后轮询 `/health` 直到 ok，最长 60s） | 用户触发 |
| Bionic 状态 | `GET :1234/api/v0/models`（state 字段）+ `Get-Process`/TaskSnapshot 查 Bionic 进程存活 | 2Hz |
| Bionic 加载/卸载 | `lms.exe ps / load / unload / ls`（同进程环境；输出按行解析，首版正则，后续若 CLI 有 `--json` 则切 JSON） | 用户触发 + 轮询确认 |
| Bionic 活动 | tail 最新 `server-logs\YYYY-MM\*.log`（文件名按日期滚动，需处理跨文件切换） | 事件驱动 |
| GPU | `nvidia-smi --query-gpu=... --format=csv,noheader,nounits` | 1Hz |
| 前端推送 | `TelemetryHub` 合流所有 adapter → 环形缓冲(500) → 以 ≤10Hz 批量 `emit("telemetry", batch)` | 背压：高频事件先聚合再发 |

---

## 5. 技术选型

### 5.1 框架：**Tauri 2**（[tauri-apps/tauri](https://github.com/tauri-apps/tauri)，111,408★）

| 候选 | 结论 | 理由 |
|---|---|---|
| **Tauri 2 + Rust core** | ✅ **采用** | 应用的全部"重量"是 HTTP 轮询 + 文件 tail + 子进程管理 —— 纯胶水，Rust 做最合适。壳是系统 WebView2（Win11 自带），安装体 ~5-10MB，自身 RSS 目标 <200MB；前端仍用 React 生态，开发成本接近 Electron |
| Electron + React | ✖ 备选 | 成熟但基线 ~150-300MB + 自带 Chromium ~100MB+；对本应用（无重前端渲染需求）是纯浪费；与"性能要好"直接冲突 |
| 原生 WinUI/WPF | ✖ | 实时图表与组件生态弱，迭代慢 |
| 纯 Web 本地服务 | ✖ | 用户要"软件"（可开机自启、托盘、不占浏览器）；且进程控制/文件 tail 放服务端反而要再写一层 |

**依赖（Rust core）**：`tokio`（异步运行时）、`reqwest`（HTTP + SSE）、`serde/serde_json`（逐行 JSONL）、`notify` 或手动 poll（文件 tail，WSL UNC 路径上 notify 兼容性需 M0 验证，不行就 250ms 增量读）、`sysinfo`/`windows` crate（Bionic 进程存活）、`tauri-plugin-autostart`、`tauri-plugin-single-instance`。

### 5.2 前端

| 项 | 选择 | 理由 |
|---|---|---|
| 框架 | React 18 + TypeScript + Vite | 生态、团队熟悉度 |
| 组件/样式 | **shadcn/ui**（[shadcn-ui/ui](https://github.com/shadcn-ui/ui)，124,622★）+ Tailwind CSS v4 | GitHub 上最流行的组件风格系统，暗色 zinc 基调 |
| 图表 | **uPlot**（~60KB） | 为高频时序数据而生，60fps；recharts/ECharts 在这个数据量下是负担。tok/s 走 1 样本/秒 的 10min 滚动窗 |
| 列表虚拟化 | @tanstack/react-virtual | 活动流 500 条不伤 DOM |
| 数据/状态 | TanStack Query（轮询/缓存/失效）+ Zustand（UI 状态、事件流订阅） | 分离服务端状态与本地状态 |
| 字体 | Inter（UI）+ JetBrains Mono（数字/日志） | Linear/Open WebUI 系标配，等宽数字对 tok/s 大数字观感关键 |

### 5.3 UI 风格（按 GitHub 热度选定）

- **风格基准：Open WebUI**（[open-webui/open-webui](https://github.com/open-webui/open-webui)，153,228★）—— GitHub 上星数最高的开源 LLM 界面，暗色、密集、卡片化、状态 pill 化，正是本域（本地模型管理）的最佳审美参照。
- **组件底座：shadcn/ui**（124,622★）—— Linear 式克制的暗色设计语言。
- **点缀**：magicui / aceternity 式微交互（hover 光晕、数字滚动、状态点呼吸动画）——只用三处：引擎状态点、tok/s 大数字、加载进度，避免花哨。

**色板与版式规格**：

```
背景     zinc-950  #09090b
面板     zinc-900/60 + 1px border zinc-800, rounded-xl, 内边距 16-20px
文字     zinc-100 / 次要 zinc-400
状态色   运行 emerald-400 · 加载/预热 amber-400 · 错误 rose-400 · 空闲 zinc-500
强调色   emerald-400（数字高亮、active 图表线）
图表线   emerald-400 1.5px，渐变填充 alpha 15%，网格 zinc-800/50
大数字   JetBrains Mono 28-40px tok/s；辅助指标 13px
布局     左栏 56px 图标导航(Open WebUI 式) | 主区: 顶部引擎卡片行 → GPU 条 →
         下两栏: [模型总表 | 活动流]；设置页独立
```

**四屏线框**：

```
┌─▮──────────────────────────────────────────────────────────────┐
│ ◉ Dashboard  ▢ Models  ▤ Activity  ⚙                          │
├──────────────────────────────────────────────┬─────────────────┤
│ ● NInfer  qwen3.8-27b (NVFP4+MTP3)          │ ● Bionic        │
│ 194.3 tok/s ▁▂▅▇▅▃▂▁  (decode, 5s)         │ Q4_K_M · not-loaded│
│ prefill 8,340 · active 1 · wait 0           │ 1234 ok · idle   │
│ MTP accept 76% · KV 240k                    │                  │
│ [⏹ 停止] [重启]        VRAM ██████░░ 21.4G  │ [加载] [停止Bionic]│
├──────────────────────────────────────────────┴─────────────────┤
│ GPU 5090   VRAM 23.1/32G · util 42% · 186W · 61°C              │
├──────────────────────────────────────────────┬─────────────────┤
│ 模型总表（跨引擎）                             │ 活动流(实时)      │
│ Qwen3.8-27B NVFP4 │NInfer │loaded │ 22.09G   │ 00:12:03 ✓ 8.2s │
│ Qwen3.8-27B Q4_K  │Bionic │ idle  │ 16.2G [加载]│ 1024 tok · 198t/s│
│ Qwen3.8-27B Q6_K  │Bionic │ idle  │ 21.9G [加载]│ ttft 0.9s · MTP 3.2/轮│
│ Swift-1.5 Q5_K_M  │Bionic │ idle  │ 17.8G [加载]│ 00:11:40 ✓ 3.1s│
└──────────────────────────────────────────────┴─────────────────┘
```

---

## 6. 性能设计（硬指标）

| 指标 | 目标 | 手段 |
|---|---|---|
| 冷启动 | <1s 到首屏 | Tauri 壳 + 静态前端；adapter 懒启动 |
| 应用自身 RSS | <200MB（不含 WebView2 系统进程） | 无 Electron；前端不做 60fps 动画 |
| 前端渲染频率 | ≤4Hz 有效更新 | 事件在 Rust 侧聚合（≤10Hz 批量 emit）；uPlot 自管理帧 |
| JSONL 解析 | 单行 <50µs，零全文件重读 | seek-to-end + 增量 read + 半行缓冲；只保留末 500 事件在内存 |
| 引擎侧开销 | 0（纯被动读取日志/HTTP GET） | 不注入代理、不改引擎代码；唯一改动是官方 `--request-log-jsonl` 标志 |
| 子进程轮询 | wsl/nvidia-smi 各 1Hz，单飞（前次未返回不重发） | 防 WSL 冷启动排队 |
| 内存曲线 | 活动流环形缓冲 500 条、图表 10min 定点长 | 无泄漏路径 |

---

## 7. 仓库结构（monorepo）

```
begin/
├─ crates/
│  └─ dock-core/                 # Rust core（可独立测试，不依赖 Tauri API）
│     └─ src/
│        ├─ lib.rs               # TelemetryHub, 事件模型
│        ├─ adapter/{ninfer,bionic}.rs
│        ├─ tailer.rs            # 通用增量文件 tailer（WSL UNC 兼容）
│        ├─ jsonl.rs             # 流式行解析
│        └─ gpu.rs               # nvidia-smi
├─ src/                          # Tauri 前端
│  ├─ components/{EngineCard,ModelTable,ActivityFeed,GpuBar,TpsChart}.tsx
│  ├─ pages/{Dashboard,Models,Activity,Settings}.tsx
│  └─ lib/{query.ts,events.ts,format.ts}
├─ src-tauri/                    # Tauri 壳（commands: start/stop/load/unload;
│                                #  events: telemetry batch）
├─ scripts/{update-ninfer-unit.ps1,smoke-test.ps1}
└─ docs/DESIGN.md (本文件)
```

`dock-core` 与 Tauri 解耦：CI 里可对 adapter 做单元测试（mock HTTP + 假 JSONL 文件）。

---

## 8. 里程碑

| 阶段 | 内容 | 验收标准 | 预估 |
|---|---|---|---|
| **M0** | Rust 骨架 + NInfer adapter（health/systemctl/JSONL tail）+ Dashboard（tok/s 大数字 + 走势图 + GPU 条） | DSH 在跑时，ModelDock 实时显示 5s 级 decode tok/s（与 `throughput` 事件一致）；启停 NInfer 后 60s 内状态翻转正确 | 1-2 天 |
| **M1** | Bionic adapter（lms ps/ls/load/unload、`/api/v0/models`、log tail）+ 模型总表 + 显存预检 | 在 UI 里加载/卸载 Q4_K_M，状态、VRAM、`lms ps` 三者一致；预检能正确拦截"两个 27B 同载" | 1-2 天 |
| **M2** | 活动流（双引擎统一时间线）+ 请求明细（ttft/decode 秒/MTP 接受率）+ 内置快速基准 | 时间线与引擎日志逐条对得上；基准结果与手动 curl 一致（±5%） | 1-2 天 |
| **M3** | 打磨：托盘/自启/设置页/installer、错误恢复（WSL 未启动、JSONL 缺失、Bionic 未运行 的降级 UI） | 断网级故障注入下不崩溃、状态自恢复 | 1 天 |
| Stretch | 客户端归属标注、代理模式、Ollama 适配器、多卡 | — | 另行规划 |

---

## 9. 前置改动（一次性）

**NInfer 开启遥测日志**（否则 `throughput`/`request_done` 数据源不存在）：

```bash
# 在 WSL Ubuntu-24.04 内
sudo mkdir -p /var/log/ninfer
sudo systemctl edit ninfer
```

写入：

```ini
[Service]
Environment="NINFER_REQUEST_LOG="
ExecStart=
ExecStart=/root/ninfer/build/apps/ninfer-serve /mnt/d/models/qwen3_8_27b_nvfp4.ninfer \
  --max-context 240000 --kv-capacity 240000 --max-concurrency 2 --kv-dtype fp8 \
  --device-state-slots 2 --host-state-slots 8 --host-kv-mib 8192 \
  --spec mtp --draft-tokens 3 --lm-head-draft --preserve-thinking \
  --request-log-jsonl /var/log/ninfer/request-log.jsonl
```

```bash
sudo systemctl daemon-reload
sudo systemctl restart ninfer
```

> 注意：`systemctl edit` 覆盖 ExecStart 需先写 `ExecStart=` 清行（见上）。改动后本 DSH 会话会短暂失联，属预期。
> 日志会持续增长（5s/行 ≈ 17k 行/天），M3 阶段在 app 里加"轮转保留 7 天"或 systemd `LogRotate`。

其余无前置：Bionic 侧全部走现成 CLI/HTTP；UNC 路径 `\\wsl.localhost\Ubuntu-24.04` 在 Win11 默认可用。

---

## 10. 风险与对策

| 风险 | 影响 | 对策 |
|---|---|---|
| WSL 发行版停止时 UNC 路径不可达 | NInfer tail 中断 | 错误降级为"引擎离线"；`spawn_telemetry` 带 5s 重连；UI 显示"WSL 未运行" |
| `notify` 在 `\\wsl.localhost` 上不稳 | 漏事件 | M0 即验证；fallback：250ms 增量读（开销可忽略，文件小） |
| `lms.exe` 输出格式变化 | 解析失败 | 首版宽松正则 + 失败时原样落日志；`lms ps` 返回空时回退 `/api/v0/models` |
| Bionic 更新换端口/路径 | 断连 | 端口/路径全部进设置页，默认值即当前实测值 |
| 两个 27B 同载 OOM | 引擎崩溃 | 加载前强制预检 + 显存余量 <4GB 时二次确认 |
| JSONL 日志膨胀 | 磁盘 | 7 天轮转（M3） |
| Bionic sqlite 锁 | 读崩溃 | 明确只读 JSON 注册表，永不碰 sqlite |
| WSL 冷启动时 systemctl 调用排队 | 操作卡顿 | 子进程单飞 + 30s 超时 + 操作按钮 loading 态 |

---

## 11. 关键决策记录（ADR 摘要）

1. **Tauri 2 而非 Electron** —— 胶水型应用，Rust core + 系统 WebView2，性能预算宽松达成。
2. **被动遥测优先，不做代理** —— NInfer 官方 JSONL 已提供引擎级 5s 吞吐 + 每请求完整计数，被动读取零侵入；代理模式留给 Stretch。
3. **Bionic 控制走 `lms.exe` CLI 而非 HTTP** —— 实测其 REST API 无 load/stop 端点，CLI 是官方且完整的通道。
4. **只读 Bionic 的 JSON 注册表** —— sqlite 运行时被 WAL 锁；JSON 文件是官方缓存且无锁。
5. **风格 = Open WebUI 基准 + shadcn/ui 底座** —— 两者分别是本域与组件层面 GitHub 星数最高的参照（153k★ / 124k★）。
6. **uPlot 而非 ECharts** —— 60KB、为 1Hz 流式数据设计，契合"性能要好"。
