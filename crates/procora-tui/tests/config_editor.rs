//! 配置编辑页的文本操作、校验保存和退出保护测试。

use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use procora_config::ConfigFormat;
use procora_tui::ConfigEditor;
use ratatui::{Terminal, backend::TestBackend, buffer::Cell};

/// 创建当前测试独占的配置路径。
fn temporary_config() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "procora-editor-{}-{nonce}.yaml",
        std::process::id()
    ))
}

#[test]
fn 只有有效配置才会保存() {
    let path = temporary_config();
    let initial = "version: 1\nproject: demo\ntasks: {}\n";
    fs::write(&path, initial).unwrap();
    let mut editor = ConfigEditor::from_text(&path, ConfigFormat::Yaml, initial);
    editor.handle_key(KeyEvent::new(KeyCode::Char('!'), KeyModifiers::NONE));
    editor.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
    assert!(editor.message().starts_with("配置无效"));
    assert_eq!(fs::read_to_string(&path).unwrap(), initial);
    fs::remove_file(path).unwrap();
}

#[test]
fn 未保存修改需要二次确认退出() {
    let mut editor = ConfigEditor::from_text(
        "procora.yaml",
        ConfigFormat::Yaml,
        "version: 1\nproject: demo\ntasks: {}\n",
    );
    editor.handle_key(KeyEvent::new(KeyCode::Char('#'), KeyModifiers::NONE));
    editor.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(!editor.should_quit());
    assert!(editor.message().contains("再次按"));
    editor.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(editor.should_quit());
}

#[test]
fn 宽屏编辑页会显示配置内容与依赖引导() {
    let editor = ConfigEditor::from_text(
        "procora.yaml",
        ConfigFormat::Yaml,
        "version: 1\nproject: demo\ntasks: {}\n",
    );
    let backend = TestBackend::new(110, 28);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|frame| editor.render(frame)).unwrap();
    let text = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(Cell::symbol)
        .collect::<String>()
        .replace(' ', "");

    assert!(text.contains("Procora配置编辑器"));
    assert!(text.contains("version:1"));
    assert!(text.contains("管理依赖"));
    assert!(text.contains("${dependency.<id>}"));
}
