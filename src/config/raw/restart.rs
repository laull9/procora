use super::{ConfigDiagnostic, RawTask, diagnostic};

/// 配置允许的最大自动重启次数。
const MAX_RESTARTS: u32 = 1_000_000;
/// 配置允许的最大自动重启基础退避时间。
const MAX_RESTART_DELAY_MS: u64 = 30_000;
/// 配置允许的最大连续重启计数重置窗口。
const MAX_RESTART_RESET_AFTER_MS: u64 = 86_400_000;
/// 配置允许的最大单 Task 优雅停止时间。
const MAX_SHUTDOWN_TIMEOUT_MS: u64 = 300_000;

impl RawTask {
    /// 聚合校验重启与停止策略的资源边界。
    pub(super) fn validate_runtime_limits(
        &self,
        path: &str,
        diagnostics: &mut Vec<ConfigDiagnostic>,
    ) {
        validate_positive_limit(
            self.restart_delay_ms
                .unwrap_or_else(default_restart_delay_ms),
            MAX_RESTART_DELAY_MS,
            &format!("{path}.restart_delay_ms"),
            diagnostics,
        );
        if self.max_restarts.unwrap_or_default() > MAX_RESTARTS {
            diagnostics.push(diagnostic(
                format!("{path}.max_restarts"),
                format!("不能超过 {MAX_RESTARTS}，0 表示无限"),
            ));
        }
        if self
            .restart_reset_after_ms
            .unwrap_or_else(default_restart_reset_after_ms)
            > MAX_RESTART_RESET_AFTER_MS
        {
            diagnostics.push(diagnostic(
                format!("{path}.restart_reset_after_ms"),
                format!("不能超过 {MAX_RESTART_RESET_AFTER_MS} 毫秒，0 表示永不重置"),
            ));
        }
        validate_positive_limit(
            self.shutdown_timeout_ms
                .unwrap_or_else(default_shutdown_timeout_ms),
            MAX_SHUTDOWN_TIMEOUT_MS,
            &format!("{path}.shutdown_timeout_ms"),
            diagnostics,
        );
    }
}

/// 校验必须为正且不超过上限的毫秒字段。
fn validate_positive_limit(
    value: u64,
    maximum: u64,
    path: &str,
    diagnostics: &mut Vec<ConfigDiagnostic>,
) {
    if value == 0 {
        diagnostics.push(diagnostic(path, "必须大于零"));
    } else if value > maximum {
        diagnostics.push(diagnostic(path, format!("不能超过 {maximum} 毫秒")));
    }
}

/// 默认重启退避毫秒数。
pub(super) const fn default_restart_delay_ms() -> u64 {
    500
}

/// 默认连续重启计数重置窗口。
pub(super) const fn default_restart_reset_after_ms() -> u64 {
    60_000
}

/// 默认停止宽限毫秒数。
pub(super) const fn default_shutdown_timeout_ms() -> u64 {
    5_000
}
