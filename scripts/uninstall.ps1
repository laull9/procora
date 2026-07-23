# 停止 Procora 自动托管并删除当前用户安装的命令。
$ErrorActionPreference = "Stop"
$installDir = if ($env:PROCORA_INSTALL_DIR) { $env:PROCORA_INSTALL_DIR } else { "$env:LOCALAPPDATA\Procora\bin" }
$binary = Join-Path $installDir "procora.exe"
$force = $env:PROCORA_FORCE_UNINSTALL -eq "1"

if (-not (Test-Path $binary -PathType Leaf)) {
    Write-Host "未找到 Procora：$binary"
    return
}

try {
    & $binary disable
    if ($LASTEXITCODE -ne 0) {
        throw "procora disable 退出码为 $LASTEXITCODE"
    }
} catch {
    if (-not $force) {
        throw "停用 Procora 开机自启动失败，尚未删除程序。确认无需保留后台托管后，可设置 PROCORA_FORCE_UNINSTALL=1 强制删除。`n$($_.Exception.Message)"
    }
    Write-Warning "未能停用开机自启动，正在强制删除程序：$($_.Exception.Message)"
}

Remove-Item $binary -Force
Write-Host "已删除 $binary"
Write-Host "运行状态、数据库和服务日志均已保留。"
