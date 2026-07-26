//! Task 运行错误的归一化、聚合与特殊日志编码。

use std::{
    collections::{BTreeMap, VecDeque},
    io,
    sync::{Arc, Mutex},
};

use crate::{
    core::{TaskId, TaskSpec},
    protocol::{TaskDiagnosticDto, TaskDiagnosticKindDto},
};

/// 每个 Task 在综合分析中保留的最近诊断类别上限。
const MAX_TASK_DIAGNOSTICS: usize = 8;
/// 单条诊断允许进入状态和日志的最大字符数。
const MAX_DIAGNOSTIC_CHARS: usize = 2048;

/// 单次诊断记录后的聚合结果。
pub(crate) struct RecordedDiagnostic {
    /// 更新后的聚合诊断。
    pub(crate) diagnostic: TaskDiagnosticDto,
    /// 是否应把本次出现写入持久日志。
    pub(crate) emit_log: bool,
}

/// 单个宿主生命周期内的有界 Task 诊断集合。
#[derive(Debug, Default)]
pub(crate) struct TaskDiagnostics {
    entries: BTreeMap<TaskId, VecDeque<TaskDiagnosticDto>>,
}

impl TaskDiagnostics {
    /// 清空上一轮显式服务启动留下的综合分析。
    pub(super) fn clear(&mut self) {
        self.entries.clear();
    }

    /// 聚合一条诊断，并仅在首次和二次幂重复次数时请求写日志。
    pub(super) fn record(
        &mut self,
        task_id: &TaskId,
        kind: TaskDiagnosticKindDto,
        message: impl Into<String>,
        suggestion: Option<String>,
    ) -> RecordedDiagnostic {
        let message = sanitize(&message.into());
        let suggestion = suggestion.map(|suggestion| sanitize(&suggestion));
        let entries = self.entries.entry(task_id.clone()).or_default();
        let existing = entries.iter().position(|diagnostic| {
            diagnostic.kind == kind
                && diagnostic.message == message
                && diagnostic.suggestion == suggestion
        });
        let mut diagnostic = existing.map_or_else(
            || TaskDiagnosticDto {
                kind,
                message,
                suggestion,
                occurrences: 0,
            },
            |index| entries.remove(index).expect("诊断索引来自同一队列"),
        );
        diagnostic.occurrences = diagnostic.occurrences.saturating_add(1);
        let emit_log = diagnostic.occurrences == 1 || diagnostic.occurrences.is_power_of_two();
        entries.push_back(diagnostic.clone());
        if entries.len() > MAX_TASK_DIAGNOSTICS {
            entries.pop_front();
        }
        RecordedDiagnostic {
            diagnostic,
            emit_log,
        }
    }

    /// 返回一个 Task 按最近更新时间排列的诊断。
    pub(super) fn task(&self, task_id: &TaskId) -> Vec<TaskDiagnosticDto> {
        self.entries
            .get(task_id)
            .map_or_else(Vec::new, |entries| entries.iter().cloned().collect())
    }
}

/// 在线程安全的共享集合中记录一条诊断。
pub(crate) fn record_shared(
    diagnostics: &Arc<Mutex<TaskDiagnostics>>,
    task_id: &TaskId,
    kind: TaskDiagnosticKindDto,
    message: impl Into<String>,
    suggestion: Option<String>,
) -> RecordedDiagnostic {
    diagnostics
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .record(task_id, kind, message, suggestion)
}

/// 把聚合诊断编码为终端可识别、纯文本仍可读的一行系统日志。
pub(crate) fn styled_log(diagnostic: &TaskDiagnosticDto) -> Vec<u8> {
    let message = sanitize(&diagnostic.message);
    let repeats = if diagnostic.occurrences > 1 {
        format!(" · 累计 {} 次", diagnostic.occurrences)
    } else {
        String::new()
    };
    let suggestion = diagnostic
        .suggestion
        .as_deref()
        .map(sanitize)
        .map(|suggestion| format!("；建议：{suggestion}"))
        .unwrap_or_default();
    format!(
        "\u{1b}[2;3;91m{} {}] {}{suggestion}{repeats}\u{1b}[0m\n",
        crate::log::TASK_DIAGNOSTIC_PREFIX,
        kind_label(diagnostic.kind),
        message,
    )
    .into_bytes()
}

/// 为进程创建错误生成可操作的中文诊断。
pub(super) fn spawn_error(task: &TaskSpec, error: &io::Error) -> (String, Option<String>) {
    let (reason, suggestion) = io_error(error);
    let cwd = task
        .cwd
        .as_deref()
        .map_or_else(|| "服务目录".to_owned(), |path| path.display().to_string());
    (
        format!(
            "Task 启动失败：{reason}；命令 `{}`；工作目录 `{cwd}`",
            sanitize(&task.command)
        ),
        Some(suggestion),
    )
}

/// 为通用进程 I/O 错误生成稳定中文说明与建议。
pub(crate) fn process_io_error(context: &str, error: &io::Error) -> (String, Option<String>) {
    let (reason, suggestion) = io_error(error);
    (format!("{context}：{reason}"), Some(suggestion))
}

/// 返回诊断类别的短中文标签。
pub(super) const fn kind_label(kind: TaskDiagnosticKindDto) -> &'static str {
    match kind {
        TaskDiagnosticKindDto::Spawn => "启动",
        TaskDiagnosticKindDto::Exit => "退出",
        TaskDiagnosticKindDto::Health => "健康检查",
        TaskDiagnosticKindDto::Process => "进程",
        TaskDiagnosticKindDto::Output => "输出",
        TaskDiagnosticKindDto::Restart => "重启",
    }
}

/// 把常见平台 I/O 错误映射为稳定中文根因。
fn io_error(error: &io::Error) -> (String, String) {
    let (summary, suggestion) = match error.kind() {
        io::ErrorKind::NotFound => (
            "未找到文件或目录",
            "检查 Task 的 command、工作目录及依赖文件路径",
        ),
        io::ErrorKind::PermissionDenied => ("权限不足", "检查程序执行权限和服务目录访问权限"),
        io::ErrorKind::ConnectionRefused => ("连接被拒绝", "确认目标服务已经启动并监听正确地址"),
        io::ErrorKind::ConnectionReset => ("连接被对端重置", "检查目标服务状态和网络中间设备"),
        io::ErrorKind::TimedOut => ("操作超时", "检查系统负载、超时配置和外部依赖可用性"),
        io::ErrorKind::InvalidInput | io::ErrorKind::InvalidData => {
            ("输入或数据无效", "检查 Task 配置、参数和相关文件内容")
        }
        io::ErrorKind::OutOfMemory => ("系统内存不足", "释放内存或降低并发 Task 数量后重试"),
        _ => ("系统进程操作失败", "查看系统日志并检查进程、路径与资源限制"),
    };
    let original = sanitize(&error.to_string());
    let reason = if original.is_empty() {
        summary.to_owned()
    } else {
        format!("{summary}（系统：{original}）")
    };
    (reason, suggestion.to_owned())
}

/// 清除控制序列、换行和过长内容，确保诊断只占一条日志行。
fn sanitize(value: &str) -> String {
    crate::log::strip_ansi(value)
        .replace(['\n', '\t'], " ")
        .chars()
        .take(MAX_DIAGNOSTIC_CHARS)
        .collect()
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use crate::{
        core::TaskId,
        protocol::{TaskDiagnosticDto, TaskDiagnosticKindDto},
    };

    use super::{TaskDiagnostics, styled_log};

    #[test]
    // 相同诊断持续聚合，但仅在首次和二次幂次数请求落盘。
    fn repeated_diagnostics_are_aggregated_and_rate_limited() {
        let task_id = TaskId::from_str("app").unwrap();
        let mut diagnostics = TaskDiagnostics::default();
        let emitted = (1..=5)
            .map(|_| {
                diagnostics
                    .record(
                        &task_id,
                        TaskDiagnosticKindDto::Health,
                        "连接失败",
                        Some("检查依赖".to_owned()),
                    )
                    .emit_log
            })
            .collect::<Vec<_>>();

        assert_eq!(emitted, [true, true, false, true, false]);
        assert_eq!(diagnostics.task(&task_id)[0].occurrences, 5);
    }

    #[test]
    // 诊断日志具有稳定标记、斜体样式和单行建议。
    fn diagnostic_log_uses_distinct_single_line_style() {
        let bytes = styled_log(&TaskDiagnosticDto {
            kind: TaskDiagnosticKindDto::Spawn,
            message: "未找到\n文件".to_owned(),
            suggestion: Some("检查路径".to_owned()),
            occurrences: 1,
        });
        let text = String::from_utf8(bytes).unwrap();

        assert!(text.starts_with("\u{1b}[2;3;91m[Procora 诊断 · 启动]"));
        assert!(text.contains("建议：检查路径"));
        assert_eq!(text.matches('\n').count(), 1);
    }
}
