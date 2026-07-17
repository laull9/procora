//! 命名 Task 模板的继承、覆盖、来源、TUI 和跨前端契约。

use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use procora::{
    config::{ConfigError, ConfigFormat, ValueOrigin, load_path, load_str},
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
        "procora-task-template-{label}-{}-{nonce}-{sequence}",
        std::process::id()
    ));
    fs::create_dir_all(&directory).unwrap();
    directory
}

/// 向结构化编辑器发送一个无修饰键。
fn press(editor: &mut ConfigEditor, code: KeyCode) {
    editor.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
}

/// 向当前字段逐字输入文本。
fn type_text(editor: &mut ConfigEditor, value: &str) {
    for character in value.chars() {
        press(editor, KeyCode::Char(character));
    }
}

#[test]
// 模板链按map合并、标量和列表替换，并在三种格式中得到同一领域语义。
fn template_chains_are_precise_across_formats() {
    let cases = [
        (
            ConfigFormat::Yaml,
            "version: 1\nproject: demo\ntask_templates:\n  base:\n    command: [runner, base]\n    env: {BASE: yes, SHARED: base}\n    success_exit_codes: [0, 130]\n    restart: always\n  worker:\n    extends: base\n    command: 'runner worker'\n    env: {SHARED: worker}\n    restart: on-failure\ntasks:\n  api:\n    extends: worker\n    env: {LOCAL: api}\n    success_exit_codes: [7]\n",
        ),
        (
            ConfigFormat::Toml,
            "version = 1\nproject = 'demo'\n[task_templates.base]\ncommand = ['runner', 'base']\nsuccess_exit_codes = [0, 130]\nrestart = 'always'\n[task_templates.base.env]\nBASE = 'yes'\nSHARED = 'base'\n[task_templates.worker]\nextends = 'base'\ncommand = 'runner worker'\nrestart = 'on-failure'\n[task_templates.worker.env]\nSHARED = 'worker'\n[tasks.api]\nextends = 'worker'\nsuccess_exit_codes = [7]\n[tasks.api.env]\nLOCAL = 'api'\n",
        ),
        (
            ConfigFormat::Json,
            r#"{"version":1,"project":"demo","task_templates":{"base":{"command":["runner","base"],"env":{"BASE":"yes","SHARED":"base"},"success_exit_codes":[0,130],"restart":"always"},"worker":{"extends":"base","command":"runner worker","env":{"SHARED":"worker"},"restart":"on-failure"}},"tasks":{"api":{"extends":"worker","env":{"LOCAL":"api"},"success_exit_codes":[7]}}}"#,
        ),
    ];
    let compiled = cases
        .into_iter()
        .map(|(format, input)| load_str(input, format).unwrap())
        .collect::<Vec<_>>();

    assert_eq!(compiled[0].spec, compiled[1].spec);
    assert_eq!(compiled[1].spec, compiled[2].spec);
    let id = "api".parse().unwrap();
    let task = &compiled[0].spec.tasks[&id];
    assert_eq!(task.command, "runner");
    assert_eq!(task.args, ["worker"]);
    assert_eq!(task.env["BASE"], "yes");
    assert_eq!(task.env["SHARED"], "worker");
    assert_eq!(task.env["LOCAL"], "api");
    assert_eq!(task.success_exit_codes, [0, 7].into_iter().collect());
    let origins = &compiled[0].task_origins[&id];
    assert_eq!(origins.field("command"), ValueOrigin::TaskTemplate);
    assert_eq!(origins.template("command"), Some("worker"));
    assert_eq!(origins.field("success_exit_codes"), ValueOrigin::Task);
    assert_eq!(origins.env["BASE"], ValueOrigin::TaskTemplate);
    assert_eq!(origins.template_env["BASE"], "base");
    assert_eq!(origins.template_env["SHARED"], "worker");
    assert_eq!(compiled[0].task_template_names.len(), 2);
}

#[test]
// 未知模板、自继承和循环链返回稳定字段路径。
fn invalid_template_references_are_actionable() {
    for (input, expected_path) in [
        (
            "version: 1\nproject: demo\ntasks:\n  api: {extends: missing}\n",
            "tasks.api.extends",
        ),
        (
            "version: 1\nproject: demo\ntask_templates:\n  base: {extends: base}\ntasks:\n  api: {extends: base}\n",
            "task_templates.base.extends",
        ),
        (
            "version: 1\nproject: demo\ntask_templates:\n  a: {extends: b}\n  b: {extends: a}\ntasks:\n  api: {extends: a}\n",
            "task_templates.a.extends",
        ),
    ] {
        let error = load_str(input, ConfigFormat::Yaml).unwrap_err();
        let ConfigError::Validation { diagnostics, .. } = error else {
            panic!("模板引用错误应返回结构化诊断");
        };
        assert!(
            diagnostics.iter().any(|item| item.path == expected_path),
            "{diagnostics:?}"
        );
    }
}

#[test]
// 未使用模板也会独立校验生命周期字段，避免隐藏无效声明。
fn unused_templates_are_validated() {
    let error = load_str(
        "version: 1\nproject: demo\ntask_templates:\n  broken:\n    restart_delay_ms: 0\ntasks:\n  api: {command: api}\n",
        ConfigFormat::Yaml,
    )
    .unwrap_err();
    let ConfigError::Validation { diagnostics, .. } = error else {
        panic!("非法未使用模板应返回结构化诊断");
    };
    assert!(
        diagnostics
            .iter()
            .any(|item| item.path == "task_templates.broken.restart_delay_ms")
    );
}

#[test]
// include模板继承的环境文件以模板声明目录解析并保留逐键来源。
fn included_template_env_file_uses_declaring_directory() {
    let root = temporary_directory("include-env");
    fs::create_dir_all(root.join("fragment")).unwrap();
    fs::write(root.join("fragment/task.env"), "FROM_FILE=yes\n").unwrap();
    fs::write(
        root.join("fragment/base.yaml"),
        "task_templates:\n  base:\n    command: api\n    env_file: task.env\n",
    )
    .unwrap();
    let path = root.join("procora.yaml");
    fs::write(
        &path,
        "include: [fragment/base.yaml]\nversion: 1\nproject: demo\ntasks:\n  api: {extends: base}\n",
    )
    .unwrap();

    let compiled = load_path(&path).unwrap();
    let id = "api".parse().unwrap();
    assert_eq!(compiled.spec.tasks[&id].env["FROM_FILE"], "yes");
    assert_eq!(
        compiled.task_origins[&id].env["FROM_FILE"],
        ValueOrigin::EnvFile
    );
    assert_eq!(
        compiled.task_origins[&id].field("env_file"),
        ValueOrigin::TaskTemplate
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
// TUI可为Task选择模板，并在保存时删除本地命令而不展开继承值。
fn task_dialog_selects_template_without_expanding_it() {
    let root = temporary_directory("tui-select");
    let path = root.join("procora.yaml");
    fs::write(
        &path,
        "version: 1\nproject: demo\ntask_templates:\n  base:\n    command: [runner, inherited]\n    env: {SHARED: template}\n    restart: always\ntasks:\n  api:\n    command: api\n",
    )
    .unwrap();
    let mut editor = ConfigEditor::open(&path).unwrap();

    press(&mut editor, KeyCode::Right);
    press(&mut editor, KeyCode::Enter);
    press(&mut editor, KeyCode::Tab);
    for _ in 0..3 {
        press(&mut editor, KeyCode::Backspace);
    }
    for _ in 0..12 {
        press(&mut editor, KeyCode::Tab);
    }
    type_text(&mut editor, "base");
    press(&mut editor, KeyCode::Enter);
    editor.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));

    let saved = fs::read_to_string(&path).unwrap();
    let compiled = load_path(&path).unwrap();
    let id = "api".parse().unwrap();
    assert!(saved.contains("extends: \"base\""));
    assert_eq!(saved.matches("\"command\"").count(), 1);
    assert_eq!(saved.matches("\"restart\"").count(), 1);
    assert_eq!(compiled.spec.tasks[&id].command, "runner");
    assert_eq!(compiled.spec.tasks[&id].args, ["inherited"]);
    assert_eq!(
        compiled.task_origins[&id].field("command"),
        ValueOrigin::TaskTemplate
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
// 三种声明式格式经过TUI保存后都保留模板链和Task局部覆盖。
fn form_roundtrip_preserves_templates_across_formats() {
    for (extension, content) in [
        (
            "yaml",
            "version: 1\nproject: demo\ntask_templates:\n  base: {command: [runner, base], restart: always}\n  worker: {extends: base, command: [runner, worker]}\ntasks:\n  api: {extends: worker, env: {LOCAL: yes}}\n",
        ),
        (
            "toml",
            "version = 1\nproject = 'demo'\n[task_templates.base]\ncommand = ['runner', 'base']\nrestart = 'always'\n[task_templates.worker]\nextends = 'base'\ncommand = ['runner', 'worker']\n[tasks.api]\nextends = 'worker'\n[tasks.api.env]\nLOCAL = 'yes'\n",
        ),
        (
            "json",
            r#"{"version":1,"project":"demo","task_templates":{"base":{"command":["runner","base"],"restart":"always"},"worker":{"extends":"base","command":["runner","worker"]}},"tasks":{"api":{"extends":"worker","env":{"LOCAL":"yes"}}}}"#,
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
        assert_eq!(before.task_origins, after.task_origins, "{extension}");
        assert_eq!(before.task_template_names, after.task_template_names);
        fs::remove_dir_all(root).unwrap();
    }
}
