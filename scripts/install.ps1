# 从 GitHub Releases 安装当前 Windows 架构的 Procora 二进制。
$ErrorActionPreference = "Stop"
$repo = if ($env:PROCORA_REPO) { $env:PROCORA_REPO } else { "laull9/procora" }
$version = if ($env:PROCORA_VERSION) { $env:PROCORA_VERSION } else { "latest" }
$installDir = if ($env:PROCORA_INSTALL_DIR) { $env:PROCORA_INSTALL_DIR } else { "$env:LOCALAPPDATA\Procora\bin" }
$githubMirror = if ($env:PROCORA_GITHUB_MIRROR) { $env:PROCORA_GITHUB_MIRROR.Trim() } else { "" }
$downloadCommand = if ($env:PROCORA_DOWNLOAD_COMMAND) { $env:PROCORA_DOWNLOAD_COMMAND } else { "" }

if ($repo -notmatch '^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$') {
    throw "PROCORA_REPO 必须使用 owner/repo 格式"
}
if ($version -ne "latest" -and $version -notmatch '^[A-Za-z0-9._-]+$') {
    throw "PROCORA_VERSION 包含无效字符"
}
if ($githubMirror -and ($githubMirror -notmatch '^https://' -or $githubMirror -match '\s')) {
    throw "PROCORA_GITHUB_MIRROR 必须是无空白的 HTTPS 前缀或包含 {url} 的模板"
}
if ($downloadCommand -and -not (Get-Command $downloadCommand -ErrorAction SilentlyContinue)) {
    throw "找不到 PROCORA_DOWNLOAD_COMMAND：$downloadCommand"
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
    # 使用镜像前缀或模板改写 GitHub 地址。
    function Resolve-ProcoraDownloadUrl([string]$Url) {
        if (-not $githubMirror) {
            return $Url
        }
        if ($githubMirror.Contains("{url}")) {
            return $githubMirror.Replace("{url}", $Url)
        }
        return "$($githubMirror.TrimEnd('/'))/$Url"
    }

    # 使用内置客户端或 URL/输出路径双参数下载程序获取文件。
    function Receive-ProcoraFile([string]$Url, [string]$Destination) {
        $resolvedUrl = Resolve-ProcoraDownloadUrl $Url
        Write-Host "下载 $resolvedUrl"
        if ($downloadCommand) {
            & $downloadCommand $resolvedUrl $Destination
            if (-not $? -or ($null -ne $LASTEXITCODE -and $LASTEXITCODE -ne 0)) {
                throw "下载命令失败：$resolvedUrl"
            }
        } else {
            Invoke-WebRequest $resolvedUrl -OutFile $Destination -UseBasicParsing
        }
    }

    $archive = Join-Path $temporary $asset
    $checksum = "$archive.sha256"
    Receive-ProcoraFile "$baseUrl/$asset" $archive
    Receive-ProcoraFile "$baseUrl/$asset.sha256" $checksum

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
    $python = Get-Command python -ErrorAction SilentlyContinue
    if ($python) {
        & $destination python install --interpreter $python.Source --quiet
        if ($LASTEXITCODE -eq 0) {
            Write-Host "Procora Python API 已安装到当前用户的 Python 3 环境"
        } else {
            Write-Warning "Procora 已安装，但 Python API 自动安装失败；可稍后运行 procora python install。"
        }
    } else {
        Write-Host "提示：未找到 Python 3；安装后可运行 procora python install 启用 Python API。"
    }
    Write-Host "Procora 已安装到 $destination"

    $pathEntries = $env:PATH -split ';'
    if ($installDir -notin $pathEntries) {
        Write-Host "提示：请把 $installDir 加入 PATH。"
    }
} finally {
    Remove-Item $temporary -Recurse -Force -ErrorAction SilentlyContinue
}
