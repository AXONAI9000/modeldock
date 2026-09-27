#!/bin/bash
# ninfer 就绪探针（供 systemd ExecStartPost 调用）
# 语义：返回 0 = 引擎已监听并响应 /health；返回非 0 = 启动失败（systemd 会判为 failed）
# 冷启动基线：加载 20 GiB 权重约 50–60s，故超时给到 240s，避免"正常慢启动"被误杀。
#
# ⚠️ 本脚本运行时服务处于 activating（尚未 active），因此【禁止】用
#    `systemctl is-active --quiet ninfer` 判活——那会在启动瞬间误判并让 unit 直接 failed。
#    只轮询 /health；服务若中途死亡，curl 会持续失败，最终由 TimeoutStartSec(300s) 兜底。
set -u

URL="http://127.0.0.1:8080/health"
DEADLINE=$(( $(date +%s) + 240 ))

while [ "$(date +%s)" -lt "$DEADLINE" ]; do
    if curl -sf -m 5 -o /dev/null "$URL" 2>/dev/null; then
        echo "ninfer ready: $URL 于 $(date +%H:%M:%S) 响应"
        exit 0
    fi
    sleep 2
done

echo "ninfer 就绪探针超时（240s）：$URL 无响应" >&2
exit 1
