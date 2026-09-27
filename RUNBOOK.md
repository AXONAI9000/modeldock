# ModelDock M0 运行手册

M0 交付：**NInfer + Bionic 双引擎控制台**（Tauri 2 + Rust + React）。
遥测链路：`NInfer --request-log-jsonl` (JSONL v21) → `dock-core` tailer/解析器 → Hub 合并 → Tauri `telemetry` 事件 → WebUI（5Hz 合批）。

---

## 1. 启动 / 停止

| 操作 | 命令 |
|---|---|
| 启动 M0（推荐） | `D:\setup\codeProject\modeldock\target\release\modeldock.exe`（9.2MB，内嵌前端，无控制台，不依赖 Vite） |
| 启动 M0（调试） | `D:\setup\codeProject\modeldock\target\debug\modeldock.exe`（**必须先 `npm run dev` 起 Vite:5173**，debug 构建硬编码 devUrl） |
| 停止 M0 | 关闭窗口；或 `taskkill /IM modeldock.exe /F` |
| 重新构建 | 先 `D:\setup\nodehs\npm.cmd run build`（前端 → dist/），再 `D:\setup\tools\cargo\bin\cargo.exe build --release`（工作目录 `D:\setup\codeProject\modeldock`） |
| 无头验证 | `target\debug\examples\probe.exe`（环境变量 `PROBE_SECS=40`，每秒打印一行摘要） |
| 控制台日志 | 仅 debug 版有控制台；以 `Start-Process -RedirectStandardOutput modeldock-console.log` 启动 debug 版可查 emit/解析日志。每张引擎卡片有**启动/停止**按钮（`engine_start`/`engine_stop`，NInfer 启动含 90s 健康等待） |

> 环境变量（新 PowerShell 会话若未生效）：`RUSTUP_HOME=D:\setup\tools\rustup`、`CARGO_HOME=D:\setup\tools\cargo`，PATH 需含
> `D:\setup\tools\cargo\bin` 与 `D:\setup\tools\mingw64\mingw64\bin`（dlltool/windres 依赖，已写入用户 PATH）。

## 2. 遥测开关（systemd drop-in）

- 主 unit：`/etc/systemd/system/ninfer.service`（备份 `/root/ninfer.service.bak-20260927`）
- 遥测 drop-in：`/etc/systemd/system/ninfer.service.d/telemetry.conf`（追加 `--request-log-jsonl /var/log/ninfer/request-log.jsonl`）
- 就绪门 drop-in：`/etc/systemd/system/ninfer.service.d/readiness.conf`
  （`TimeoutStartSec=300` + `ExecStartPost=/usr/bin/bash /usr/local/sbin/ninfer-ready-probe.sh`）
- 日志文件：`\\wsl.localhost\Ubuntu-24.04\var\log\ninfer\request-log.jsonl`（Windows 侧 UNC 直读，tailer 处理截断/轮转）

```bash
# 关闭遥测（回滚）
wsl -d Ubuntu-24.04 -e sh -c 'rm -f /etc/systemd/system/ninfer.service.d/telemetry.conf && systemctl daemon-reload && systemctl restart ninfer'
# 恢复遥测
wsl -d Ubuntu-24.04 -e sh -c 'systemctl daemon-reload && systemctl restart ninfer'
```

### 2.1 冷启动语义（重要）

引擎冷启动要加载 20 GiB 权重，**约 50–60s** 才 `listening on :8080`。
加固后 `systemctl start|restart ninfer` 会**阻塞到就绪探针确认 /health 通过**才返回，
因此脚本无需再自己 sleep 轮询：

```powershell
# 阻塞至就绪（默认最多 300s）后返回 0；失败返回非 0
wsl -d Ubuntu-24.04 -u root -e systemctl restart ninfer
curl.exe -s --max-time 5 http://127.0.0.1:8080/health   # {"status":"ok"}
```

> ⚠️ **绝不要在冷加载的 ~50s 内因 /health 不通就重启** —— 那会清零加载进度，
> 形成"永远加载不完"的自毁循环（2026-09-27 那次 16–17s 一次的停止循环就是这个）。
> 另见 `D:\ninfer-setup\README.md`。


## 3. 引擎控制（M0 已内置于 UI 命令层，亦可手动）

| 引擎 | 启动 | 停止 | 状态 |
|---|---|---|---|
| NInfer (:8080) | `wsl -d Ubuntu-24.04 -e systemctl start ninfer` | `wsl -d Ubuntu-24.04 -e systemctl stop ninfer` | `curl :8080/health`、`/v1/models` |
| Bionic (:1234) | `Start-Process D:\setup\bionic\Bionic.exe` | `taskkill /F /IM Bionic.exe /T` | `curl :1234/api/v0/models` |

**ModelDock 按钮语义（NInfer）**：

- **启动** = 一键拉起 Ubuntu + NInfer：检测到无常驻 WSL 会话时，先以分离隐藏方式启动
  `wsl -d Ubuntu-24.04 -u root -- bash -lc "systemctl start ninfer; exec tail -f /dev/null"`
  （`tail -f /dev/null` 是常驻会话：WSL 在最后一个会话结束时会**整个关机发行版**，
  没有它的话 ninfer 会在模型加载中途被杀掉），再执行 `systemctl start ninfer`（幂等），
  最后 90s 健康等待。开机后 DSH/ModelDock 点这个按钮即可恢复全部服务。
- **停止** = `systemctl stop ninfer` + 杀掉所有 keepalive 会话 → WSL 发行版随之断电，
  回到干净状态（GPU 完全释放，无残留进程）。
- 开机自动启动：**`NInfer-WSL-Keepalive` 计划任务已启用**（登录时触发，幂等；
  作用是把常驻 `tail -f /dev/null` 会话拉起，防止 WSL 在最后一个会话结束时整个断电把 ninfer 一起杀掉）。
  `ninfer.service` 在 WSL 内仍是 enabled——WSL 起来后它自动跟起。
  旧的 `NInfer-WSL-Serve` 保持 Disabled，避免与保活任务重复 `systemctl start`。
  手动一键恢复：`Start-ScheduledTask -TaskName 'NInfer-WSL-Keepalive'`
  （或直接 `wsl -d Ubuntu-24.04 -u root -e systemctl start ninfer`，会阻塞到就绪）。

> ⚠️ NInfer 就是 DSH 会话后端：停止/重启期间 DSH 会不可用（启动需 ~30-60s 冷加载）。
> VRAM 硬约束：5090 32GB 只能容纳一个 27B 模型——加载第二个引擎前先停止另一个（M1 做预检）。

## 4. 关键路径 / 端口

| 项 | 值 |
|---|---|
| 工作区 | `D:\setup\codeProject\modeldock`（crate：`crates/dock-core`、`src-tauri`；2026-09-27 由 `begin` 改名而来） |
| Git | MinGit（不在 PATH）：`D:\setup\tools\git\cmd\git.exe`；仓库 `D:\setup\codeProject\modeldock`，remote `origin` → `https://github.com/AXONAI9000/modeldock`（public）。提交：`git add -A && git commit -m "..." && git push origin main`（推送需临时 PAT：助手脚本 `D:\setup\tools\gh-make-token.ps1` / `cdp-eval.ps1`，现建即删） |
| Rust 工具链 | GNU `stable-x86_64-pc-windows-gnu`（rustc 1.98.1）@ `D:\setup\tools\rustup` |
| cargo registry | `D:\setup\tools\cargo` |
| mingw binutils（dlltool/windres 运行时） | `D:\setup\tools\mingw64\mingw64\bin`（MSYS2 包，无安装器） |
| windres 预处理器 shim | `gcc.cmd` + `D:\setup\tools\shim\rcprep.mjs`（剥离 `#pragma` 后透传） |
| Node（便携） | `D:\setup\nodehs`（npm 11.19） |
| 模型 | `D:\models\qwen3_8_27b_nvfp4.ninfer`（NVFP4+MTP3，22.1GB，WSL 内 `/mnt/d/...`） |
| 端口 | 8080 NInfer · 1234/41343 Bionic · 5173 Vite dev · 3080 DSH |
| 基线 | NInfer ~128–198 tok/s（单请求）· Bionic Q4_K_M 预期 50–80 tok/s（未测） |

## 5. 事件 schema（实测 v21，解析器已对齐）

| 事件 | 关键字段（实测路径） |
|---|---|
| `server_start` | `engine.*`（`speculative_backend:"mtp"`, `draft_window:3`, `log_stats_interval_ms:5000`）、`memory.*` |
| `request_start` | `request.{protocol,stream,tool_count,requested_reasoning_effort}` |
| `request_done` | `result.{prompt_tokens,completion_tokens,finish_reason}` · `timings_seconds.{prepare,ttft,vision,prefill,decode,total}` · `speculative.{rounds,drafted_tokens,accepted_tokens,fallback_steps,accepted_per_position}` |
| `throughput`（5s 窗口） | `throughput_tokens_per_second.{decode,prefill}`（预计算）· `tokens.{committed_decode,computed_prefill}` · `decode_batch.average_size` · `scheduler.{running,waiting,prefilling,decode_ready,materializing,capture_pending,terminal_pending}` · `interval_seconds` |

首条原始事件由 `dock-core` 以 INFO 级别记录（truncated 1500 字符），便于将来 schema 升级时比对。

## 6. 已知限制 / M1 计划

- Bionic 无 tok/s 遥测（M1：解析 `/api/v0/chat/completions` 的 `stats.tokens_per_second` 做旁路估计）
- Bionic 模型加载/卸载走 `lms.exe`（`D:\setup\bionic\resources\app\.webpack-bionic\lms.exe`，`load/unload/ls/ps`）
- VRAM 冲突预检（加载前检查空闲显存 ≥ 模型尺寸 ×1.2）
- release 构建（`cargo build --release`，`windows_subsystem="windows"` 静默无控制台）
