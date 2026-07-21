use std::io::{self, IsTerminal};

use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind},
    terminal::{disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal, TerminalOptions, Viewport,
    backend::CrosstermBackend,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
};

/// 可复用选择栏中的一个选项。
#[derive(Clone, Debug)]
pub struct SelectionItem<T> {
    label: String,
    description: String,
    value: T,
}

impl<T> SelectionItem<T> {
    /// 创建一个带标题、说明和返回值的选项。
    pub fn new(label: impl Into<String>, description: impl Into<String>, value: T) -> Self {
        Self {
            label: label.into(),
            description: description.into(),
            value,
        }
    }
}

/// 选择栏对一次按键的处理结果。
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SelectionEvent<T> {
    /// 选择仍在进行。
    Pending,
    /// 用户确认了一个选项。
    Selected(T),
    /// 用户取消了选择。
    Cancelled,
}

/// 支持上下导航、确认和取消的可复用选择状态。
#[derive(Clone, Debug)]
pub struct SelectionState<T> {
    items: Vec<SelectionItem<T>>,
    selected: usize,
}

impl<T: Clone> SelectionState<T> {
    /// 从至少一个选项创建选择状态。
    ///
    /// # Panics
    ///
    /// 当选项列表为空时触发，因为空选择栏无法导航或确认。
    pub fn new(items: Vec<SelectionItem<T>>) -> Self {
        assert!(!items.is_empty(), "选择栏至少需要一个选项");
        Self { items, selected: 0 }
    }

    /// 返回当前高亮选项序号。
    pub const fn selected(&self) -> usize {
        self.selected
    }

    /// 返回选项数量。
    pub const fn len(&self) -> usize {
        self.items.len()
    }

    /// 返回选择栏是否没有选项；有效状态始终为否。
    pub const fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// 处理上下、Home/End、Enter 与 Esc。
    pub fn handle_key(&mut self, key: KeyEvent) -> SelectionEvent<T> {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.selected = self.selected.saturating_sub(1);
                SelectionEvent::Pending
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.selected = (self.selected + 1).min(self.items.len() - 1);
                SelectionEvent::Pending
            }
            KeyCode::Home => {
                self.selected = 0;
                SelectionEvent::Pending
            }
            KeyCode::End => {
                self.selected = self.items.len() - 1;
                SelectionEvent::Pending
            }
            KeyCode::Enter => SelectionEvent::Selected(self.items[self.selected].value.clone()),
            KeyCode::Esc | KeyCode::Char('q') => SelectionEvent::Cancelled,
            _ => SelectionEvent::Pending,
        }
    }

    /// 绘制带边框和统一键位提示的选择栏。
    pub fn render(&self, frame: &mut Frame<'_>, area: Rect, title: &str, message: &str) {
        let rows = ratatui::layout::Layout::vertical([
            ratatui::layout::Constraint::Length(2),
            ratatui::layout::Constraint::Min(1),
        ])
        .split(area);
        frame.render_widget(Paragraph::new(message).wrap(Wrap { trim: false }), rows[0]);
        let items = self
            .items
            .iter()
            .map(|item| {
                ListItem::new(Line::from(vec![
                    ratatui::text::Span::styled(
                        item.label.clone(),
                        Style::default().add_modifier(Modifier::BOLD),
                    ),
                    ratatui::text::Span::raw(format!("  {}", item.description)),
                ]))
            })
            .collect::<Vec<_>>();
        let mut state = ListState::default().with_selected(Some(self.selected));
        let list = List::new(items)
            .highlight_symbol("› ")
            .highlight_style(Style::default().fg(Color::Cyan))
            .block(
                Block::default()
                    .title(title)
                    .title_bottom("↑↓ 选择 · Enter 确认 · Esc 取消")
                    .borders(Borders::ALL),
            );
        frame.render_stateful_widget(list, rows[1], &mut state);
    }
}

/// 原始模式恢复守卫。
struct RawModeGuard;

impl RawModeGuard {
    /// 进入终端原始模式。
    fn enable() -> io::Result<Self> {
        enable_raw_mode()?;
        Ok(Self)
    }
}

impl Drop for RawModeGuard {
    /// 尽力恢复普通终端输入模式。
    fn drop(&mut self) {
        let _ = disable_raw_mode();
    }
}

/// 在普通 CLI 输出流中运行一个不占满屏幕的选择栏。
///
/// # Errors
///
/// 当输入输出不是交互式终端，或原始模式、绘制和输入读取失败时返回错误。
pub fn select_inline<T: Clone>(
    title: &str,
    message: &str,
    items: Vec<SelectionItem<T>>,
) -> io::Result<Option<T>> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err(io::Error::new(
            io::ErrorKind::NotConnected,
            "当前输入输出不是交互式终端",
        ));
    }
    let mut state = SelectionState::new(items);
    let height = u16::try_from(state.len().saturating_add(5)).unwrap_or(u16::MAX);
    let _raw_mode = RawModeGuard::enable()?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::with_options(
        backend,
        TerminalOptions {
            viewport: Viewport::Inline(height),
        },
    )?;
    terminal.hide_cursor()?;
    let result = loop {
        terminal.draw(|frame| state.render(frame, frame.area(), title, message))?;
        match event::read()? {
            Event::Key(key) if key.kind == KeyEventKind::Press => match state.handle_key(key) {
                SelectionEvent::Pending => {}
                SelectionEvent::Selected(value) => break Some(value),
                SelectionEvent::Cancelled => break None,
            },
            _ => {}
        }
    };
    terminal.show_cursor()?;
    terminal.clear()?;
    Ok(result)
}
