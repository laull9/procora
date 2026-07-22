//! TUI 终端环境能力探测。

/// 根据环境变量判断是否启用低能力终端兼容模式。
pub(super) fn terminal_plain_mode() -> bool {
    std::env::var_os("PROCORA_TUI_PLAIN").is_some()
        || std::env::var_os("NO_COLOR").is_some()
        || std::env::var("TERM").is_ok_and(|term| term.eq_ignore_ascii_case("dumb"))
}
