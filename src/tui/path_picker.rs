use std::{
    fs,
    io::{self, IsTerminal},
    path::{Path, PathBuf},
};

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
};

use super::{inline_terminal::InlineTerminal, key_hints, text_view};

/// 路径浏览器中一条可执行项目。
#[derive(Clone, Debug)]
struct PathEntry {
    label: String,
    action: PathAction,
}

/// 路径项目被确认后的动作。
#[derive(Clone, Debug)]
enum PathAction {
    SelectCurrent,
    Parent,
    Open(PathBuf),
    Select(PathBuf),
}

/// 同时支持普通文件和目录的终端路径浏览器。
struct PathPicker {
    current: PathBuf,
    entries: Vec<PathEntry>,
    selected: usize,
    error: Option<String>,
    title: String,
    instruction: String,
}

impl PathPicker {
    /// 从上次路径或当前目录创建浏览器。
    fn new(initial: Option<&Path>, title: &str, instruction: &str) -> io::Result<Self> {
        let current = initial
            .and_then(|path| {
                if path.is_dir() {
                    Some(path)
                } else {
                    path.parent()
                }
            })
            .map_or(crate::platform::current_dir()?, Path::to_path_buf);
        let current = crate::platform::canonicalize(&current)
            .unwrap_or_else(|_| crate::platform::simplify_path(&current));
        let mut picker = Self {
            current,
            entries: Vec::new(),
            selected: 0,
            error: None,
            title: title.to_owned(),
            instruction: instruction.to_owned(),
        };
        picker.refresh();
        Ok(picker)
    }

    /// 读取当前目录并按目录优先、名称稳定排序。
    fn refresh(&mut self) {
        self.entries.clear();
        self.error = None;
        self.entries.push(PathEntry {
            label: "✓ 选择当前文件夹".to_owned(),
            action: PathAction::SelectCurrent,
        });
        if let Some(parent) = self.current.parent() {
            self.entries.push(PathEntry {
                label: "↰ 返回上级".to_owned(),
                action: PathAction::Parent,
            });
            debug_assert_ne!(parent, self.current);
        }
        match fs::read_dir(&self.current) {
            Ok(entries) => {
                let mut entries = entries
                    .filter_map(Result::ok)
                    .filter_map(|entry| {
                        let file_type = entry.file_type().ok()?;
                        let name = entry.file_name().to_string_lossy().into_owned();
                        if file_type.is_dir() {
                            Some((
                                0_u8,
                                name.to_ascii_lowercase(),
                                PathEntry {
                                    label: format!("▸ {name}/"),
                                    action: PathAction::Open(entry.path()),
                                },
                            ))
                        } else if file_type.is_file() {
                            Some((
                                1_u8,
                                name.to_ascii_lowercase(),
                                PathEntry {
                                    label: format!("  {name}"),
                                    action: PathAction::Select(entry.path()),
                                },
                            ))
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>();
                entries.sort_by(|left, right| (&left.0, &left.1).cmp(&(&right.0, &right.1)));
                self.entries
                    .extend(entries.into_iter().map(|(_, _, entry)| entry));
            }
            Err(error) => self.error = Some(format!("无法读取目录：{error}")),
        }
        self.selected = self.selected.min(self.entries.len().saturating_sub(1));
    }

    /// 处理一次按键并在确认时返回路径。
    fn handle_key(&mut self, code: KeyCode) -> PickerEvent {
        match code {
            KeyCode::Esc | KeyCode::Char('q') => PickerEvent::Cancelled,
            KeyCode::Up | KeyCode::Char('k') => {
                self.selected = self.selected.saturating_sub(1);
                PickerEvent::Pending
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.selected = (self.selected + 1).min(self.entries.len().saturating_sub(1));
                PickerEvent::Pending
            }
            KeyCode::Home => {
                self.selected = 0;
                PickerEvent::Pending
            }
            KeyCode::End => {
                self.selected = self.entries.len().saturating_sub(1);
                PickerEvent::Pending
            }
            KeyCode::Left | KeyCode::Backspace => {
                self.open_parent();
                PickerEvent::Pending
            }
            KeyCode::Char('r') => {
                self.refresh();
                PickerEvent::Pending
            }
            KeyCode::Char(' ') => PickerEvent::Selected(self.current.clone()),
            KeyCode::Enter | KeyCode::Right => self.activate(),
            _ => PickerEvent::Pending,
        }
    }

    /// 激活当前项目。
    fn activate(&mut self) -> PickerEvent {
        let Some(action) = self
            .entries
            .get(self.selected)
            .map(|entry| entry.action.clone())
        else {
            return PickerEvent::Pending;
        };
        match action {
            PathAction::SelectCurrent => PickerEvent::Selected(self.current.clone()),
            PathAction::Parent => {
                self.open_parent();
                PickerEvent::Pending
            }
            PathAction::Open(path) => {
                self.current = path;
                self.selected = 0;
                self.refresh();
                PickerEvent::Pending
            }
            PathAction::Select(path) => PickerEvent::Selected(path),
        }
    }

    /// 返回上级目录。
    fn open_parent(&mut self) {
        let Some(parent) = self.current.parent() else {
            return;
        };
        self.current = parent.to_path_buf();
        self.selected = 0;
        self.refresh();
    }

    /// 绘制内联路径浏览器。
    fn render(&self, frame: &mut Frame<'_>, area: Rect) {
        let header_height = if area.height >= 6 { 2 } else { 1 };
        let rows = ratatui::layout::Layout::vertical([
            ratatui::layout::Constraint::Length(header_height),
            ratatui::layout::Constraint::Min(1),
        ])
        .split(area);
        let message = self.error.as_deref().unwrap_or(&self.instruction);
        let header_width = usize::from(rows[0].width);
        let mut header = vec![Line::from(text_view::clipped(message, 0, header_width))];
        if header_height > 1 {
            header.push(Line::from(text_view::clipped(
                &format!("当前位置：{}", self.current.display()),
                0,
                header_width,
            )));
        }
        frame.render_widget(Paragraph::new(header), rows[0]);
        let item_width = usize::from(rows[1].width.saturating_sub(4));
        let items = self
            .entries
            .iter()
            .map(|entry| {
                ListItem::new(Line::from(Span::raw(text_view::clipped(
                    &entry.label,
                    0,
                    item_width,
                ))))
            })
            .collect::<Vec<_>>();
        let mut state = ListState::default().with_selected(Some(self.selected));
        let list = List::new(items)
            .highlight_symbol("› ")
            .highlight_style(
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )
            .block(
                Block::default()
                    .title(text_view::clipped(
                        &self.title,
                        0,
                        usize::from(rows[1].width.saturating_sub(4)),
                    ))
                    .title_bottom(key_hints::adaptive(
                        &[
                            "↑↓ 选择 · Enter 确认/进入 · Esc 取消".to_owned(),
                            "Enter确认/进入 · Esc取消".to_owned(),
                        ],
                        rows[1].width.saturating_sub(2),
                    ))
                    .borders(Borders::ALL),
            );
        frame.render_stateful_widget(list, rows[1], &mut state);
    }
}

/// 路径选择结果。
enum PickerEvent {
    Pending,
    Selected(PathBuf),
    Cancelled,
}

/// 以内联小 TUI 选择普通文件或目录。
pub(crate) fn select_path_inline(initial: Option<&Path>) -> io::Result<Option<PathBuf>> {
    select_path_inline_named(
        initial,
        "选择上传文件或文件夹",
        "Enter 打开文件夹或选择文件；Space 选择当前文件夹；Backspace 返回上级。",
    )
}

/// 以调用场景专属标题和说明运行路径浏览器。
pub(crate) fn select_path_inline_named(
    initial: Option<&Path>,
    title: &str,
    instruction: &str,
) -> io::Result<Option<PathBuf>> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err(io::Error::new(
            io::ErrorKind::NotConnected,
            "当前输入输出不是交互式终端",
        ));
    }
    let mut picker = PathPicker::new(initial, title, instruction)?;
    let mut terminal = InlineTerminal::new(18)?;
    let result = loop {
        terminal.draw(|frame| picker.render(frame, frame.area()))?;
        if let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            match picker.handle_key(key.code) {
                PickerEvent::Pending => {}
                PickerEvent::Selected(path) => break Some(path),
                PickerEvent::Cancelled => break None,
            }
        }
    };
    terminal.finish()?;
    Ok(result)
}
