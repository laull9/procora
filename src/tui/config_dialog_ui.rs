use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};

use super::{
    config_directory_picker::DirectoryPicker,
    config_form_dialog::Dialog,
    config_map_dialog::{MapColumn, MapEditor},
    config_ui_support::{centered_rect, focus_style},
    text_view,
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
                let cursor = dialog.selected_input().map_or(0, |(_, _, cursor)| cursor);
                text_view::input_view(
                    value,
                    cursor,
                    inner_width.saturating_sub(prefix_width).saturating_sub(1),
                )
                .text
            } else {
                text_view::clipped(
                    value,
                    0,
                    inner_width.saturating_sub(prefix_width).saturating_sub(1),
                )
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
    let hint = if dialog.selected_is_directory() {
        "F5 浏览目录；也可直接输入，Enter 确认"
    } else if dialog.selected_is_map() {
        "F4 键值表；←→ 移动光标，↑↓ 切换字段，Enter 确认"
    } else if selected_is_choice {
        "↑↓ 切换字段，←→ 选择选项，Enter 确认，Esc 取消"
    } else {
        "直接输入；←→ 移动光标，↑↓ 切换字段，Enter 确认"
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
    if let Some(editor) = dialog.map_editor() {
        render_map_editor(frame, editor);
    }
    if let Some(picker) = dialog.directory_picker() {
        render_directory_picker(frame, picker);
    }
}

/// 绘制跨平台目录选择子弹窗。
fn render_directory_picker(frame: &mut Frame<'_>, picker: &DirectoryPicker) {
    use ratatui::widgets::{List, ListItem, ListState};

    let height = u16::try_from(picker.entries().count().saturating_add(5))
        .unwrap_or(u16::MAX)
        .min(frame.area().height.saturating_sub(4))
        .max(8);
    let area = centered_rect(78, height, frame.area());
    frame.render_widget(Clear, area);
    let mut items = picker
        .entries()
        .map(|(label, selected)| {
            ListItem::new(label).style(if selected {
                focus_style()
            } else {
                Style::default()
            })
        })
        .collect::<Vec<_>>();
    if let Some(error) = picker.error() {
        items.push(ListItem::new(format!("⚠ {error}")));
    }
    let mut state = ListState::default().with_selected(Some(picker.selected()));
    let list = List::new(items).highlight_symbol("› ").block(
        Block::default()
            .title(format!("选择运行目录 · {}", picker.location()))
            .title_bottom("↑↓ 选择 · Enter/→ 打开 · Space 选定当前目录 · ← 返回 · Esc 取消")
            .borders(Borders::ALL),
    );
    frame.render_stateful_widget(list, area, &mut state);
}

/// 把光标放在可编辑字段的真实字符位置。
fn render_input_cursor(
    frame: &mut Frame<'_>,
    area: Rect,
    dialog: &Dialog,
    selected: usize,
    scroll: usize,
    inner_width: usize,
) {
    let Some((label, value, cursor)) = dialog.selected_input() else {
        return;
    };
    let prefix_width = Line::from(format!("› {label}：")).width();
    let view = text_view::input_view(
        value,
        cursor,
        inner_width.saturating_sub(prefix_width).saturating_sub(1),
    );
    let x =
        area.x + 1 + u16::try_from(prefix_width.saturating_add(view.cursor_x)).unwrap_or(u16::MAX);
    let y = area.y + 1 + u16::try_from(selected - scroll).unwrap_or(u16::MAX);
    frame.set_cursor_position((x.min(area.right().saturating_sub(2)), y));
}

/// 绘制映射字段的键值表子弹窗。
fn render_map_editor(frame: &mut Frame<'_>, editor: &MapEditor) {
    let height = u16::try_from(editor.rows().len().saturating_add(5))
        .unwrap_or(u16::MAX)
        .min(frame.area().height.saturating_sub(4));
    let area = centered_rect(72, height.max(7), frame.area());
    frame.render_widget(Clear, area);
    let block = Block::default()
        .title("键值表编辑")
        .title_bottom("Tab 切换键/值 · ↑↓ 换行 · Ctrl-N/D 增删 · Enter 应用 · Esc 取消")
        .borders(Borders::ALL);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let rows_area = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1)])
        .split(inner);
    let key_width = usize::from(inner.width.saturating_sub(7)) / 2;
    let value_width = usize::from(inner.width.saturating_sub(7)).saturating_sub(key_width);
    frame.render_widget(
        Paragraph::new(format!(" #  {:key_width$} │ VALUE", "KEY"))
            .style(Style::default().fg(Color::DarkGray)),
        rows_area[0],
    );
    let (selected, column, cursor) = editor.position();
    let visible = usize::from(rows_area[1].height).max(1);
    let scroll = selected
        .saturating_sub(visible.saturating_sub(1))
        .min(editor.rows().len().saturating_sub(visible));
    let lines = editor
        .rows()
        .iter()
        .enumerate()
        .map(|(index, row)| {
            let row_style = if index == selected {
                focus_style()
            } else {
                Style::default()
            };
            let key = if index == selected && column == MapColumn::Key {
                text_view::input_view(&row.key, cursor, key_width).text
            } else {
                text_view::clipped(&row.key, 0, key_width)
            };
            let value = if index == selected && column == MapColumn::Value {
                text_view::input_view(&row.value, cursor, value_width).text
            } else {
                text_view::clipped(&row.value, 0, value_width)
            };
            Line::from(vec![
                Span::styled(format!("{:>2}  ", index + 1), row_style),
                Span::styled(format!("{key:key_width$}"), row_style),
                Span::raw(" │ "),
                Span::styled(value, row_style),
            ])
        })
        .collect::<Vec<_>>();
    frame.render_widget(
        Paragraph::new(lines).scroll((u16::try_from(scroll).unwrap_or(u16::MAX), 0)),
        rows_area[1],
    );
    let row = &editor.rows()[selected];
    let (text, cell_width, base_x) = match column {
        MapColumn::Key => (&row.key, key_width, 4),
        MapColumn::Value => (&row.value, value_width, 4 + key_width + 3),
    };
    let view = text_view::input_view(text, cursor, cell_width);
    let x = inner.x + u16::try_from(base_x + view.cursor_x).unwrap_or(u16::MAX);
    let y = rows_area[1].y + u16::try_from(selected - scroll).unwrap_or(u16::MAX);
    frame.set_cursor_position((x.min(inner.right().saturating_sub(1)), y));
}
