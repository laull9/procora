//! Task 依赖简写、上游条件别名、模板来源与 TUI 紧凑写回契约。

use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use procora::{
    config::{ConfigFormat, ValueOrigin, load_path, load_str},
    core::DependencyCondition,
    tui::ConfigEditor,
};

/// 并行测试创建临时目录时使用的进程内序号。
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// 创建当前测试独占的临时目录。
fn temporary_directory(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let directory = std::env::temp_dir().join(format!(
        "procora-dependency-syntax-{label}-{}-{nonce}-{sequence}",
        std::process::id()
    ));
    fs::create_dir_all(&directory).unwrap();
    directory
}

/// 向结构化配置编辑器发送无修饰按键。
fn press(editor: &mut ConfigEditor, code: KeyCode) {
    editor.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
}

/// 向当前弹窗字段逐字输入。
fn type_text(editor: &mut ConfigEditor, value: &str) {
    for character in value.chars() {
        press(editor, KeyCode::Char(character));
    }
}

/// 返回包含三个条件依赖的通用 Task 前缀。
fn yaml_tasks_prefix() -> &'static str {
    "version: 1\nproject: demo\ntasks:\n  cache: {command: cache}\n  database: {command: database}\n  migrate: {command: migrate}\n  app:\n    command: app\n"
}

#[test]
// 条件标量和process-compose别名在三种格式中与旧版对象产生相同领域语义。
fn scalar_dependency_conditions_are_equivalent_across_formats() {
    let cases = [
        (
            ConfigFormat::Yaml,
            format!(
                "{}    depends_on:\n      database: process_started\n      cache: process_healthy\n      migrate: process_completed_successfully\n",
                yaml_tasks_prefix()
            ),
        ),
        (
            ConfigFormat::Toml,
            "version = 1\nproject = 'demo'\n[tasks.cache]\ncommand = 'cache'\n[tasks.database]\ncommand = 'database'\n[tasks.migrate]\ncommand = 'migrate'\n[tasks.app]\ncommand = 'app'\n[tasks.app.depends_on]\ndatabase = 'process_started'\ncache = 'healthy'\nmigrate = 'process_completed_successfully'\n"
                .to_owned(),
        ),
        (
            ConfigFormat::Json,
            r#"{"version":1,"project":"demo","tasks":{"cache":{"command":"cache"},"database":{"command":"database"},"migrate":{"command":"migrate"},"app":{"command":"app","depends_on":{"database":"started","cache":"process_healthy","migrate":"completed_successfully"}}}}"#
                .to_owned(),
        ),
        (
            ConfigFormat::Yaml,
            format!(
                "{}    depends_on:\n      database: {{condition: started}}\n      cache: {{condition: healthy}}\n      migrate: {{condition: completed_successfully}}\n",
                yaml_tasks_prefix()
            ),
        ),
        (
            ConfigFormat::Yaml,
            format!(
                "{}    depends_on:\n      database: {{condition: process_started}}\n      cache: {{condition: process_healthy}}\n      migrate: {{condition: process_completed_successfully}}\n",
                yaml_tasks_prefix()
            ),
        ),
    ];
    let compiled = cases
        .into_iter()
        .map(|(format, input)| load_str(&input, format).unwrap())
        .collect::<Vec<_>>();

    for candidate in &compiled[1..] {
        assert_eq!(candidate.spec, compiled[0].spec);
        assert_eq!(
            candidate.graph.start_order(),
            compiled[0].graph.start_order()
        );
    }
    let app = &compiled[0].spec.tasks[&"app".parse().unwrap()];
    assert_eq!(
        app.depends_on[&"database".parse().unwrap()].condition,
        DependencyCondition::Started
    );
    assert_eq!(
        app.depends_on[&"cache".parse().unwrap()].condition,
        DependencyCondition::Healthy
    );
    assert_eq!(
        app.depends_on[&"migrate".parse().unwrap()].condition,
        DependencyCondition::CompletedSuccessfully
    );
}

#[test]
// TUI依赖字段接受process-compose条件别名，并保存为Procora规范拼写。
fn tui_normalizes_dependency_condition_aliases() {
    let root = temporary_directory("tui-alias");
    let path = root.join("procora.yaml");
    fs::write(
        &path,
        "version: 1\nproject: demo\ntasks:\n  app: {command: app}\n  database: {command: database}\n",
    )
    .unwrap();
    let mut editor = ConfigEditor::open(&path).unwrap();

    press(&mut editor, KeyCode::Tab);
    press(&mut editor, KeyCode::Enter);
    for _ in 0..6 {
        press(&mut editor, KeyCode::Tab);
    }
    type_text(&mut editor, "database:process_healthy");
    press(&mut editor, KeyCode::Enter);
    editor.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));

    let saved = fs::read_to_string(&path).unwrap();
    let compiled = load_path(&path).unwrap();
    let app = &compiled.spec.tasks[&"app".parse().unwrap()];
    assert_eq!(
        app.depends_on[&"database".parse().unwrap()].condition,
        DependencyCondition::Healthy
    );
    assert!(saved.contains("\"database\": healthy"));
    assert!(!saved.contains("process_healthy"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
// Task名称数组为每条边使用started默认条件，并严格拒绝重复或非字符串元素。
fn dependency_name_lists_are_precise() {
    let shorthand = load_str(
        "version: 1\nproject: demo\ntasks:\n  cache: {command: cache}\n  database: {command: database}\n  app: {command: app, depends_on: [database, cache]}\n",
        ConfigFormat::Yaml,
    )
    .unwrap();
    let legacy = load_str(
        r#"{"version":1,"project":"demo","tasks":{"cache":{"command":"cache"},"database":{"command":"database"},"app":{"command":"app","depends_on":{"database":{},"cache":{"condition":"started"}}}}}"#,
        ConfigFormat::Json,
    )
    .unwrap();
    assert_eq!(shorthand.spec, legacy.spec);

    for (input, expected_path, expected) in [
        (
            "version: 1\nproject: demo\ntasks: {app: {command: app, depends_on: [db, db]}}\n",
            "tasks.app.depends_on",
            "第 1 项 `db` 重复出现",
        ),
        (
            "version: 1\nproject: demo\ntasks: {app: {command: app, depends_on: [db, 42]}}\n",
            "tasks.app.depends_on",
            "Task 名称字符串",
        ),
        (
            "version: 1\nproject: demo\ntasks: {app: {command: app, depends_on: {db: waiting}}}\n",
            "tasks.app.depends_on.db",
            "未知依赖条件 `waiting`",
        ),
        (
            "version: 1\nproject: demo\ntasks: {app: {command: app, depends_on: {db: {condition: waiting}}}}\n",
            "tasks.app.depends_on.db.condition",
            "unknown variant `waiting`",
        ),
        (
            "version: 1\nproject: demo\ntasks: {app: {command: app, depends_on: {db: {wait: true}}}}\n",
            "tasks.app.depends_on.db.wait",
            "unknown field `wait`",
        ),
    ] {
        let error = load_str(input, ConfigFormat::Yaml).unwrap_err().to_string();
        assert!(error.contains(expected_path), "{error}");
        assert!(error.contains(expected), "{error}");
    }
}

#[test]
// include中的模板依赖简写按键组合，并保留每条最终依赖的模板或Task来源。
fn compact_dependencies_compose_with_include_and_templates() {
    let root = temporary_directory("include-template");
    fs::write(
        root.join("base.yaml"),
        "task_templates:\n  common:\n    command: app\n    depends_on: [database]\ntasks:\n  database: {command: database}\n  cache: {command: cache}\n  migrate: {command: migrate}\n",
    )
    .unwrap();
    let entry = root.join("procora.yaml");
    fs::write(
        &entry,
        "include: [base.yaml]\nversion: 1\nproject: demo\ntask_templates:\n  service:\n    extends: common\n    depends_on: {cache: process_healthy}\ntasks:\n  app:\n    extends: service\n    depends_on: {migrate: completed_successfully}\n",
    )
    .unwrap();

    let compiled = load_path(&entry).unwrap();
    let app_id = "app".parse().unwrap();
    let app = &compiled.spec.tasks[&app_id];
    assert_eq!(app.depends_on.len(), 3);
    let origins = &compiled.task_origins[&app_id];
    assert_eq!(origins.depends_on["database"], ValueOrigin::TaskTemplate);
    assert_eq!(origins.template_depends_on["database"], "common");
    assert_eq!(origins.template_depends_on["cache"], "service");
    assert_eq!(origins.depends_on["migrate"], ValueOrigin::Task);
    fs::remove_dir_all(root).unwrap();
}

#[test]
// TUI把旧版依赖对象写成确定性紧凑语法，并在三种格式中保持同一任务图。
fn tui_writes_compact_dependencies_across_formats() {
    for extension in ["yaml", "toml", "json"] {
        let root = temporary_directory(extension);
        let path = root.join(format!("procora.{extension}"));
        let input = match extension {
            "yaml" => {
                "version: 1\nproject: demo\ntasks:\n  cache: {command: cache}\n  database: {command: database}\n  app:\n    command: app\n    depends_on:\n      database: {condition: started}\n      cache: {condition: healthy}\n  worker:\n    command: worker\n    depends_on:\n      database: {condition: started}\n"
            }
            "toml" => {
                "version = 1\nproject = 'demo'\n[tasks.cache]\ncommand = 'cache'\n[tasks.database]\ncommand = 'database'\n[tasks.app]\ncommand = 'app'\n[tasks.app.depends_on.database]\ncondition = 'started'\n[tasks.app.depends_on.cache]\ncondition = 'healthy'\n[tasks.worker]\ncommand = 'worker'\n[tasks.worker.depends_on.database]\ncondition = 'started'\n"
            }
            "json" => {
                r#"{"version":1,"project":"demo","tasks":{"cache":{"command":"cache"},"database":{"command":"database"},"app":{"command":"app","depends_on":{"database":{"condition":"started"},"cache":{"condition":"healthy"}}},"worker":{"command":"worker","depends_on":{"database":{"condition":"started"}}}}}"#
            }
            _ => unreachable!(),
        };
        fs::write(&path, input).unwrap();
        let before = load_path(&path).unwrap();
        let mut editor = ConfigEditor::open(&path).unwrap();
        editor.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
        let saved = fs::read_to_string(&path).unwrap();
        let after = load_path(&path).unwrap();

        assert_eq!(after.spec, before.spec, "{extension}: {saved}");
        assert_eq!(after.graph.start_order(), before.graph.start_order());
        assert!(!saved.contains("condition"), "{extension}: {saved}");
        match extension {
            "yaml" => {
                assert!(saved.contains("\"database\": started"), "{saved}");
                assert!(saved.contains("depends_on: [\"database\"]"), "{saved}");
            }
            "toml" => {
                assert!(saved.contains("database = \"started\""), "{saved}");
                assert!(saved.contains("depends_on = [\"database\"]"), "{saved}");
            }
            "json" => {
                let value: serde_json::Value = serde_json::from_str(&saved).unwrap();
                assert_eq!(value["tasks"]["app"]["depends_on"]["database"], "started");
                assert_eq!(
                    value["tasks"]["worker"]["depends_on"],
                    serde_json::json!(["database"])
                );
            }
            _ => unreachable!(),
        }
        fs::remove_dir_all(root).unwrap();
    }
}
