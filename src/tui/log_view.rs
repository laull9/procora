//! ANSI 日志解析、纯文本匹配和 Ratatui 文本构造。

use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
};

/// 已解析的一行日志及其不含控制序列的匹配文本。
#[derive(Clone, Debug, Default)]
struct StyledLine {
    plain: String,
    spans: Vec<Span<'static>>,
}

/// 返回日志在当前搜索与过滤条件下的可见纯文本行。
pub(crate) fn visible_lines(raw: &[u8], query: &str, filtered: bool) -> Vec<String> {
    parse(raw)
        .into_iter()
        .filter(|line| !filtered || line_matches(&line.plain, query))
        .map(|line| line.plain)
        .collect()
}

/// 返回搜索条件在当前可见行中的索引。
pub(crate) fn match_lines(raw: &[u8], query: &str, filtered: bool) -> Vec<usize> {
    if query.is_empty() {
        return Vec::new();
    }
    visible_lines(raw, query, filtered)
        .iter()
        .enumerate()
        .filter_map(|(index, line)| line_matches(line, query).then_some(index))
        .collect()
}

/// 把原始 ANSI 日志转换为可直接渲染的样式文本。
pub(crate) fn styled_text(
    raw: &[u8],
    query: &str,
    filtered: bool,
    active_match_line: Option<usize>,
    plain_mode: bool,
) -> Text<'static> {
    let mut visible_index = 0;
    let lines = parse(raw)
        .into_iter()
        .filter_map(|line| {
            let matches = line_matches(&line.plain, query);
            if filtered && !matches {
                return None;
            }
            let line_style = if active_match_line == Some(visible_index) {
                Style::default().add_modifier(Modifier::REVERSED)
            } else if matches && !query.is_empty() {
                Style::default().add_modifier(Modifier::UNDERLINED)
            } else {
                Style::default()
            };
            visible_index += 1;
            let spans = if plain_mode {
                vec![Span::raw(line.plain)]
            } else {
                line.spans
            };
            Some(Line::from(spans).style(line_style))
        })
        .collect::<Vec<_>>();
    Text::from(lines)
}

/// 执行不区分 ASCII 大小写的日志行包含匹配。
pub(crate) fn line_matches(line: &str, query: &str) -> bool {
    query.is_empty()
        || line
            .to_ascii_lowercase()
            .contains(&query.to_ascii_lowercase())
}

/// 解析常见 CSI SGR 颜色与文本修饰，同时丢弃其他终端控制序列。
fn parse(raw: &[u8]) -> Vec<StyledLine> {
    if raw.is_empty() {
        return Vec::new();
    }
    let text = String::from_utf8_lossy(raw);
    let characters = text.chars().collect::<Vec<_>>();
    let mut lines = Vec::new();
    let mut line = StyledLine::default();
    let mut segment = String::new();
    let mut style = Style::default();
    let mut index = 0;
    while index < characters.len() {
        match characters[index] {
            '\n' => {
                push_segment(&mut line, &mut segment, style);
                lines.push(std::mem::take(&mut line));
                index += 1;
            }
            '\r' => index += 1,
            '\u{1b}' => {
                push_segment(&mut line, &mut segment, style);
                index = consume_escape(&characters, index, &mut style);
            }
            character if character.is_control() && character != '\t' => index += 1,
            character => {
                line.plain.push(character);
                segment.push(character);
                index += 1;
            }
        }
    }
    push_segment(&mut line, &mut segment, style);
    if !line.plain.is_empty() || !line.spans.is_empty() || !raw.ends_with(b"\n") {
        lines.push(line);
    }
    lines
}

/// 把当前同样式文本片段提交到行中。
fn push_segment(line: &mut StyledLine, segment: &mut String, style: Style) {
    if !segment.is_empty() {
        line.spans
            .push(Span::styled(std::mem::take(segment), style));
    }
}

/// 消费一个 ANSI 转义序列并返回下一字符位置。
fn consume_escape(characters: &[char], start: usize, style: &mut Style) -> usize {
    let Some(prefix) = characters.get(start + 1).copied() else {
        return start + 1;
    };
    if prefix == '[' {
        let mut end = start + 2;
        while end < characters.len() && !('\u{40}'..='\u{7e}').contains(&characters[end]) {
            end += 1;
        }
        if characters.get(end) == Some(&'m') {
            let parameters = characters[start + 2..end].iter().collect::<String>();
            apply_sgr(style, &parameters);
        }
        return (end + 1).min(characters.len());
    }
    if prefix == ']' {
        let mut end = start + 2;
        while end < characters.len() {
            if characters[end] == '\u{7}' {
                return end + 1;
            }
            if characters[end] == '\u{1b}' && characters.get(end + 1) == Some(&'\\') {
                return end + 2;
            }
            end += 1;
        }
        return characters.len();
    }
    (start + 2).min(characters.len())
}

/// 应用一组 SGR 参数。
fn apply_sgr(style: &mut Style, parameters: &str) {
    let values = if parameters.is_empty() {
        vec![0]
    } else {
        parameters
            .split(';')
            .map(|value| value.parse::<u16>().unwrap_or(0))
            .collect::<Vec<_>>()
    };
    let mut index = 0;
    while index < values.len() {
        let value = values[index];
        match value {
            0 => *style = Style::default(),
            1 => *style = style.add_modifier(Modifier::BOLD),
            2 => *style = style.add_modifier(Modifier::DIM),
            3 => *style = style.add_modifier(Modifier::ITALIC),
            4 => *style = style.add_modifier(Modifier::UNDERLINED),
            5 => *style = style.add_modifier(Modifier::SLOW_BLINK),
            7 => *style = style.add_modifier(Modifier::REVERSED),
            9 => *style = style.add_modifier(Modifier::CROSSED_OUT),
            22 => *style = style.remove_modifier(Modifier::BOLD | Modifier::DIM),
            23 => *style = style.remove_modifier(Modifier::ITALIC),
            24 => *style = style.remove_modifier(Modifier::UNDERLINED),
            25 => *style = style.remove_modifier(Modifier::SLOW_BLINK | Modifier::RAPID_BLINK),
            27 => *style = style.remove_modifier(Modifier::REVERSED),
            29 => *style = style.remove_modifier(Modifier::CROSSED_OUT),
            30..=37 | 90..=97 => *style = style.fg(basic_color(value)),
            39 => *style = style.fg(Color::Reset),
            40..=47 | 100..=107 => *style = style.bg(basic_color(value - 10)),
            49 => *style = style.bg(Color::Reset),
            38 | 48 => {
                let foreground = value == 38;
                if let Some((color, consumed)) = extended_color(&values[index + 1..]) {
                    *style = if foreground {
                        style.fg(color)
                    } else {
                        style.bg(color)
                    };
                    index += consumed;
                }
            }
            _ => {}
        }
        index += 1;
    }
}

/// 把标准与高亮八色 SGR 代码映射到 Ratatui 颜色。
const fn basic_color(code: u16) -> Color {
    match code {
        30 => Color::Black,
        31 => Color::Red,
        32 => Color::Green,
        33 => Color::Yellow,
        34 => Color::Blue,
        35 => Color::Magenta,
        36 => Color::Cyan,
        37 => Color::Gray,
        90 => Color::DarkGray,
        91 => Color::LightRed,
        92 => Color::LightGreen,
        93 => Color::LightYellow,
        94 => Color::LightBlue,
        95 => Color::LightMagenta,
        96 => Color::LightCyan,
        _ => Color::White,
    }
}

/// 解析 256 色或 RGB 扩展色并返回额外消费的参数数量。
fn extended_color(values: &[u16]) -> Option<(Color, usize)> {
    match values {
        [5, color, ..] => Some((Color::Indexed(u8::try_from(*color).ok()?), 2)),
        [2, red, green, blue, ..] => Some((
            Color::Rgb(
                u8::try_from(*red).ok()?,
                u8::try_from(*green).ok()?,
                u8::try_from(*blue).ok()?,
            ),
            4,
        )),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use ratatui::style::Color;

    use super::{parse, visible_lines};

    #[test]
    // ANSI颜色不会进入搜索文本，同时会保留到渲染Span。
    fn ansi_colors_are_styled_and_searchable() {
        let lines = parse(b"normal \x1b[31merror\x1b[0m\n");
        assert_eq!(lines[0].plain, "normal error");
        assert_eq!(lines[0].spans[1].style.fg, Some(Color::Red));
        assert_eq!(
            visible_lines(b"ok\n\x1b[31mERROR\x1b[0m\n", "error", true),
            ["ERROR"]
        );
    }
}
