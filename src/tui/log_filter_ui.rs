//! 日志页低干扰来源过滤标签。

use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

use super::{App, LogSourceFilter, ui_support::display_color};

/// 构造日志标题，并按宽度展示来源过滤标签。
pub(super) fn title(
    area_width: u16,
    task: &str,
    position: Option<&str>,
    app: &App,
) -> Line<'static> {
    let base = position.map_or_else(
        || format!("日志 · {task}"),
        |position| format!("日志 · {task} · {position}"),
    );
    let switch_hint = if app.plain_mode() {
        "j/k 切换任务日志"
    } else {
        "↑/↓ 切换任务日志"
    };
    let expanded = position.map_or_else(
        || format!("日志 · {task} · {switch_hint}"),
        |position| format!("日志 · {task} · {switch_hint} · {position}"),
    );
    let tags_width = Line::from("  [全部] [Procora] [子进程]").width();
    let available = usize::from(area_width).saturating_sub(4);
    let title = if Line::from(expanded.as_str())
        .width()
        .saturating_add(tags_width)
        <= available
    {
        expanded
    } else {
        base
    };
    let mut spans = vec![Span::raw(title.clone())];
    if Line::from(title.as_str())
        .width()
        .saturating_add(tags_width)
        <= available
    {
        for source in [
            LogSourceFilter::All,
            LogSourceFilter::Procora,
            LogSourceFilter::Child,
        ] {
            spans.push(filter_span(source, app.log_source_filter(), app));
        }
    } else {
        let active = format!("  [{}]", app.log_source_filter().label());
        if Line::from(title.as_str())
            .width()
            .saturating_add(Line::from(active.as_str()).width())
            <= available
        {
            spans.push(Span::styled(
                active,
                filter_style(app.log_source_filter(), true, app),
            ));
        }
    }
    Line::from(spans)
}

/// 创建一个日志来源标签，并用颜色与轻量修饰表达当前选择。
fn filter_span(source: LogSourceFilter, active: LogSourceFilter, app: &App) -> Span<'static> {
    let selected = source == active;
    let marker = if app.plain_mode() {
        if selected { " *" } else { "  " }
    } else {
        " "
    };
    Span::styled(
        format!("{marker}[{}]", source.label()),
        filter_style(source, selected, app),
    )
}

/// 返回各日志来源稳定且不过分抢眼的颜色。
fn filter_style(source: LogSourceFilter, selected: bool, app: &App) -> Style {
    let color = match source {
        LogSourceFilter::All => Color::Cyan,
        LogSourceFilter::Procora => Color::LightRed,
        LogSourceFilter::Child => Color::Green,
    };
    let mut style = Style::default().fg(display_color(app, color));
    if selected {
        style = style.add_modifier(Modifier::BOLD);
    } else if !app.plain_mode() {
        style = style.add_modifier(Modifier::DIM);
    }
    style
}
