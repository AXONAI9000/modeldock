# 端到端验证：重启 ninfer，观察"加载中 → 就绪门放行 → active"
$ErrorActionPreference = 'SilentlyContinue'
"restart 触发时间: $(Get-Date -Format 'HH:mm:ss')"
wsl -d Ubuntu-24.04 -u root -e systemctl restart ninfer
for ($i = 1; $i -le 40; $i++) {
    Start-Sleep -Seconds 6
    $st = (wsl -d Ubuntu-24.04 -e systemctl is-active ninfer 2>$null) -join ''
    $sub = (wsl -d Ubuntu-24.04 -e systemctl show ninfer -p SubState --value 2>$null) -join ''
    $h = 'ERR'
    try { $h = (Invoke-WebRequest -Uri 'http://127.0.0.1:8080/health' -TimeoutSec 4 -UseBasicParsing).StatusCode } catch { $h = 'ERR' }
    "{0}  t={1,3}s  state={2}/{3}  health={4}" -f (Get-Date -Format 'HH:mm:ss'), ($i * 6), $st.Trim(), $sub.Trim(), $h
    if ($h -eq 200 -and $st.Trim() -eq 'active' -and $sub.Trim() -eq 'running') { ">>> 就绪门已放行，服务稳定 active"; break }
}
