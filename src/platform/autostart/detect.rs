//! 当前平台自启动定义的归属检测。

use super::{AutostartError, DaemonAutostart};

/// 判断 systemd 用户单元是否属于指定中心定义。
#[cfg(target_os = "linux")]
pub(super) fn is_enabled(definition: &DaemonAutostart) -> Result<bool, AutostartError> {
    use directories::BaseDirs;

    let path = BaseDirs::new()
        .map(|dirs| dirs.config_dir().join("systemd/user/procora.service"))
        .ok_or(AutostartError::MissingUserDirectory)?;
    definition_file_contains(&path, &definition.endpoint, "读取 systemd 用户单元")
}

/// 判断 `LaunchAgent` 是否属于指定中心定义。
#[cfg(target_os = "macos")]
pub(super) fn is_enabled(definition: &DaemonAutostart) -> Result<bool, AutostartError> {
    use directories::BaseDirs;

    let path = BaseDirs::new()
        .map(|dirs| {
            dirs.home_dir()
                .join("Library/LaunchAgents/dev.procora.center.plist")
        })
        .ok_or(AutostartError::MissingUserDirectory)?;
    definition_file_contains(&path, &definition.endpoint, "读取 LaunchAgent plist")
}

/// 判断 Windows 登录任务是否属于指定中心定义。
#[cfg(target_os = "windows")]
pub(super) fn is_enabled(definition: &DaemonAutostart) -> Result<bool, AutostartError> {
    let query = std::process::Command::new("schtasks.exe")
        .args(["/Query", "/TN", "Procora Center", "/V", "/FO", "LIST"])
        .output()
        .map_err(|source| AutostartError::Io {
            action: "查询 Windows 自启动任务",
            source,
        })?;
    Ok(query.status.success()
        && String::from_utf8_lossy(&query.stdout).contains(&definition.endpoint))
}

/// 不受支持的平台无法查询原生托管状态。
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub(super) const fn is_enabled(_definition: &DaemonAutostart) -> Result<bool, AutostartError> {
    Err(AutostartError::Unsupported)
}

/// 检查原生托管文件是否存在并包含当前中心的唯一端点。
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn definition_file_contains(
    path: &std::path::Path,
    endpoint: &str,
    action: &'static str,
) -> Result<bool, AutostartError> {
    if !path.is_file() {
        return Ok(false);
    }
    let content =
        std::fs::read_to_string(path).map_err(|source| AutostartError::Io { action, source })?;
    Ok(content.contains(endpoint))
}
