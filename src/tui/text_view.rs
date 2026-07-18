//! TUI 单行文本的宽度计算、折叠和水平视口辅助。

use ratatui::text::Line;

/// 单行输入框内可见的文本与光标列。
pub(crate) struct InputView {
    /// 已按终端宽度折叠的文本。
    pub(crate) text: String,
    /// 光标相对可见文本起点的显示列。
    pub(crate) cursor_x: usize,
}

/// 返回文本占用的终端显示列数。
pub(crate) fn width(text: &str) -> usize {
    Line::from(text).width()
}

/// 从字符偏移处显示单行文本，并在被截断的方向放置省略号。
pub(crate) fn clipped(text: &str, offset: usize, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }
    let characters = text.chars().collect::<Vec<_>>();
    let start = offset.min(characters.len());
    let prefix = usize::from(start > 0);
    let mut result = if prefix == 1 {
        "…".to_owned()
    } else {
        String::new()
    };
    let mut used = prefix;
    let mut end = start;
    while end < characters.len() {
        let character = characters[end];
        let character_width = width(&character.to_string());
        let needs_suffix = end + 1 < characters.len();
        let reserved = usize::from(needs_suffix);
        if used
            .saturating_add(character_width)
            .saturating_add(reserved)
            > max_width
        {
            break;
        }
        result.push(character);
        used += character_width;
        end += 1;
    }
    if end < characters.len() && used < max_width {
        result.push('…');
    }
    result
}

/// 生成始终包含指定字符光标的单行输入视口。
pub(crate) fn input_view(text: &str, cursor: usize, max_width: usize) -> InputView {
    if max_width == 0 {
        return InputView {
            text: String::new(),
            cursor_x: 0,
        };
    }
    let characters = text.chars().collect::<Vec<_>>();
    let cursor = cursor.min(characters.len());
    let mut start = 0;
    while start < cursor {
        let prefix = usize::from(start > 0);
        let cursor_width = characters[start..cursor]
            .iter()
            .map(|character| width(&character.to_string()))
            .sum::<usize>();
        if prefix.saturating_add(cursor_width) < max_width {
            break;
        }
        start += 1;
    }
    let text = clipped(text, start, max_width);
    let prefix = usize::from(start > 0);
    let cursor_x = prefix
        + characters[start..cursor]
            .iter()
            .map(|character| width(&character.to_string()))
            .sum::<usize>();
    InputView {
        text,
        cursor_x: cursor_x.min(max_width.saturating_sub(1)),
    }
}
