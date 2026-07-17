//! 项目级 Task 默认层的结构化编辑器往返测试。

use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use procora::{
    config::{ValueOrigin, load_path},
    core::RestartPolicy,
    tui::ConfigEditor,
};

/// 并行测试创建独占目录时使用的进程内序号。
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// 创建当前测试独占的临时目录。
fn temporary_directory(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let directory = std::env::temp_dir().join(format!(
        "procora-task-defaults-{label}-{}-{nonce}-{sequence}",
        std::process::id()
    ));
    fs::create_dir_all(&directory).unwrap();
    directory
}

/// 向配置表单发送一个无修饰键。
fn press(editor: &mut ConfigEditor, code: KeyCode) {
    editor.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
}

/// 向当前配置表单字段逐字输入。
fn type_text(editor: &mut ConfigEditor, value: &str) {
    for character in value.chars() {
        press(editor, KeyCode::Char(character));
    }
}

/// 清空当前普通文本字段；多余退格安全无效。
fn clear_field(editor: &mut ConfigEditor) {
    for _ in 0..256 {
        press(editor, KeyCode::Backspace);
    }
}

#[test]
// 项目弹窗修改Task默认值后刷新有效Task，保存仍只写一份默认声明。
fn project_dialog_edits_defaults_without_expanding_tasks() {
    let root = temporary_directory("edit");
    let path = root.join("procora.yaml");
    fs::write(
        &path,
        "version: 1\nproject: demo\ntask_defaults:\n  cwd: work\n  env: {SHARED: default}\n  restart: on-failure\ntasks:\n  api:\n    command: api\n",
    )
    .unwrap();
    let mut editor = ConfigEditor::open(&path).unwrap();

    press(&mut editor, KeyCode::Enter);
    for _ in 0..5 {
        press(&mut editor, KeyCode::Tab);
    }
    press(&mut editor, KeyCode::Right);
    press(&mut editor, KeyCode::Enter);
    editor.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));

    let saved = fs::read_to_string(&path).unwrap();
    let compiled = load_path(&path).unwrap();
    let id = "api".parse().unwrap();
    assert_eq!(saved.matches("restart: always").count(), 1);
    assert_eq!(saved.matches("cwd:").count(), 1);
    assert_eq!(compiled.spec.tasks[&id].restart, RestartPolicy::Always);
    assert_eq!(
        compiled.task_origins[&id].field("restart"),
        ValueOrigin::TaskDefaults
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
// 新建Task使用项目默认有效值，但自身只保存名称和命令声明。
fn new_task_inherits_defaults_without_copying_them() {
    let root = temporary_directory("new-task");
    let path = root.join("procora.yaml");
    fs::write(
        &path,
        "version: 1\nproject: demo\ntask_defaults:\n  restart: on-failure\n  max_restarts: 3\ntasks: {}\n",
    )
    .unwrap();
    let mut editor = ConfigEditor::open(&path).unwrap();

    press(&mut editor, KeyCode::Right);
    press(&mut editor, KeyCode::Char('n'));
    type_text(&mut editor, "worker");
    press(&mut editor, KeyCode::Tab);
    type_text(&mut editor, "worker");
    press(&mut editor, KeyCode::Enter);
    editor.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));

    let saved = fs::read_to_string(&path).unwrap();
    let compiled = load_path(&path).unwrap();
    let task = compiled.spec.tasks.values().next().unwrap();
    assert_eq!(saved.matches("restart: on-failure").count(), 1);
    assert_eq!(saved.matches("max_restarts: 3").count(), 1);
    assert_eq!(task.restart, RestartPolicy::OnFailure);
    assert_eq!(task.max_restarts, 3);
    fs::remove_dir_all(root).unwrap();
}

#[test]
// 三种声明式格式通过结构化TUI保存后仍保留完整默认层和来源语义。
fn form_roundtrip_preserves_all_defaults_across_formats() {
    for (extension, content) in [
        (
            "yaml",
            "version: 1\nproject: demo\ntask_defaults:\n  cwd: work\n  env: {SHARED: default}\n  success_exit_codes: [0, 130]\n  restart: always\n  restart_delay_ms: 700\n  max_restarts: 4\n  restart_reset_after_ms: 8000\n  shutdown_timeout_ms: 3000\ntasks:\n  api: {command: api}\n",
        ),
        (
            "toml",
            "version = 1\nproject = 'demo'\n[task_defaults]\ncwd = 'work'\nsuccess_exit_codes = [0, 130]\nrestart = 'always'\nrestart_delay_ms = 700\nmax_restarts = 4\nrestart_reset_after_ms = 8000\nshutdown_timeout_ms = 3000\n[task_defaults.env]\nSHARED = 'default'\n[tasks.api]\ncommand = 'api'\n",
        ),
        (
            "json",
            r#"{"version":1,"project":"demo","task_defaults":{"cwd":"work","env":{"SHARED":"default"},"success_exit_codes":[0,130],"restart":"always","restart_delay_ms":700,"max_restarts":4,"restart_reset_after_ms":8000,"shutdown_timeout_ms":3000},"tasks":{"api":{"command":"api"}}}"#,
        ),
    ] {
        let root = temporary_directory(extension);
        let path = root.join(format!("procora.{extension}"));
        fs::write(&path, content).unwrap();
        let before = load_path(&path).unwrap();
        let mut editor = ConfigEditor::open(&path).unwrap();

        editor.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));

        let after = load_path(&path).unwrap();
        assert_eq!(before.spec, after.spec, "{extension}");
        assert_eq!(before.task_defaults, after.task_defaults, "{extension}");
        assert_eq!(before.task_origins, after.task_origins, "{extension}");
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
// Task弹窗可以清除本地覆盖并恢复项目默认来源，落盘不复制有效值。
fn task_dialog_can_restore_inherited_defaults() {
    let root = temporary_directory("restore-inherit");
    let path = root.join("procora.yaml");
    fs::write(
        &path,
        "version: 1\nproject: demo\ntask_defaults:\n  cwd: work\n  success_exit_codes: [42]\n  restart: always\n  restart_delay_ms: 700\n  max_restarts: 4\n  restart_reset_after_ms: 8000\n  shutdown_timeout_ms: 3000\ntasks:\n  api:\n    command: api\n    cwd: local\n    success_exit_codes: [130]\n    restart: never\n    restart_delay_ms: 900\n    max_restarts: 9\n    restart_reset_after_ms: 1000\n    shutdown_timeout_ms: 2000\n",
    )
    .unwrap();
    let mut editor = ConfigEditor::open(&path).unwrap();

    press(&mut editor, KeyCode::Right);
    press(&mut editor, KeyCode::Enter);
    for _ in 0..3 {
        press(&mut editor, KeyCode::Tab);
    }
    clear_field(&mut editor);
    for _ in 0..4 {
        press(&mut editor, KeyCode::Tab);
    }
    clear_field(&mut editor);
    press(&mut editor, KeyCode::Tab);
    press(&mut editor, KeyCode::Left);
    for _ in 0..4 {
        press(&mut editor, KeyCode::Tab);
        clear_field(&mut editor);
    }
    press(&mut editor, KeyCode::Enter);
    editor.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));

    let saved = fs::read_to_string(&path).unwrap();
    let compiled = load_path(&path).unwrap();
    let id = "api".parse().unwrap();
    let task = &compiled.spec.tasks[&id];
    assert_eq!(saved.matches("cwd:").count(), 1);
    assert_eq!(saved.matches("success_exit_codes:").count(), 1);
    assert_eq!(saved.matches("restart:").count(), 1);
    assert_eq!(saved.matches("restart_delay:").count(), 1);
    assert_eq!(saved.matches("max_restarts:").count(), 1);
    assert_eq!(saved.matches("restart_reset_after:").count(), 1);
    assert_eq!(saved.matches("shutdown_timeout:").count(), 1);
    assert_eq!(task.restart, RestartPolicy::Always);
    assert_eq!(task.max_restarts, 4);
    assert_eq!(task.cwd.as_ref().unwrap().file_name().unwrap(), "work");
    assert_eq!(
        compiled.task_origins[&id].field("restart"),
        ValueOrigin::TaskDefaults
    );
    fs::remove_dir_all(root).unwrap();
}
