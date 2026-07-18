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
                    task.task_id.as_str().chars().count(),
                    task.command.chars().count(),
                    dependencies.chars().count(),
                    task.message
                        .as_deref()
                        .map_or(0, |value| value.chars().count()),
                ]
                .into_iter()
                .max()
                .unwrap_or(0)
            })
            .max(if global {
                app.snapshot()
                    .tasks
                    .iter()
                    .map(|task| task.task_id.as_str().chars().count())
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
                    .map(|value| value.chars().count())
                    .max()
                    .unwrap_or(0);
                dependency.saturating_add(task.task_id.as_str().chars().count() + 4)
            })
            .max()
            .unwrap_or(0),
        ActiveTab::Logs => app
            .selected_task()
            .and_then(|task| app.log_text(&task.task_id))
            .map_or(0, |text| {
                text.lines()
                    .map(|line| line.chars().count())
                    .max()
                    .unwrap_or(0)
            }),
    };
    if global {
        content_maximum
            .max(app.snapshot().project.chars().count().saturating_add(12))
            .max(app.feedback().map_or(0, |value| value.chars().count()))
            .max(
                app.selected_task()
                    .map_or(0, |task| task.task_id.as_str().chars().count()),
            )
    } else {
        content_maximum
    }
}
