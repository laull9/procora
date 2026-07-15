# 从 GitHub Releases 安装当前 Windows 架构的 Procora 二进制。
$ErrorActionPreference = "Stop"
$repo = if ($env:PROCORA_REPO) { $env:PROCORA_REPO } else { "laull/procora" }
$version = if ($env:PROCORA_VERSION) { $env:PROCORA_VERSION } else { "latest" }
$installDir = if ($env:PROCORA_INSTALL_DIR) { $env:PROCORA_INSTALL_DIR } else { "$env:LOCALAPPDATA\Procora\bin" }

$architecture = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()
$target = switch ($architecture) {
    "X64" { "x86_64-pc-windows-msvc" }
    "Arm64" { "aarch64-pc-windows-msvc" }
    default { throw "不支持的处理器架构：$architecture" }
}
$asset = "procora-$target.zip"
$baseUrl = if ($version -eq "latest") {
    "https://github.com/$repo/releases/latest/download"
} else {
    "https://github.com/$repo/releases/download/$version"
}

$temporary = Join-Path ([System.IO.Path]::GetTempPath()) "procora-$([guid]::NewGuid())"
New-Item -ItemType Directory -Path $temporary | Out-Null
try {
    $archive = Join-Path $temporary $asset
    $checksum = "$archive.sha256"
    Invoke-WebRequest "$baseUrl/$asset" -OutFile $archive
    Invoke-WebRequest "$baseUrl/$asset.sha256" -OutFile $checksum
    $expected = (Get-Content $checksum -Raw).Split(" ", [System.StringSplitOptions]::RemoveEmptyEntries)[0]
    $actual = (Get-FileHash $archive -Algorithm SHA256).Hash
    if ($actual -ne $expected) { throw "下载文件 SHA-256 校验失败" }
    Expand-Archive $archive -DestinationPath $temporary -Force
    New-Item -ItemType Directory -Force -Path $installDir | Out-Null
    Copy-Item (Join-Path $temporary "procora.exe") (Join-Path $installDir "procora.exe") -Force
    Write-Host "Procora 已安装到 $installDir\procora.exe"
} finally {
    Remove-Item $temporary -Recurse -Force -ErrorAction SilentlyContinue
}
