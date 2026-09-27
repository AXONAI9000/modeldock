# D:\ninfer-setup 运维说明（2026-09-27 加固）

WSL2 里的 **NInfer**（`ninfer.service`，:8080）是本机 DSH 会话的模型后端。
本目录存放它的启动/等待/诊断脚本。以下记录 2026-09-27 那次"模型反复挂掉"的定因与加固。

---

## 1. 症状与定因

**症状**：`http://127.0.0.1:8080/health` 一直不通，看起来像"模型挂了"。

**实际根因（两条，叠加）**：

1. **冷启动需要 ~50s，但引擎被外部每 16–17 秒停一次**，永远加载不完。
   证据：`journalctl -u ninfer` 里 `Result=success`、`NRestarts=0`、无 OOM/panic，
   只有成对出现的 `Stopping ninfer.service` / `Deactivated successfully`：

   | 启动 | 停止 | 存活 |
   |---|---|---|
   | 12:28:11 | 12:28:28 | 17s |
   | 12:28:37 | 12:28:54 | 17s |
   | 12:29:02 | 12:29:19 | 17s |
   | 12:29:53 | 12:30:10 | 17s |

   权重加载到 ~20% 就被打断，永远走不到 `listening on http://127.0.0.1:8080`。
   unit 里 `Restart=no`（防 OOM 无限重启，见主 unit 注释），所以这些停止**全部来自外部主动指令**。

2. **WSL 在最后一个 `wsl.exe` 会话结束时会把整个发行版断电**，
   `systemd-logind` 打出 `The system will power off now!`，ninfer 被连坐杀死。
   11:52:08 就发生过一次整发行版关机。
   K 线证据：`/proc/uptime`（VM 内核）持续增长，但发行版用户态（PID 1）被反复重启
   —— 说明是 `wsl --terminate` 级别的用户态重启，不是 VM 重启。

> 次要：01:55–02:03 有 8 次**真·启动失败**：
> `FATAL ... requires 10108354835 bytes, but only 7.2GB available` —— 显存被其他进程占住。
> 加载前确认空闲显存 ≥ 22 GB（5090 32 GB 只放得下一个 27B 模型）。

---

## 2. 已实施的加固

### 2.1 就绪门 + 权威探针（WSL 内）

新增 drop-in `/etc/systemd/system/ninfer.service.d/readiness.conf`：

```ini
[Service]
TimeoutStartSec=300
ExecStartPost=/usr/bin/bash /usr/local/sbin/ninfer-ready-probe.sh
```

探针 `/usr/local/sbin/ninfer-ready-probe.sh`：轮询 `:8080/health`（最多 240s，每 2s 一次），
响应成功才返回 0。效果：

- `systemctl start ninfer` **会一直阻塞到引擎真正可服务**才返回（实测 13:24:xx → 13:25:19，约 50s）；
- 外部探测者看到的是明确的"还在 starting"，而不是"失败→重启"；
- 启动期间不会再被误判。

⚠️ 探针在 `activating` 阶段运行，**禁止**用 `systemctl is-active --quiet ninfer` 判活
（对 activating 返回非 0，会让 unit 直接 failed——这个坑已经踩过一次）。

源码在本目录：`readiness.conf`、`infer-ready-probe.sh`。

### 2.2 常驻 WSL 会话（保活）

计划任务 **`NInfer-WSL-Keepalive`**（已启用，登录时触发）：

```
wscript.exe "D:\ninfer-setup\ninfer-keepalive.vbs"
```

`infer-keepalive.vbs` 幂等：已存在 `tail -f /dev/null` 会话就直接退出；
否则隐藏窗口（style 0）启动
`wsl -d Ubuntu-24.04 -u root -- bash -lc "systemctl start ninfer; exec tail -f /dev/null"`。

另有一个**旧的、用途不明且同样会 `systemctl start`** 的任务 `NInfer-WSL-Serve`，
目前保持 Disabled 以免与保活任务重复触发。用 `Get-ScheduledTask NInfer-WSL-*` 查看。

---

## 3. 常用命令

```powershell
# 看服务是否真的很健康（唯一可信判据）
curl.exe -s --max-time 5 http://127.0.0.1:8080/health          # {"status":"ok"}
curl.exe -s --max-time 5 http://127.0.0.1:8080/v1/models

# 手动拉起（含常驻会话；幂等，冷加载到 ready 约 50s）
Start-ScheduledTask -TaskName 'NInfer-WSL-Keepalive'

# 干净重启（会阻塞到探针确认就绪）
wsl -d Ubuntu-24.04 -u root -e systemctl restart ninfer

# 停止（同时释放显存；保活会话仍在，发行版不会断电）
wsl -d Ubuntu-24.04 -u root -e systemctl stop ninfer

# 今天每次启停的时间线（复现问题时就靠它对齐时间）
wsl -d Ubuntu-24.04 -e sh /mnt/d/setup/codeProject/modeldock/diag-timeline.sh
```

**不要**在引擎冷加载期间（约 50s）把它当成"挂了"去重启 —— 这正是当初那个自毁循环。

---

## 4. 未决事项

- **触发那个 16–17 秒停止循环的客户端还没最终锁定**。它表现为固定间隔的
  `systemctl stop`→`start`（于 12:20:12–12:30:18 持续 10 分钟）。
  高度怀疑是某个用**短超时**轮询 `:8080/health`、失败即重启后端的看门狗/控制面板
  （我停掉自己的探测后循环立刻停止，随后引擎一次加载成功）。
  如果你在 ModelDock、DSH 路由或别的脚本里发现"探测失败自动重启"逻辑，
  请把超时提到 **≥120s** 并加指数退避，或直接去掉重启动作。
- `ninfer.service` 主 unit 与 `telemetry.conf`、`vision.conf` 三处 `ExecStart=` 重复定义，
  生效的是 `vision.conf`。建议合并成一份，避免误读实际参数。
- 不要恢复 `Restart=on-failure`，否则 OOM 时会变成无限重启。
