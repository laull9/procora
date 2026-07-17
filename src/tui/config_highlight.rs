use std::sync::OnceLock;

use ratatui::{
    style::{Color, Modifier, Style},
    text::Span,
};
use syntect::{
    easy::HighlightLines,
    highlighting::{FontStyle, Style as SyntaxStyle, ThemeSet},
    parsing::{SyntaxDefinition, SyntaxSet},
};

use crate::config::ConfigFormat;

/// 高亮器共享的内建语法和主题资源。
struct HighlightAssets {
    syntaxes: SyntaxSet,
    themes: ThemeSet,
}

/// 全进程复用 syntect 资源，避免每次 TUI 重绘都解压内建定义。
static HIGHLIGHT_ASSETS: OnceLock<HighlightAssets> = OnceLock::new();

/// 按配置格式高亮完整文本，并保留跨行语法状态。
pub(crate) fn highlighted_lines(
    format: ConfigFormat,
    lines: impl IntoIterator<Item = String>,
) -> Vec<Vec<Span<'static>>> {
    let assets = HIGHLIGHT_ASSETS.get_or_init(|| HighlightAssets {
        syntaxes: syntax_set(),
        themes: ThemeSet::load_defaults(),
    });
    let syntax = assets
        .syntaxes
        .find_syntax_by_extension(extension(format))
        .unwrap_or_else(|| assets.syntaxes.find_syntax_plain_text());
    let theme = assets
        .themes
        .themes
        .get("base16-ocean.dark")
        .expect("syntect 内建主题应包含 base16-ocean.dark");
    let mut highlighter = HighlightLines::new(syntax, theme);

    lines
        .into_iter()
        .map(|line| {
            let source = format!("{line}\n");
            highlighter
                .highlight_line(&source, &assets.syntaxes)
                .map_or_else(
                    |_| vec![Span::raw(line)],
                    |regions| {
                        regions
                            .into_iter()
                            .filter_map(|(style, text)| {
                                let text = text.strip_suffix('\n').unwrap_or(text);
                                (!text.is_empty())
                                    .then(|| Span::styled(text.to_owned(), ratatui_style(style)))
                            })
                            .collect()
                    },
                )
        })
        .collect()
}

/// 在 syntect 默认语法之外补入其内建包未提供的 TOML grammar。
fn syntax_set() -> SyntaxSet {
    let mut builder = SyntaxSet::load_defaults_newlines().into_builder();
    let toml = SyntaxDefinition::load_from_str(
        include_str!("syntaxes/TOML.sublime-syntax"),
        true,
        Some("TOML"),
    )
    .expect("内嵌 TOML 语法定义应保持有效");
    builder.add(toml);
    builder.build()
}

/// 返回 syntect 内建语法识别使用的扩展名。
const fn extension(format: ConfigFormat) -> &'static str {
    match format {
        ConfigFormat::Yaml => "yaml",
        ConfigFormat::Toml => "toml",
        ConfigFormat::Json => "json",
    }
}

/// 把 syntect 颜色和字体属性转换为 Ratatui 样式。
fn ratatui_style(style: SyntaxStyle) -> Style {
    let mut modifiers = Modifier::empty();
    if style.font_style.contains(FontStyle::BOLD) {
        modifiers.insert(Modifier::BOLD);
    }
    if style.font_style.contains(FontStyle::ITALIC) {
        modifiers.insert(Modifier::ITALIC);
    }
    if style.font_style.contains(FontStyle::UNDERLINE) {
        modifiers.insert(Modifier::UNDERLINED);
    }
    Style::default()
        .fg(Color::Rgb(
            style.foreground.r,
            style.foreground.g,
            style.foreground.b,
        ))
        .add_modifier(modifiers)
}
