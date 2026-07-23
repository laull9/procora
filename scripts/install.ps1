# 从 GitHub Releases 安装当前 Windows 架构的 Procora 二进制。
$ErrorActionPreference = "Stop"
$repo = if ($env:PROCORA_REPO) { $env:PROCORA_REPO } else { "laull9/procora" }
$version = if ($env:PROCORA_VERSION) { $env:PROCORA_VERSION } else { "latest" }
$installDir = if ($env:PROCORA_INSTALL_DIR) { $env:PROCORA_INSTALL_DIR } else { "$env:LOCALAPPDATA\Procora\bin" }

if ($repo -notmatch '^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$') {
    throw "PROCORA_REPO 必须使用 owner/repo 格式"
}
if ($version -ne "latest" -and $version -notmatch '^[A-Za-z0-9._-]+$') {
    throw "PROCORA_VERSION 包含无效字符"
}

if ($PSVersionTable.PSEdition -eq "Desktop") {
    [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
}

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
    Write-Host "下载 $baseUrl/$asset"
    Invoke-WebRequest "$baseUrl/$asset" -OutFile $archive -UseBasicParsing
    Invoke-WebRequest "$baseUrl/$asset.sha256" -OutFile $checksum -UseBasicParsing

    $checksumContent = Get-Content $checksum -Raw
    $checksumMatch = [regex]::Match($checksumContent, '(?i)^\s*([0-9a-f]{64})(?:\s|$)')
    if (-not $checksumMatch.Success) {
        throw "SHA-256 校验文件格式无效"
    }
    $expected = $checksumMatch.Groups[1].Value
    $actual = (Get-FileHash $archive -Algorithm SHA256).Hash
    if ($actual -ne $expected) {
        throw "下载文件 SHA-256 校验失败"
    }

    Expand-Archive $archive -DestinationPath $temporary -Force
    $executable = Join-Path $temporary "procora.exe"
    if (-not (Test-Path $executable -PathType Leaf)) {
        throw "发布归档中缺少 procora.exe"
    }

    New-Item -ItemType Directory -Force -Path $installDir | Out-Null
    $destination = Join-Path $installDir "procora.exe"
    Copy-Item $executable $destination -Force
    Write-Host "Procora 已安装到 $destination"

    $pathEntries = $env:PATH -split ';'
    if ($installDir -notin $pathEntries) {
        Write-Host "提示：请把 $installDir 加入 PATH。"
    }
} finally {
    Remove-Item $temporary -Recurse -Force -ErrorAction SilentlyContinue
}
