#!/bin/bash
# 稳定性观察：3 分钟内引擎 PID 是否保持不变（不变=没有被杀）
for i in $(seq 1 10); do
  pid=$(systemctl show -p MainPID --value ninfer 2>/dev/null)
  age=$(ps -o etimes= -p "$pid" 2>/dev/null | tr -d ' ')
  code=$(curl -s -o /dev/null -w '%{http_code}' -m 4 http://127.0.0.1:8080/health 2>/dev/null)
  echo "$(date +%H:%M:%S) pid=$pid age=${age}s health=$code"
  sleep 15
done
