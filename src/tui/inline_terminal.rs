//! 内联 TUI 的终端初始化与原位清理。

use std::{
    io::{self, Stdout},
    ops::{Deref, DerefMut},
};

use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use ratatui::{
    Terminal, TerminalOptions, Viewport,
    backend::{Backend, CrosstermBackend},
};

/// 内联 TUI 使用的标准输出终端。
type StdoutTerminal = Terminal<CrosstermBackend<Stdout>>;

/// 在离开作用域时恢复普通终端输入模式。
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

/// 退出时清除占用行并把光标放回原始起始行的内联终端。
pub(crate) struct InlineTerminal {
    terminal: StdoutTerminal,
    _raw_mode: RawModeGuard,
    cleaned: bool,
}

impl InlineTerminal {
    /// 创建指定高度的内联终端并隐藏光标。
    pub(crate) fn new(height: u16) -> io::Result<Self> {
        let raw_mode = RawModeGuard::enable()?;
        let backend = CrosstermBackend::new(io::stdout());
        let mut terminal = Terminal::with_options(
            backend,
            TerminalOptions {
                viewport: Viewport::Inline(height),
            },
        )?;
        terminal.hide_cursor()?;
        Ok(Self {
            terminal,
            _raw_mode: raw_mode,
            cleaned: false,
        })
    }

    /// 清除内联区域并恢复光标，成功后不再在析构时重复清理。
    pub(crate) fn finish(mut self) -> io::Result<()> {
        self.cleanup()?;
        self.cleaned = true;
        Ok(())
    }

    /// 原位清理内联区域，不让 Ratatui 预留的行留在后续 CLI 输出之前。
    fn cleanup(&mut self) -> io::Result<()> {
        cleanup_terminal(&mut self.terminal)
    }
}

impl Deref for InlineTerminal {
    type Target = StdoutTerminal;

    /// 访问底层 Ratatui 终端。
    fn deref(&self) -> &Self::Target {
        &self.terminal
    }
}

impl DerefMut for InlineTerminal {
    /// 可变访问底层 Ratatui 终端。
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.terminal
    }
}

impl Drop for InlineTerminal {
    /// 错误或提前退出时也尽力清除内联区域并恢复光标。
    fn drop(&mut self) {
        if !self.cleaned {
            let _ = self.cleanup();
        }
    }
}

/// 清空内联视口并把光标复位到视口左上角。
fn cleanup_terminal<B: Backend>(terminal: &mut Terminal<B>) -> Result<(), B::Error> {
    let area = terminal.get_frame().area();
    let origin = (area.x, area.y);
    terminal.clear()?;
    terminal.set_cursor_position(origin)?;
    terminal.show_cursor()
}

#[cfg(test)]
mod tests {
    use ratatui::{
        Terminal, TerminalOptions, Viewport,
        backend::{Backend, TestBackend},
        layout::Position,
        widgets::Paragraph,
    };

    use super::cleanup_terminal;

    #[test]
    // 内联TUI清理后复用进入时的起始行，而不是停留在预留区域末尾。
    fn cleanup_restores_inline_viewport_origin() {
        let mut backend = TestBackend::new(20, 10);
        backend.set_cursor_position(Position::new(0, 3)).unwrap();
        let mut terminal = Terminal::with_options(
            backend,
            TerminalOptions {
                viewport: Viewport::Inline(4),
            },
        )
        .unwrap();
        terminal
            .draw(|frame| frame.render_widget(Paragraph::new("inline"), frame.area()))
            .unwrap();

        cleanup_terminal(&mut terminal).unwrap();

        terminal
            .backend_mut()
            .assert_cursor_position(Position::new(0, 3));
        assert!(
            terminal
                .backend()
                .buffer()
                .content()
                .iter()
                .all(|cell| cell.symbol() == " ")
        );
    }
}
