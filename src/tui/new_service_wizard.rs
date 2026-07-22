//! 总览页新建托管服务的目录与名称向导。

use std::{fs, io, path::PathBuf};

use crate::core::ServiceName;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph},
};

use super::{
    INPUT_MAX_WAIT,
    config_directory_picker::{DirectoryPicker, DirectoryPickerEvent},
    config_ui_support::{centered_rect, focus_style},
    text_view,
};

/// 新服务向导完成后交给 CLI 的创建意图。
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct NewServiceChoice {
    /// 用户选中的已有托管目录。
    pub(crate) directory: PathBuf,
    /// 目录没有配置时要创建的服务名称；已有入口时为空。
    pub(crate) new_service_name: Option<String>,
}

/// 新服务向导当前所在步骤。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WizardStep {
    Directory,
    Name,
}

/// 向导循环的完成状态。
enum WizardResult {
    Pending,
    Cancelled,
    Created(NewServiceChoice),
}

/// 新服务向导的可测试交互状态。
struct NewServiceWizard {
    base_directory: PathBuf,
    picker: DirectoryPicker,
    step: WizardStep,
    directory: Option<PathBuf>,
    name: String,
    cursor: usize,
    error: Option<String>,
    result: WizardResult,
}

impl NewServiceWizard {
    /// 从调用者当前目录创建默认聚焦“选择此目录”的向导。
    fn new(base_directory: PathBuf) -> Self {
        Self {
            picker: DirectoryPicker::new(0, ".", &base_directory),
            base_directory,
            step: WizardStep::Directory,
            directory: None,
            name: String::new(),
            cursor: 0,
            error: None,
            result: WizardResult::Pending,
        }
    }

    /// 处理当前步骤的一次按键。
    fn handle_key(&mut self, key: KeyEvent) {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.result = WizardResult::Cancelled;
            return;
        }
        match self.step {
            WizardStep::Directory => self.handle_directory_key(key),
            WizardStep::Name => self.handle_name_key(key),
        }
    }

    /// 处理目录浏览，已有配置直接完成，空目录进入命名步骤。
    fn handle_directory_key(&mut self, key: KeyEvent) {
        match self.picker.handle_key(key) {
            DirectoryPickerEvent::Continue => {}
            DirectoryPickerEvent::Cancel => self.result = WizardResult::Cancelled,
            DirectoryPickerEvent::Selected { value, .. } => {
                let directory = self.base_directory.join(value);
                let directory = fs::canonicalize(&directory).unwrap_or(directory);
                if contains_config_entry(&directory) {
                    self.result = WizardResult::Created(NewServiceChoice {
                        directory,
                        new_service_name: None,
                    });
                    return;
                }
                self.name = default_service_name(&directory);
                self.cursor = self.name.chars().count();
                self.directory = Some(directory);
                self.error = None;
                self.step = WizardStep::Name;
            }
        }
    }

    /// 编辑并校验新配置使用的稳定服务名称。
    fn handle_name_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.step = WizardStep::Directory;
                self.error = None;
            }
            KeyCode::Enter => match self.name.parse::<ServiceName>() {
                Ok(_) => {
                    self.result = WizardResult::Created(NewServiceChoice {
                        directory: self.directory.clone().expect("命名步骤已经选择目录"),
                        new_service_name: Some(self.name.clone()),
                    });
                }
                Err(error) => self.error = Some(error.to_string()),
            },
            KeyCode::Left => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Right => self.cursor = (self.cursor + 1).min(self.name.chars().count()),
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = self.name.chars().count(),
            KeyCode::Backspace if self.cursor > 0 => {
                let index = char_to_byte(&self.name, self.cursor - 1);
                self.name.remove(index);
                self.cursor -= 1;
                self.error = None;
            }
            KeyCode::Delete if self.cursor < self.name.chars().count() => {
                let index = char_to_byte(&self.name, self.cursor);
                self.name.remove(index);
                self.error = None;
            }
            KeyCode::Char(character) => {
                let index = char_to_byte(&self.name, self.cursor);
                self.name.insert(index, character);
                self.cursor += 1;
                self.error = None;
            }
            _ => {}
        }
    }

    /// 绘制目录或服务命名步骤。
    fn render(&self, frame: &mut Frame<'_>) {
        match self.step {
            WizardStep::Directory => render_directory(frame, &self.picker),
            WizardStep::Name => render_name(frame, self),
        }
    }
}

/// 运行总览页的新建托管服务向导。
pub(crate) fn run(base_directory: PathBuf) -> io::Result<Option<NewServiceChoice>> {
    let mut wizard = NewServiceWizard::new(base_directory);
    ratatui::run(|terminal| {
        loop {
            terminal.draw(|frame| wizard.render(frame))?;
            if event::poll(INPUT_MAX_WAIT)? {
                match event::read()? {
                    Event::Key(key) if key.kind == KeyEventKind::Press => wizard.handle_key(key),
                    _ => {}
                }
            }
            match std::mem::replace(&mut wizard.result, WizardResult::Pending) {
                WizardResult::Pending => {}
                WizardResult::Cancelled => break Ok(None),
                WizardResult::Created(choice) => break Ok(Some(choice)),
            }
        }
    })
}

/// 绘制全屏目录选择步骤。
fn render_directory(frame: &mut Frame<'_>, picker: &DirectoryPicker) {
    let area = frame.area();
    frame.render_widget(Clear, area);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(4), Constraint::Min(5)])
        .split(area);
    frame.render_widget(
        Paragraph::new("新建托管服务 · 1/2\n选择已有服务目录，或选择一个空目录创建标准配置")
            .alignment(Alignment::Center)
            .block(Block::default().borders(Borders::ALL)),
        rows[0],
    );
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
    frame.render_stateful_widget(
        List::new(items).highlight_symbol("› ").block(
            Block::default()
                .title(picker.location())
                .title_bottom("↑↓ 选择 · Enter/→ 打开 · Space 选定 · ← 返回 · r 刷新 · Esc 取消")
                .borders(Borders::ALL),
        ),
        rows[1],
        &mut state,
    );
}

/// 绘制新配置的服务名称步骤并设置终端光标。
fn render_name(frame: &mut Frame<'_>, wizard: &NewServiceWizard) {
    let area = centered_rect(72, 9, frame.area());
    frame.render_widget(Clear, area);
    let directory = wizard.directory.as_deref().expect("命名步骤已经选择目录");
    let mut lines = vec![
        Line::from(format!("托管目录：{}", directory.display())),
        Line::from(""),
        Line::from(format!("服务名称：{}", wizard.name)),
        Line::from(""),
        Line::from("将创建 procora.yaml；进入服务页后按 n 新建第一个 Task。"),
    ];
    if let Some(error) = &wizard.error {
        lines.push(Line::styled(error, Style::default().fg(Color::Red)));
    }
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .title("新建托管服务 · 2/2")
                .title_bottom("Enter 创建并进入编辑 · Esc 返回目录选择")
                .borders(Borders::ALL)
                .border_style(Style::default().add_modifier(Modifier::BOLD)),
        ),
        area,
    );
    let prefix = text_view::width("服务名称：");
    let cursor = text_view::width(&wizard.name.chars().take(wizard.cursor).collect::<String>());
    frame.set_cursor_position((
        area.x
            .saturating_add(1)
            .saturating_add(u16::try_from(prefix + cursor).unwrap_or(u16::MAX))
            .min(area.right().saturating_sub(2)),
        area.y.saturating_add(3),
    ));
}

/// 判断目录是否已经含有约定名称的声明式配置入口。
fn contains_config_entry(directory: &std::path::Path) -> bool {
    [
        "procora.yaml",
        "procora.yml",
        "procora.toml",
        "procora.json",
    ]
    .iter()
    .any(|name| directory.join(name).is_file())
}

/// 从目录名生成符合约束的默认服务名称。
fn default_service_name(directory: &std::path::Path) -> String {
    let mut name = directory
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("service")
        .to_ascii_lowercase()
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_') {
                character
            } else {
                '-'
            }
        })
        .collect::<String>();
    while name.starts_with(|character: char| !character.is_ascii_alphanumeric()) {
        name.remove(0);
    }
    if name.is_empty() {
        "service".to_owned()
    } else {
        name
    }
}

/// 把字符序号转换为 UTF-8 字节位置。
fn char_to_byte(value: &str, index: usize) -> usize {
    value
        .char_indices()
        .nth(index)
        .map_or(value.len(), |(byte, _)| byte)
}
