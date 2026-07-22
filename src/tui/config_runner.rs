//! 独立配置编辑命令的终端事件循环。

use std::{io, path::Path, time::Instant};

use crossterm::event::{self, Event, KeyEventKind};

use super::{ConfigEditor, INPUT_MAX_WAIT};

/// 打开一个带字段引导和保存前校验的配置编辑页面。
///
/// # Errors
///
/// 当配置文件无法读取或终端无法切换到 TUI 时返回错误。
pub fn edit_config(path: &Path) -> io::Result<()> {
    let mut editor = ConfigEditor::open(path)?;
    ratatui::run(|terminal| {
        let mut dirty = true;
        let mut last_auto_scroll = Instant::now();
        loop {
            if dirty {
                terminal.draw(|frame| editor.render(frame))?;
                dirty = false;
            }
            if event::poll(INPUT_MAX_WAIT)? {
                match event::read()? {
                    Event::Key(key) if key.kind == KeyEventKind::Press => {
                        editor.handle_key(key);
                        dirty = true;
                    }
                    Event::Resize(_, _) => dirty = true,
                    _ => {}
                }
            }
            let now = Instant::now();
            let elapsed = now.saturating_duration_since(last_auto_scroll);
            last_auto_scroll = now;
            dirty |= editor.advance_auto_scroll(elapsed);
            if editor.should_quit() {
                break Ok(());
            }
        }
    })
}
