//! 主 TUI 的状态标签、颜色和短文本格式化辅助。

use crate::protocol::{SnapshotSourceDto, TaskStatusDto, TaskView};
use ratatui::{
    style::Color,
    symbols::border,
    widgets::{Block, Borders},
};

use super::App;

/// 低能力终端使用的 ASCII 边框。
const ASCII_BORDER: border::Set<'static> = border::Set {
    top_left: "+",
    top_right: "+",
    bottom_left: "+",
    bottom_right: "+",
    vertical_left: "|",
    vertical_right: "|",
    horizontal_top: "-",
    horizontal_bottom: "-",
};

/// 返回快照来源标签及颜色。
pub(super) const fn source_label(source: SnapshotSourceDto, plain: bool) -> (&'static str, Color) {
    let (label, color) = match source {
        SnapshotSourceDto::ConfigPreview => ("预览", Color::Yellow),
        SnapshotSourceDto::EmbeddedLive => ("临时服务", Color::Green),
        SnapshotSourceDto::CenterLive => ("全局服务", Color::Green),
        SnapshotSourceDto::CenterStale => ("连接中断", Color::Red),
    };
    (label, if plain { Color::Reset } else { color })
}

/// 返回任务状态的符号及颜色。
pub(super) const fn status_visual(status: TaskStatusDto, plain: bool) -> (&'static str, Color) {
    if plain {
        return match status {
            TaskStatusDto::Pending => ("o", Color::Reset),
            TaskStatusDto::Blocked => ("?", Color::Reset),
            TaskStatusDto::Running => ("*", Color::Reset),
            TaskStatusDto::Stopped => ("-", Color::Reset),
            TaskStatusDto::Failed => ("x", Color::Reset),
        };
    }
    match status {
        TaskStatusDto::Pending => ("○", Color::Yellow),
        TaskStatusDto::Blocked => ("◆", Color::Magenta),
        TaskStatusDto::Running => ("●", Color::Green),
        TaskStatusDto::Stopped => ("■", Color::DarkGray),
        TaskStatusDto::Failed => ("×", Color::Red),
    }
}

/// 返回任务状态的中文标签。
pub(super) const fn status_label(status: TaskStatusDto) -> &'static str {
    match status {
        TaskStatusDto::Pending => "等待调度",
        TaskStatusDto::Blocked => "依赖阻断",
        TaskStatusDto::Running => "运行中",
        TaskStatusDto::Stopped => "已停止",
        TaskStatusDto::Failed => "失败",
    }
}

/// 返回任务资源的可读标签。
pub(super) fn resource_labels(task: &TaskView, plain: bool) -> (String, String) {
    let unavailable = if plain { "-" } else { "—" };
    task.resources.map_or_else(
        || (unavailable.to_owned(), unavailable.to_owned()),
        |resources| {
            let cpu = resources.cpu_tenths_percent.map_or_else(
                || unavailable.to_owned(),
                |value| format!("{}.{:01}%", value / 10, value % 10),
            );
            let memory = resources
                .memory_bytes
                .map_or_else(|| unavailable.to_owned(), format_bytes);
            (cpu, memory)
        },
    )
}

/// 创建统一边框块。
pub(super) fn bordered(app: &App) -> Block<'_> {
    bordered_for(app.plain_mode())
}

/// 按终端能力创建统一边框块。
pub(super) fn bordered_for(plain: bool) -> Block<'static> {
    let block = Block::default().borders(Borders::ALL);
    if plain {
        block.border_set(ASCII_BORDER)
    } else {
        block
    }
}

/// 在纯文本模式下关闭显式颜色。
pub(super) const fn display_color(app: &App, color: Color) -> Color {
    display_color_for(app.plain_mode(), color)
}

/// 按终端能力决定是否保留显式颜色。
pub(super) const fn display_color_for(plain: bool, color: Color) -> Color {
    if plain { Color::Reset } else { color }
}

/// 将字节数格式化为适合终端详情面板的短文本。
fn format_bytes(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = KIB * 1024;
    const GIB: u64 = MIB * 1024;
    if bytes >= GIB {
        format_unit(bytes, GIB, "GiB")
    } else if bytes >= MIB {
        format_unit(bytes, MIB, "MiB")
    } else if bytes >= KIB {
        format_unit(bytes, KIB, "KiB")
    } else {
        format!("{bytes} B")
    }
}

/// 使用整数运算生成保留一位小数的容量文本。
fn format_unit(bytes: u64, unit: u64, suffix: &str) -> String {
    let whole = bytes / unit;
    let decimal = (bytes % unit) * 10 / unit;
    format!("{whole}.{decimal} {suffix}")
}
