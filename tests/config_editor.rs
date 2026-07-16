//! 配置编辑页的文本操作、校验保存和退出保护测试。

use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use procora::config::ConfigFormat;
use procora::tui::ConfigEditor;
use ratatui::{Terminal, backend::TestBackend, buffer::Cell};

/// 同一进程内并行创建临时配置时使用的去重序号。
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// 创建当前测试独占的配置路径。
fn temporary_config() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "procora-editor-{}-{nonce}-{sequence}.yaml",
        std::process::id()
    ))
}

#[test]
// 只有有效配置才会保存。
fn only_valid_config_is_saved() {
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
// 未保存修改需要二次确认退出。
fn unsaved_changes_require_exit_confirmation() {
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
// 宽屏编辑页会显示配置内容与依赖引导。
fn wide_editor_shows_config_and_dependency_guidance() {
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

#[test]
// include入口保持文本模式并按完整闭包校验保存。
fn include_entry_stays_text_mode_and_validates_full_closure() {
    let path = temporary_config();
    let fragment = path.with_file_name("procora-editor-fragment.yaml");
    fs::write(&fragment, "tasks:\n  worker:\n    command: worker\n").unwrap();
    fs::write(
        &path,
        "include: [procora-editor-fragment.yaml]\nversion: 1\nproject: demo\ntasks: {}\n",
    )
    .unwrap();
    let mut editor = ConfigEditor::open(&path).unwrap();

    assert!(editor.message().contains("include 配置"));
    editor.handle_key(KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE));
    assert!(editor.message().contains("避免表单展开"));
    editor.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
    assert!(editor.message().starts_with("已保存"));
    assert!(fs::read_to_string(&path).unwrap().contains("include:"));

    fs::remove_file(path).unwrap();
    fs::remove_file(fragment).unwrap();
}

#[test]
// 打开有效配置后可通过表单新建并保存_task。
fn form_can_create_and_save_task_from_valid_config() {
    let path = temporary_config();
    fs::write(&path, "version: 1\nproject: demo\ntasks: {}\n").unwrap();
    let mut editor = ConfigEditor::open(&path).unwrap();

    editor.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
    editor.handle_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE));
    for character in "worker".chars() {
        editor.handle_key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE));
    }
    editor.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    for character in "echo".chars() {
        editor.handle_key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE));
    }
    for _ in 0..7 {
        editor.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    }
    editor.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    editor.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));

    let saved = fs::read_to_string(&path).unwrap();
    let compiled = procora::config::load_str(&saved, ConfigFormat::Yaml).unwrap();
    assert!(compiled.spec.tasks.contains_key(&"worker".parse().unwrap()));
    assert!(editor.message().starts_with("已保存"));
    fs::remove_file(path).unwrap();
}

#[test]
// 表单界面会显示任务和管理依赖弹窗入口。
fn form_shows_task_and_dependency_dialog_entries() {
    let path = temporary_config();
    fs::write(&path, "version: 1\nproject: demo\ntasks: {}\n").unwrap();
    let mut editor = ConfigEditor::open(&path).unwrap();
    editor.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
    editor.handle_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE));
    let backend = TestBackend::new(110, 30);
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

    assert!(text.contains("结构化表单"));
    assert!(text.contains("新建Task"));
    assert!(text.contains("重启策略"));
    fs::remove_file(path).unwrap();
}

#[test]
// 表单保存支持_json_和_toml。
fn form_save_supports_json_and_toml() {
    for (extension, content, format) in [
        (
            "json",
            "{\"version\":1,\"project\":\"demo\",\"tasks\":{}}",
            ConfigFormat::Json,
        ),
        (
            "toml",
            "version = 1\nproject = \"demo\"\n[tasks]\n",
            ConfigFormat::Toml,
        ),
    ] {
        let path = temporary_config().with_extension(extension);
        fs::write(&path, content).unwrap();
        let mut editor = ConfigEditor::open(&path).unwrap();
        editor.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
        assert!(procora::config::load_str(&fs::read_to_string(&path).unwrap(), format).is_ok());
        fs::remove_file(path).unwrap();
    }
}

#[test]
// 表单往返不会丢失健康检查。
fn form_roundtrip_preserves_health_checks() {
    let path = temporary_config();
    let initial = "version: 1\nproject: demo\ntasks:\n  api:\n    command: api\n    healthcheck:\n      command: checker\n      args: ['--ready']\n      period_ms: 25\n      timeout_ms: 10\n      success_threshold: 2\n      failure_threshold: 4\n";
    fs::write(&path, initial).unwrap();
    let mut editor = ConfigEditor::open(&path).unwrap();

    editor.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));

    let saved = fs::read_to_string(&path).unwrap();
    let compiled = procora::config::load_str(&saved, ConfigFormat::Yaml).unwrap();
    let healthcheck = compiled
        .spec
        .tasks
        .values()
        .next()
        .unwrap()
        .healthcheck
        .as_ref()
        .unwrap();
    assert_eq!(healthcheck.args, ["--ready"]);
    assert_eq!(healthcheck.success_threshold, 2);
    assert_eq!(healthcheck.failure_threshold, 4);
    fs::remove_file(path).unwrap();
}

#[test]
// 表单删除确认可以用_esc_取消而不退出编辑器。
fn escape_cancels_form_delete_without_exiting() {
    let path = temporary_config();
    fs::write(
        &path,
        "version: 1\nproject: demo\ntasks:\n  worker:\n    command: echo\n",
    )
    .unwrap();
    let mut editor = ConfigEditor::open(&path).unwrap();
    editor.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
    editor.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE));
    editor.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

    assert!(!editor.should_quit());
    assert!(editor.message().contains("取消删除"));
    fs::remove_file(path).unwrap();
}
