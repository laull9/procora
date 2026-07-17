//! 命令文本在结构化 TUI 中的输入与精确 argv 保存契约。

use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use procora::tui::ConfigEditor;

/// 并行测试创建独占配置时使用的进程内序号。
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// 创建当前测试独占的配置路径。
fn temporary_config() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "procora-command-text-{}-{nonce}-{sequence}.yaml",
        std::process::id()
    ))
}

/// 向编辑器发送一个无修饰键。
fn press(editor: &mut ConfigEditor, code: KeyCode) {
    editor.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
}

/// 向当前表单字段逐字输入文本。
fn type_text(editor: &mut ConfigEditor, value: &str) {
    for character in value.chars() {
        press(editor, KeyCode::Char(character));
    }
}

#[test]
// Task弹窗可直接输入带引号和Windows反斜杠路径的完整命令文本。
fn task_dialog_accepts_command_text_with_embedded_args() {
    let path = temporary_config();
    fs::write(
        &path,
        "version: 1\nproject: demo\ntasks:\n  api:\n    command: api\n",
    )
    .unwrap();
    let mut editor = ConfigEditor::open(&path).unwrap();

    press(&mut editor, KeyCode::Right);
    press(&mut editor, KeyCode::Enter);
    press(&mut editor, KeyCode::Tab);
    for _ in 0..3 {
        press(&mut editor, KeyCode::Backspace);
    }
    type_text(&mut editor, r#"runner "hello world" "" C:\tools\app"#);
    press(&mut editor, KeyCode::Enter);
    editor.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));

    let saved = fs::read_to_string(&path).unwrap();
    let compiled = procora::config::load_path(&path).unwrap();
    let task = compiled.spec.tasks.values().next().unwrap();
    assert_eq!(task.command, "runner");
    assert_eq!(task.args, ["hello world", "", r"C:\tools\app"]);
    assert!(saved.contains(r#"command: ["runner","hello world","","C:\\tools\\app"]"#));
    fs::remove_file(path).unwrap();
}
