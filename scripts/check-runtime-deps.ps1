# 校验 Windows 发布二进制不依赖 MSVC/UCRT 动态运行时。
param(
    [Parameter(Mandatory = $true)]
    [string]$Binary
)

$ErrorActionPreference = "Stop"

if (-not (Test-Path $Binary -PathType Leaf)) {
    throw "二进制不存在：$Binary"
}

# 查找 Visual Studio 随附的 dumpbin。
function Find-Dumpbin {
    $command = Get-Command dumpbin.exe -ErrorAction SilentlyContinue
    if ($command) {
        return $command.Source
    }

    $vswhere = Join-Path ${env:ProgramFiles(x86)} "Microsoft Visual Studio\Installer\vswhere.exe"
    if (-not (Test-Path $vswhere -PathType Leaf)) {
        throw "找不到 dumpbin.exe 或 vswhere.exe"
    }
    $installation = & $vswhere -latest -products * -property installationPath
    if (-not $installation) {
        throw "找不到 Visual Studio 安装目录"
    }
    $candidate = Get-ChildItem "$installation\VC\Tools\MSVC\*\bin\Host*\*\dumpbin.exe" |
        Select-Object -First 1
    if (-not $candidate) {
        throw "Visual Studio 安装中缺少 dumpbin.exe"
    }
    return $candidate.FullName
}

$dumpbin = Find-Dumpbin
$output = & $dumpbin /nologo /dependents $Binary
if ($LASTEXITCODE -ne 0) {
    throw "dumpbin 依赖检查失败"
}
$output | Write-Host

$dependencies = $output |
    Select-String '^\s+([A-Za-z0-9_.-]+\.dll)\s*$' |
    ForEach-Object { $_.Matches[0].Groups[1].Value.ToLowerInvariant() } |
    Sort-Object -Unique
$forbidden = $dependencies | Where-Object {
    $_ -match '^msvcp\d.*\.dll$' -or
    $_ -match '^vcruntime\d.*\.dll$' -or
    $_ -eq 'ucrtbase.dll' -or
    $_ -match '^api-ms-win-crt-.*\.dll$'
}
if ($forbidden) {
    throw "Windows 发布二进制仍依赖动态 C/C++ 运行时：$($forbidden -join ', ')"
}
