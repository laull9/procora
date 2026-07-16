//! Windows 自启动命令的 UAC 提权桥接。

use std::{env, fs, path::Path, process::Command};

use anyhow::{Context, bail};
use uuid::Uuid;

/// 提权子进程写回成功结果时使用的前缀。
const SUCCESS_PREFIX: &str = "ok\n";
/// 提权子进程写回失败结果时使用的前缀。
const ERROR_PREFIX: &str = "error\n";

/// 通过 PowerShell `runas` 动词唤起 UAC，并返回提权子进程的结果文本。
pub(super) fn request(action: &str) -> anyhow::Result<String> {
    let executable = env::current_exe().context("无法确定当前 Procora 可执行文件")?;
    let result_path = env::temp_dir().join(format!("procora-uac-{}.result", Uuid::new_v4()));
    let _ = fs::remove_file(&result_path);

    let output = Command::new("powershell.exe")
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            elevation_script(),
        ])
        .env("PROCORA_UAC_EXECUTABLE", &executable)
        .env("PROCORA_UAC_ACTION", action)
        .env("PROCORA_UAC_RESULT", &result_path)
        .output()
        .context("无法启动 Windows UAC 提权请求")?;

    let result = fs::read_to_string(&result_path).ok();
    let _ = fs::remove_file(&result_path);
    if let Some(result) = result {
        return parse_result(&result);
    }
    if !output.status.success() {
        bail!("Windows 提权被拒绝或取消；请允许 UAC 请求后重试");
    }
    bail!("Windows 提权后的操作未返回结果；请确认当前账户具有管理员权限")
}

/// 把提权子进程的执行结果写入父进程指定的临时文件。
pub(super) fn write_result(
    path: &Path,
    result: &anyhow::Result<&'static str>,
) -> anyhow::Result<()> {
    let content = match result {
        Ok(message) => format!("{SUCCESS_PREFIX}{message}"),
        Err(error) => format!("{ERROR_PREFIX}{error:#}"),
    };
    fs::write(path, content).context("无法写回 Windows 提权操作结果")
}

/// 解析提权子进程写回的稳定结果协议。
fn parse_result(result: &str) -> anyhow::Result<String> {
    if let Some(message) = result.strip_prefix(SUCCESS_PREFIX) {
        return Ok(message.to_owned());
    }
    if let Some(message) = result.strip_prefix(ERROR_PREFIX) {
        bail!("Windows 提权后的操作失败：{message}");
    }
    bail!("Windows 提权操作返回了无法识别的结果")
}

/// 返回使用 `runas`、等待退出并隐藏辅助控制台的 PowerShell 脚本。
pub const fn elevation_script() -> &'static str {
    r#"$ErrorActionPreference = 'Stop'
$result = '"' + $env:PROCORA_UAC_RESULT + '"'
$arguments = @('__elevated-autostart', $env:PROCORA_UAC_ACTION, '--result', $result)
try {
    $process = Start-Process -FilePath $env:PROCORA_UAC_EXECUTABLE -ArgumentList $arguments -Verb RunAs -WindowStyle Hidden -Wait -PassThru
    exit $process.ExitCode
} catch {
    exit 1223
}"#
}
