//! Procora 终端界面的状态、渲染与输入循环。

mod app;
mod config_editor;
mod config_ui;
mod ui;

use std::{
    io,
    time::{Duration, Instant},
};

use crate::core::TaskId;
use crate::protocol::{ProjectSnapshot, ServiceActionDto};
use crossterm::event::{self, Event, KeyEventKind};

pub use app::{ActiveTab, App};
pub use config_editor::ConfigEditor;

/// TUI 从实时会话获得的一批 Task 日志更新。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LogUpdate {
    /// 日志所属 Task。
    pub task_id: TaskId,
    /// 新增原始字节。
    pub bytes: Vec<u8>,
    /// 是否跨越了已经无法读取的文件区间。
    pub gap: bool,
}

/// 打开一个带字段引导和保存前校验的配置编辑页面。
///
/// # Errors
///
/// 当配置文件无法读取或终端无法切换到 TUI 时返回错误。
pub fn edit_config(path: &std::path::Path) -> io::Result<()> {
    let mut editor = ConfigEditor::open(path)?;
    ratatui::run(|terminal| {
        loop {
            terminal.draw(|frame| editor.render(frame))?;
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => editor.handle_key(key),
                _ => {}
            }
            if editor.should_quit() {
                break Ok(());
            }
        }
    })
}

/// TUI 与一个中心服务器服务会话之间的最小交互接口。
pub trait LiveSession {
    /// 检查增量事件，并在数据变化或需要重同步时返回新快照。
    ///
    /// # Errors
    ///
    /// 当中心服务器不可用或返回无效响应时返回错误。
    fn poll_snapshot(&mut self) -> io::Result<Option<ProjectSnapshot>>;

    /// 执行服务级生命周期动作并返回动作完成后的快照。
    ///
    /// # Errors
    ///
    /// 当中心服务器拒绝操作、不可用或返回无效响应时返回错误。
    fn manage(&mut self, action: ServiceActionDto) -> io::Result<ProjectSnapshot>;

    /// 续读指定 Task 的 Service 本地日志。
    ///
    /// # Errors
    ///
    /// 当中心服务器不可用或日志文件无法读取时返回错误。
    fn poll_log(&mut self, task_id: &TaskId) -> io::Result<Option<LogUpdate>>;
}

/// 初始化终端并运行 TUI 输入循环。
///
/// # Errors
///
/// 当终端初始化、绘制、输入读取或终端恢复失败时返回 I/O 错误。
pub fn run(snapshot: ProjectSnapshot) -> io::Result<()> {
    let mut app = App::new(snapshot);
    ratatui::run(|terminal| {
        let mut dirty = true;
        loop {
            if dirty {
                terminal.draw(|frame| app.render(frame))?;
                dirty = false;
            }
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    dirty |= app.handle_key_event(key);
                }
                Event::Resize(_, _) => dirty = true,
                _ => {}
            }
            if app.should_quit() {
                break Ok(());
            }
        }
    })
}

/// 运行可实时刷新并控制所查看服务的中心服务器前端。
///
/// # Errors
///
/// 当终端操作或中心会话交互失败时返回 I/O 错误。
pub fn run_live(
    snapshot: ProjectSnapshot,
    control_allowed: bool,
    session: &mut dyn LiveSession,
) -> io::Result<()> {
    const INPUT_MAX_WAIT: Duration = Duration::from_millis(50);
    const SNAPSHOT_INTERVAL: Duration = Duration::from_millis(500);
    const LOG_INTERVAL: Duration = Duration::from_millis(200);

    let mut app = App::new(snapshot);
    app.set_control_allowed(control_allowed);
    ratatui::run(|terminal| {
        let mut dirty = true;
        let mut next_snapshot = Instant::now();
        let mut next_log = Instant::now();
        loop {
            if dirty {
                terminal.draw(|frame| app.render(frame))?;
                dirty = false;
            }

            let now = Instant::now();
            let mut deadline = next_snapshot;
            if app.active_tab() == ActiveTab::Logs {
                deadline = deadline.min(next_log);
            }
            let timeout = deadline.saturating_duration_since(now).min(INPUT_MAX_WAIT);
            if event::poll(timeout)? {
                match event::read()? {
                    Event::Key(key) if key.kind == KeyEventKind::Press => {
                        dirty |= app.handle_key_event(key);
                    }
                    Event::Resize(_, _) => dirty = true,
                    _ => {}
                }
            }
            if app.should_quit() {
                break Ok(());
            }

            if let Some(action) = app.take_pending_action() {
                match session.manage(action) {
                    Ok(snapshot) => {
                        dirty |= app.replace_snapshot(snapshot);
                        dirty |= app.set_feedback(action_feedback(action));
                    }
                    Err(error) => dirty |= app.set_feedback(format!("操作失败：{error}")),
                }
            }

            let now = Instant::now();
            if now >= next_snapshot {
                next_snapshot = now + SNAPSHOT_INTERVAL;
                match session.poll_snapshot() {
                    Ok(Some(snapshot)) => dirty |= app.replace_snapshot(snapshot),
                    Ok(None) => {}
                    Err(error) => dirty |= app.set_feedback(format!("连接异常：{error}")),
                }
            }

            if app.active_tab() == ActiveTab::Logs && now >= next_log {
                next_log = now + LOG_INTERVAL;
                if let Some(task_id) = app.selected_task().map(|task| task.task_id.clone()) {
                    match session.poll_log(&task_id) {
                        Ok(Some(update)) => {
                            dirty |= app.append_log(update.task_id, &update.bytes, update.gap);
                        }
                        Ok(None) => {}
                        Err(error) => {
                            dirty |= app.set_feedback(format!("日志读取异常：{error}"));
                        }
                    }
                }
            }
        }
    })
}

/// 返回服务生命周期动作完成后的短反馈。
const fn action_feedback(action: ServiceActionDto) -> &'static str {
    match action {
        ServiceActionDto::Start => "服务启动请求已完成",
        ServiceActionDto::Restart => "服务重启请求已完成",
        ServiceActionDto::Stop => "服务停止请求已完成",
    }
}
