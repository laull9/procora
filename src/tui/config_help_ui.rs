//! 结构化配置表单右侧的彩色键位提示。

use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

use super::config_form_state::FormState;

/// 构造带颜色区分的结构化表单键位提示。
pub(super) fn form_key_hints(form: &FormState) -> Vec<Line<'static>> {
    vec![
        Line::styled("按键", Style::default().add_modifier(Modifier::BOLD)),
        key_hint("Tab / Shift-Tab", "切换区域"),
        key_hint("↑ / ↓", "选择；边界处自动跨区"),
        key_hint("← / →", "水平移动当前高亮文本"),
        key_hint(
            "F3",
            format!("全局自动滚动折叠文本：{}", auto_scroll_label(form)),
        ),
        key_hint("Enter / h / a", "编辑 / 健康检查 / 依赖高级策略"),
        key_hint("n / d", "新建 / 二次确认删除"),
        key_hint("Ctrl-S / F2", "保存 / 高级文本"),
        key_hint("Esc", "退出；未保存内容会请求确认"),
    ]
}

/// 构造一行高亮键位和普通说明文字。
fn key_hint(keys: &'static str, description: impl Into<String>) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("{keys}  "),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(description.into()),
    ])
}

/// 返回结构化表单自动滚动及高亮冻结状态标签。
fn auto_scroll_label(form: &FormState) -> &'static str {
    if form.auto_scroll_enabled() && form.manual_scroll_frozen() {
        "开·高亮冻结"
    } else if form.auto_scroll_enabled() {
        "开"
    } else {
        "关"
    }
}
