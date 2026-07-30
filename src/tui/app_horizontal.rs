//! 主 TUI 各页面参与水平移动的文本范围计算。

use super::{ActiveTab, App};

/// 返回当前页面参与手动或全局自动移动的最长文本字符数。
pub(super) fn page_text_maximum(app: &App, global: bool) -> usize {
    let content_maximum = match app.active_tab() {
        ActiveTab::Tasks => app
            .selected_task()
            .map_or(0, |task| {
                let dependencies = task
                    .dependencies
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ");
                [
                    crate::tui::text_view::width(task.task_id.as_str()),
                    crate::tui::text_view::width(&task.command),
                    crate::tui::text_view::width(&dependencies),
                    task.message
                        .as_deref()
                        .map_or(0, crate::tui::text_view::width),
                ]
                .into_iter()
                .max()
                .unwrap_or(0)
            })
            .max(if global {
                app.snapshot()
                    .tasks
                    .iter()
                    .map(|task| crate::tui::text_view::width(task.task_id.as_str()))
                    .max()
                    .unwrap_or(0)
            } else {
                0
            }),
        ActiveTab::Dependencies => app
            .snapshot()
            .tasks
            .iter()
            .map(|task| {
                let dependency = task
                    .dependencies
                    .iter()
                    .map(ToString::to_string)
                    .map(|value| crate::tui::text_view::width(&value))
                    .max()
                    .unwrap_or(0);
                dependency.saturating_add(crate::tui::text_view::width(task.task_id.as_str()) + 4)
            })
            .max()
            .unwrap_or(0),
        ActiveTab::Logs => app
            .selected_task()
            .map_or(0, |task| app.log_maximum_width(&task.task_id)),
    };
    if global {
        content_maximum
            .max(crate::tui::text_view::width(&app.snapshot().project).saturating_add(12))
            .max(app.feedback().map_or(0, crate::tui::text_view::width))
            .max(app.selected_task().map_or(0, |task| {
                crate::tui::text_view::width(task.task_id.as_str())
            }))
    } else {
        content_maximum
    }
}
