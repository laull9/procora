//! 管理依赖一行配置与 TUI 渐进式编辑测试。

use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use procora::{config::load_path, tui::ConfigEditor};

/// 当前测试进程内的临时目录序号。
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// 创建当前测试独占的目录。
fn temporary_directory(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let directory = std::env::temp_dir().join(format!(
        "procora-tui-remote-{label}-{}-{nonce}-{sequence}",
        std::process::id()
    ));
    fs::create_dir_all(&directory).unwrap();
    directory
}

/// 向结构化编辑器发送无修饰按键。
fn press(editor: &mut ConfigEditor, code: KeyCode) {
    editor.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
}

/// 向当前字段逐字输入。
fn type_text(editor: &mut ConfigEditor, value: &str) {
    for character in value.chars() {
        press(editor, KeyCode::Char(character));
    }
}

/// 保存当前编辑器。
fn save(editor: &mut ConfigEditor) {
    editor.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
    assert!(
        editor.message().starts_with("已保存"),
        "{}",
        editor.message()
    );
}

#[test]
// 新建HTTP依赖只输入名称和来源，默认保存成一行配置。
fn tui_creates_one_line_dependency_with_two_inputs() {
    let root = temporary_directory("create");
    let path = root.join("procora.yaml");
    fs::write(&path, "version: 1\nproject: demo\ntasks: {}\n").unwrap();
    let mut editor = ConfigEditor::open(&path).unwrap();

    press(&mut editor, KeyCode::Tab);
    press(&mut editor, KeyCode::Tab);
    press(&mut editor, KeyCode::Char('n'));
    type_text(&mut editor, "tool");
    press(&mut editor, KeyCode::Tab);
    type_text(&mut editor, "https://example.com/tool.tar.gz");
    press(&mut editor, KeyCode::Enter);
    save(&mut editor);

    let saved = fs::read_to_string(&path).unwrap();
    let compiled = load_path(&path).unwrap();
    assert!(saved.contains("\"tool\": \"https://example.com/tool.tar.gz\""));
    assert_eq!(compiled.dependencies["tool"].version, "source");
    assert_eq!(compiled.dependencies["tool"].download.retries, 2);
    fs::remove_dir_all(root).unwrap();
}

#[test]
// a键只编辑高级策略，常用字段和高级字段互不丢失。
fn tui_advanced_policy_is_separate_and_preserved() {
    let root = temporary_directory("advanced");
    let path = root.join("procora.yaml");
    fs::write(
        &path,
        "version: 1\nproject: demo\ndependencies:\n  tool: ssh://release@example.com/opt/tool\ntasks: {}\n",
    )
    .unwrap();
    let mut editor = ConfigEditor::open(&path).unwrap();

    press(&mut editor, KeyCode::Tab);
    press(&mut editor, KeyCode::Tab);
    press(&mut editor, KeyCode::Char('a'));
    press(&mut editor, KeyCode::Tab);
    press(&mut editor, KeyCode::Tab);
    press(&mut editor, KeyCode::Backspace);
    type_text(&mut editor, "4");
    press(&mut editor, KeyCode::Enter);
    press(&mut editor, KeyCode::Enter);
    press(&mut editor, KeyCode::Enter);
    save(&mut editor);

    let saved = fs::read_to_string(&path).unwrap();
    let compiled = load_path(&path).unwrap();
    assert_eq!(compiled.dependencies["tool"].download.retries, 4);
    assert_eq!(
        compiled.dependencies["tool"].source,
        "ssh://release@example.com/opt/tool"
    );
    assert!(saved.contains("retries: 4"));
    assert!(!saved.contains("timeout:"));
    assert!(!saved.contains("max_bytes:"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
// YAML、TOML和JSON的一行依赖经过TUI保存后仍保持简写。
fn tui_preserves_one_line_dependency_across_formats() {
    for (extension, input, expected) in [
        (
            "yaml",
            "version: 1\nproject: demo\ndependencies:\n  asset: https://example.com/asset.bin\ntasks: {}\n",
            "\"asset\": \"https://example.com/asset.bin\"",
        ),
        (
            "toml",
            "version = 1\nproject = 'demo'\n[dependencies]\nasset = 'https://example.com/asset.bin'\n[tasks]\n",
            "asset = \"https://example.com/asset.bin\"",
        ),
        (
            "json",
            r#"{"version":1,"project":"demo","dependencies":{"asset":"https://example.com/asset.bin"},"tasks":{}}"#,
            "\"asset\": \"https://example.com/asset.bin\"",
        ),
    ] {
        let root = temporary_directory(extension);
        let path = root.join(format!("procora.{extension}"));
        fs::write(&path, input).unwrap();
        let mut editor = ConfigEditor::open(&path).unwrap();

        save(&mut editor);

        let saved = fs::read_to_string(&path).unwrap();
        assert!(saved.contains(expected), "{extension}: {saved}");
        assert_eq!(
            load_path(&path).unwrap().dependencies["asset"].version,
            "source"
        );
        fs::remove_dir_all(root).unwrap();
    }
}
