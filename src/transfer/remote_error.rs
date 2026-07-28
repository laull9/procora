use std::process::ExitStatus;

use anyhow::anyhow;

/// SSH 连接失败后允许的安全交互回退。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum LoginFailure {
    None,
    Authentication,
    HostKey,
}

/// 把 SSH 退出状态和远端错误转换成可操作提示。
pub(super) fn process_error(status: ExitStatus, stderr: &[u8], remote_bin: &str) -> anyhow::Error {
    let message = clean_remote_error(&String::from_utf8_lossy(stderr));
    let detail = if message.is_empty() {
        status.to_string()
    } else {
        message
    };
    if remote_command_missing(status.code(), &detail) {
        anyhow!(
            "远端无法启动 `{remote_bin}`：{detail}；可尝试 `--remote-bin ~/.local/bin/procora`，Windows 可使用 `--remote-bin C:/Tools/procora.exe`"
        )
    } else {
        anyhow!("SSH 上传失败：{detail}")
    }
}

/// 去掉远端 CLI 的外层错误标签与帮助尾注，避免本机重复输出。
fn clean_remote_error(message: &str) -> String {
    let message = message.trim();
    let message = message.strip_prefix("错误：").unwrap_or(message);
    message
        .lines()
        .take_while(|line| line.trim() != "运行 `procora --help` 查看用法。")
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_owned()
}

/// 区分允许交互处理的认证失败、未知主机与其他连接故障。
pub(super) fn classify_login_failure(status_code: Option<i32>, stderr: &[u8]) -> LoginFailure {
    if status_code != Some(255) {
        return LoginFailure::None;
    }
    let message = String::from_utf8_lossy(stderr).to_ascii_lowercase();
    if message.contains("remote host identification has changed")
        || message.contains("offending host key")
    {
        return LoginFailure::None;
    }
    if message.contains("host key verification failed")
        || (message.contains("host key is known") && message.contains("strict checking"))
    {
        return LoginFailure::HostKey;
    }
    if message.contains("permission denied")
        || message.contains("authentication failed")
        || message.contains("no supported authentication methods")
    {
        return LoginFailure::Authentication;
    }
    LoginFailure::None
}

/// 识别远端选择器无法解析为服务或上传目标的错误。
pub(super) fn remote_target_missing(message: &str) -> bool {
    message.contains("找不到服务")
        || message.contains("找不到上传目标")
        || message
            .to_ascii_lowercase()
            .contains("upload target not found")
}

/// 同时识别 Unix、PowerShell 与 cmd 的远端命令缺失诊断。
pub(super) fn remote_command_missing(status_code: Option<i32>, message: &str) -> bool {
    if status_code == Some(127) {
        return true;
    }
    let message = message.to_ascii_lowercase();
    message.contains("command not found")
        || message.contains("commandnotfoundexception")
        || message.contains("is not recognized as an internal or external command")
        || message.contains("不是内部或外部命令")
}

/// 识别远端对上传协议或能力版本的明确拒绝。
pub(super) fn transfer_protocol_incompatible(stderr: &[u8]) -> bool {
    let message = String::from_utf8_lossy(stderr).to_ascii_lowercase();
    message.contains("不支持上传协议版本")
        || (message.contains("unsupported") && message.contains("protocol"))
}

#[cfg(test)]
mod tests {
    use super::clean_remote_error;

    // 远端Windows与Unix换行都只保留实际错误，不重复本机帮助尾注。
    #[test]
    fn remote_cli_help_suffix_is_removed_across_line_endings() {
        for line_ending in ["\n", "\r\n"] {
            let message = format!(
                "错误：找不到服务 `demo`{line_ending}{line_ending}运行 `procora --help` 查看用法。{line_ending}"
            );
            assert_eq!(clean_remote_error(&message), "找不到服务 `demo`");
        }
    }
}
