use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};

use super::{
    config_form_dialog::Dialog,
    config_ui::{centered_rect, focus_style},
};

/// 绘制字段输入和选择器弹窗，并为普通文本字段设置终端光标。
pub(super) fn render(frame: &mut Frame<'_>, dialog: &Dialog) {
    let (field_count, selected) = dialog.field_position();
    let height = u16::try_from(field_count.saturating_add(5)).unwrap_or(u16::MAX);
    let area = centered_rect(
        86,
        height.min(frame.area().height.saturating_sub(2)),
        frame.area(),
    );
    frame.render_widget(Clear, area);
    let selected_is_choice = dialog.selected_is_choice();
    let inner_width = usize::from(area.width.saturating_sub(2));
    let lines = dialog
        .fields()
        .map(|(label, value, selected)| {
            let marker = if selected { "› " } else { "  " };
            let style = if selected {
                focus_style()
            } else {
                Style::default()
            };
            let prefix_width = Line::from(format!("{marker}{label}：")).width();
            let value = if selected && !selected_is_choice {
                trailing_text(
                    value,
                    inner_width.saturating_sub(prefix_width).saturating_sub(1),
                )
            } else {
                value.to_owned()
            };
            Line::from(vec![
                Span::styled(marker, style),
                Span::styled(format!("{label}："), Style::default().fg(Color::DarkGray)),
                Span::styled(value, style),
            ])
        })
        .collect::<Vec<_>>();
    // 底部操作提示占用边框内最后一行，滚动窗口额外为它保留一行。
    let visible_lines = usize::from(area.height.saturating_sub(3)).max(1);
    let scroll = selected
        .saturating_sub(visible_lines.saturating_sub(1))
        .min(field_count.saturating_sub(visible_lines));
    let hint = if selected_is_choice {
        "↑↓ 切换字段，←→ 选择选项，Enter 确认，Esc 取消"
    } else {
        "直接输入；↑↓ 切换字段，Enter 确认，Esc 取消"
    };
    frame.render_widget(
        Paragraph::new(lines)
            .scroll((u16::try_from(scroll).unwrap_or(u16::MAX), 0))
            .block(
                Block::default()
                    .title(format!(
                        "{} · {}/{}",
                        dialog.title(),
                        selected.saturating_add(1),
                        field_count
                    ))
                    .borders(Borders::ALL)
                    .title_bottom(hint),
            ),
        area,
    );
    render_input_cursor(frame, area, dialog, selected, scroll, inner_width);
}

/// 把光标放在可编辑字段的可见文本末尾。
fn render_input_cursor(
    frame: &mut Frame<'_>,
    area: Rect,
    dialog: &Dialog,
    selected: usize,
    scroll: usize,
    inner_width: usize,
) {
    let Some((label, value)) = dialog.selected_input() else {
        return;
    };
    let prefix_width = Line::from(format!("› {label}：")).width();
    let value = trailing_text(
        value,
        inner_width.saturating_sub(prefix_width).saturating_sub(1),
    );
    let x = area.x
        + 1
        + u16::try_from(prefix_width.saturating_add(Line::from(value).width())).unwrap_or(u16::MAX);
    let y = area.y + 1 + u16::try_from(selected - scroll).unwrap_or(u16::MAX);
    frame.set_cursor_position((x.min(area.right().saturating_sub(2)), y));
}

/// 截取文本末尾以保证当前输入位置始终留在弹窗可见区域。
fn trailing_text(value: &str, max_width: usize) -> String {
    if Line::from(value).width() <= max_width {
        return value.to_owned();
    }
    if max_width == 0 {
        return String::new();
    }
    let suffix_width = max_width.saturating_sub(1);
    let mut width: usize = 0;
    let mut suffix = Vec::new();
    for character in value.chars().rev() {
        let character_width = Line::from(character.to_string()).width();
        if width.saturating_add(character_width) > suffix_width {
            break;
        }
        width += character_width;
        suffix.push(character);
    }
    suffix.reverse();
    format!("…{}", suffix.into_iter().collect::<String>())
}
