//! TUI 终端环境能力探测。

/// 根据环境变量判断是否启用低能力终端兼容模式。
pub(super) fn terminal_plain_mode() -> bool {
    std::env::var_os("PROCORA_TUI_PLAIN").is_some()
        || std::env::var_os("NO_COLOR").is_some()
        || std::env::var("TERM").is_ok_and(|term| term.eq_ignore_ascii_case("dumb"))
}

/// 返回当前 TUI 是否运行在客户端系统未知的 SSH 会话中。
pub(super) fn terminal_remote_session() -> bool {
    ["SSH_CONNECTION", "SSH_CLIENT", "SSH_TTY"]
        .into_iter()
        .any(|name| std::env::var_os(name).is_some())
}
