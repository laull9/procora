//! 更少配置与精确 argv 写法的跨格式、覆盖顺序和诊断测试。

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
        "procora-config-usability-{label}-{}-{nonce}-{sequence}",
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

#[test]
// 项目默认环境与command argv简写在三种声明式格式中产生完全相同的领域语义。
fn project_env_and_argv_are_equivalent_across_formats() {
    let cases = [
        (
            ConfigFormat::Yaml,
            "version: 1\nproject: demo\nenv:\n  SHARED: global\n  OVERRIDE: global\ntasks:\n  api:\n    command: [runner, 'hello world', '']\n    env:\n      OVERRIDE: task\n",
        ),
        (
            ConfigFormat::Toml,
            "version = 1\nproject = 'demo'\n[env]\nSHARED = 'global'\nOVERRIDE = 'global'\n[tasks.api]\ncommand = ['runner', 'hello world', '']\n[tasks.api.env]\nOVERRIDE = 'task'\n",
        ),
        (
            ConfigFormat::Json,
            r#"{"version":1,"project":"demo","env":{"SHARED":"global","OVERRIDE":"global"},"tasks":{"api":{"command":["runner","hello world",""],"env":{"OVERRIDE":"task"}}}}"#,
        ),
    ];
    let compiled = cases
        .into_iter()
        .map(|(format, input)| load_str(input, format).unwrap())
        .collect::<Vec<_>>();

    assert_eq!(compiled[0].spec, compiled[1].spec);
    assert_eq!(compiled[1].spec, compiled[2].spec);
    assert_eq!(compiled[0].task_origins, compiled[1].task_origins);
    assert_eq!(compiled[1].task_origins, compiled[2].task_origins);
    assert_eq!(compiled[0].project_env["SHARED"], "global");
    let task = compiled[0].spec.tasks.values().next().unwrap();
    assert_eq!(task.command, "runner");
    assert_eq!(task.args, ["hello world", ""]);
    assert_eq!(task.env["SHARED"], "global");
    assert_eq!(task.env["OVERRIDE"], "task");
}

#[test]
// 新argv简写与既有command加args写法规范化为同一个TaskSpec。
fn argv_shorthand_is_equivalent_to_legacy_command_and_args() {
    let legacy = load_str(
        r#"{"version":1,"project":"demo","tasks":{"api":{"command":"runner","args":["hello world",""]}}}"#,
        ConfigFormat::Json,
    )
    .unwrap();
    let shorthand = load_str(
        r#"{"version":1,"project":"demo","tasks":{"api":{"command":["runner","hello world",""]}}}"#,
        ConfigFormat::Json,
    )
    .unwrap();

    assert_eq!(legacy.spec, shorthand.spec);
    assert_eq!(legacy.graph.start_order(), shorthand.graph.start_order());
}

#[test]
// 命令文本在三种声明式格式中用引号和反斜杠精确表达内嵌参数。
fn command_text_with_embedded_args_is_equivalent_across_formats() {
    let cases = [
        (
            ConfigFormat::Yaml,
            "version: 1\nproject: demo\ntasks:\n  api:\n    command: 'runner \"hello world\" \"\" plain\\ value C:\\tools\\app'\n",
        ),
        (
            ConfigFormat::Toml,
            "version = 1\nproject = 'demo'\n[tasks.api]\ncommand = 'runner \"hello world\" \"\" plain\\ value C:\\tools\\app'\n",
        ),
        (
            ConfigFormat::Json,
            r#"{"version":1,"project":"demo","tasks":{"api":{"command":"runner \"hello world\" \"\" plain\\ value C:\\tools\\app"}}}"#,
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
    assert_eq!(
        task.args,
        ["hello world", "", "plain value", r"C:\tools\app"]
    );
    assert_eq!(
        compiled[0].task_origins[&id].field("args"),
        ValueOrigin::Task
    );
}

#[test]
// 显式args保留旧版单程序字符串语义，非法命令文本则报告精确字段路径。
fn explicit_args_preserve_program_string_and_invalid_text_is_actionable() {
    let compatible = load_str(
        r#"{"version":1,"project":"demo","tasks":{"api":{"command":"C:\\Program Files\\app.exe","args":[]}}}"#,
        ConfigFormat::Json,
    )
    .unwrap();
    let task = compatible.spec.tasks.values().next().unwrap();
    assert_eq!(task.command, r"C:\Program Files\app.exe");
    assert!(task.args.is_empty());

    let error = load_str(
        "version: 1\nproject: demo\ntasks:\n  api:\n    command: 'runner \"unfinished'\n",
        ConfigFormat::Yaml,
    )
    .unwrap_err();
    let ConfigError::Validation { diagnostics, .. } = error else {
        panic!("未闭合命令文本应返回结构化诊断");
    };
    assert!(diagnostics.iter().any(|item| {
        item.path == "tasks.api.command" && item.message.contains("未闭合的双引号")
    }));
}

#[test]
// include从左到右合并项目环境，入口值优先并应用到所有片段Task。
fn included_project_env_merges_before_task_overrides() {
    let root = temporary_directory("include");
    fs::write(
        root.join("base.yaml"),
        "env:\n  BASE: base\n  SHARED: base\ntasks:\n  worker:\n    command: worker\n    env:\n      LOCAL: worker\n",
    )
    .unwrap();
    fs::write(
        root.join("procora.yaml"),
        "include: [base.yaml]\nversion: 1\nproject: demo\nenv:\n  SHARED: entry\n  ENTRY: entry\ntasks:\n  api:\n    command: api\n    env:\n      SHARED: api\n",
    )
    .unwrap();

    let compiled = load_path(root.join("procora.yaml")).unwrap();
    let worker = &compiled.spec.tasks[&"worker".parse().unwrap()];
    let api = &compiled.spec.tasks[&"api".parse().unwrap()];
    assert_eq!(worker.env["BASE"], "base");
    assert_eq!(worker.env["SHARED"], "entry");
    assert_eq!(worker.env["ENTRY"], "entry");
    assert_eq!(worker.env["LOCAL"], "worker");
    assert_eq!(api.env["SHARED"], "api");
    fs::remove_dir_all(root).unwrap();
}

#[test]
// 环境优先级固定为项目默认值低于env_file，env_file低于Task内联值。
fn project_env_precedence_is_lower_than_env_file_and_inline_env() {
    let root = temporary_directory("env-file");
    fs::write(
        root.join("task.env"),
        "FROM_FILE=file\nOVERRIDE=file\nSHARED=file\n",
    )
    .unwrap();
    fs::write(
        root.join("procora.yaml"),
        "version: 1\nproject: demo\nenv:\n  GLOBAL: global\n  SHARED: global\n  OVERRIDE: global\ntask_defaults:\n  env:\n    DEFAULT_ONLY: default\n    SHARED: default\ntasks:\n  api:\n    command: api\n    env_file: task.env\n    env:\n      OVERRIDE: inline\n",
    )
    .unwrap();

    let compiled = load_path(root.join("procora.yaml")).unwrap();
    let env = &compiled.spec.tasks.values().next().unwrap().env;
    assert_eq!(env["GLOBAL"], "global");
    assert_eq!(env["DEFAULT_ONLY"], "default");
    assert_eq!(env["FROM_FILE"], "file");
    assert_eq!(env["SHARED"], "file");
    assert_eq!(env["OVERRIDE"], "inline");
    fs::remove_dir_all(root).unwrap();
}

#[test]
// argv简写拒绝空数组、重复args和非字符串元素，并保留精确字段路径。
fn invalid_argv_shorthand_has_actionable_diagnostics() {
    let semantic = load_str(
        "version: 1\nproject: demo\ntasks:\n  empty:\n    command: []\n  duplicate:\n    command: [runner, one]\n    args: [two]\n",
        ConfigFormat::Yaml,
    )
    .unwrap_err();
    let ConfigError::Validation { diagnostics, .. } = semantic else {
        panic!("空 argv 与重复参数应返回结构化诊断");
    };
    assert!(
        diagnostics
            .iter()
            .any(|item| item.path == "tasks.empty.command")
    );
    assert!(
        diagnostics
            .iter()
            .any(|item| item.path == "tasks.duplicate.args")
    );

    for (format, input) in [
        (
            ConfigFormat::Yaml,
            "version: 1\nproject: demo\ntasks:\n  api:\n    command: [runner, 42]\n",
        ),
        (
            ConfigFormat::Toml,
            "version = 1\nproject = 'demo'\n[tasks.api]\ncommand = ['runner', 42]\n",
        ),
        (
            ConfigFormat::Json,
            r#"{"version":1,"project":"demo","tasks":{"api":{"command":["runner",42]}}}"#,
        ),
    ] {
        let error = load_str(input, format).unwrap_err().to_string();
        assert!(error.contains("tasks.api.command"), "{format}: {error}");
    }
}

#[test]
// 项目弹窗可编辑全局默认环境，Task仅保留自己的覆盖值。
fn project_dialog_edits_default_environment_without_task_duplication() {
    let root = temporary_directory("project-dialog");
    let path = root.join("procora.yaml");
    fs::write(
        &path,
        "version: 1\nproject: demo\nenv:\n  BASE: one\ntasks:\n  api:\n    command: api\n    env:\n      LOCAL: task\n",
    )
    .unwrap();
    let mut editor = ConfigEditor::open(&path).unwrap();

    press(&mut editor, KeyCode::Enter);
    press(&mut editor, KeyCode::Tab);
    for _ in 0..14 {
        press(&mut editor, KeyCode::Backspace);
    }
    type_text(&mut editor, r#"{"BASE":"two","CSV":"a,b"}"#);
    press(&mut editor, KeyCode::Enter);
    editor.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));

    let saved = fs::read_to_string(&path).unwrap();
    let compiled = load_path(&path).unwrap();
    let task = compiled.spec.tasks.values().next().unwrap();
    assert_eq!(compiled.project_env["BASE"], "two");
    assert_eq!(task.env["BASE"], "two");
    assert_eq!(task.env["CSV"], "a,b");
    assert_eq!(task.env["LOCAL"], "task");
    assert_eq!(saved.matches("BASE").count(), 1);
    fs::remove_dir_all(root).unwrap();
}

#[test]
// 三种声明式格式都允许env_file配置进入表单并无损保存声明层。
fn env_file_form_roundtrip_is_supported_in_all_formats() {
    for (extension, content) in [
        (
            "yaml",
            "version: 1\nproject: demo\ntasks:\n  api:\n    command: api\n    env_file: task.env\n",
        ),
        (
            "toml",
            "version = 1\nproject = 'demo'\n[tasks.api]\ncommand = 'api'\nenv_file = 'task.env'\n",
        ),
        (
            "json",
            r#"{"version":1,"project":"demo","tasks":{"api":{"command":"api","env_file":"task.env"}}}"#,
        ),
    ] {
        let root = temporary_directory(extension);
        let path = root.join(format!("procora.{extension}"));
        fs::write(root.join("task.env"), "SECRET=from-file\n").unwrap();
        fs::write(&path, content).unwrap();
        let mut editor = ConfigEditor::open(&path).unwrap();

        assert!(editor.message().contains("表单模式"));
        editor.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));

        let saved = fs::read_to_string(&path).unwrap();
        let compiled = load_path(&path).unwrap();
        assert!(saved.contains("env_file"));
        assert!(!saved.contains("from-file"));
        assert_eq!(
            compiled.spec.tasks.values().next().unwrap().env["SECRET"],
            "from-file"
        );
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
// 字段来源区分省略的内建默认、显式默认以及argv声明。
fn field_origins_preserve_explicit_defaults() {
    let omitted = load_str(
        "version: 1\nproject: demo\ntasks:\n  api:\n    command: api\n",
        ConfigFormat::Yaml,
    )
    .unwrap();
    let explicit = load_str(
        "version: 1\nproject: demo\ntasks:\n  api:\n    command: [api]\n    success_exit_codes: [0]\n    restart: never\n",
        ConfigFormat::Yaml,
    )
    .unwrap();
    let id = "api".parse().unwrap();

    assert_eq!(
        omitted.task_origins[&id].field("restart"),
        ValueOrigin::BuiltIn
    );
    assert_eq!(
        explicit.task_origins[&id].field("restart"),
        ValueOrigin::Task
    );
    assert_eq!(
        explicit.task_origins[&id].field("success_exit_codes"),
        ValueOrigin::Task
    );
    assert_eq!(explicit.task_origins[&id].field("args"), ValueOrigin::Task);
    assert_eq!(omitted.spec, explicit.spec);
}

#[test]
// 最终环境变量来源遵循项目env、env_file、Task内联值的逐键覆盖顺序。
fn environment_origins_follow_layer_precedence() {
    let root = temporary_directory("origins");
    fs::write(
        root.join("task.env"),
        "FILE_ONLY=file\nFILE_WINS=file\nINLINE_WINS=file\n",
    )
    .unwrap();
    fs::write(
        root.join("procora.yaml"),
        "version: 1\nproject: demo\nenv:\n  GLOBAL: global\n  FILE_WINS: global\n  INLINE_WINS: global\ntask_defaults:\n  env:\n    DEFAULT_ONLY: default\ntasks:\n  api:\n    command: api\n    env_file: task.env\n    env:\n      INLINE_WINS: task\n      TASK_ONLY: task\n",
    )
    .unwrap();

    let compiled = load_path(root.join("procora.yaml")).unwrap();
    let origins = &compiled.task_origins.values().next().unwrap().env;
    assert_eq!(origins["GLOBAL"], ValueOrigin::ProjectEnv);
    assert_eq!(origins["DEFAULT_ONLY"], ValueOrigin::TaskDefaults);
    assert_eq!(origins["FILE_ONLY"], ValueOrigin::EnvFile);
    assert_eq!(origins["FILE_WINS"], ValueOrigin::EnvFile);
    assert_eq!(origins["INLINE_WINS"], ValueOrigin::Task);
    assert_eq!(origins["TASK_ONLY"], ValueOrigin::Task);
    fs::remove_dir_all(root).unwrap();
}

#[test]
// 表单保存不展开省略的生命周期默认值，但保留用户显式写出的默认值。
fn form_save_preserves_default_declaration_intent() {
    for (label, declarations, expected) in [
        ("omitted", "", false),
        (
            "explicit",
            "    success_exit_codes: [0]\n    restart: never\n",
            true,
        ),
    ] {
        let root = temporary_directory(label);
        let path = root.join("procora.yaml");
        fs::write(
            &path,
            format!("version: 1\nproject: demo\ntasks:\n  api:\n    command: api\n{declarations}"),
        )
        .unwrap();
        let mut editor = ConfigEditor::open(&path).unwrap();
        editor.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));

        let saved = fs::read_to_string(&path).unwrap();
        assert_eq!(saved.contains("    restart: never"), expected);
        assert_eq!(saved.contains("    success_exit_codes:"), expected);
        assert!(!saved.contains("restart_delay_ms"));
        assert!(!saved.contains("shutdown_timeout_ms"));
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
// 项目级Task默认值在三种格式中等价，Task标量和列表整体替换、环境按键覆盖。
fn task_defaults_are_precise_across_formats() {
    let cases = [
        (
            ConfigFormat::Yaml,
            "version: 1\nproject: demo\ntask_defaults:\n  cwd: work\n  env: {SHARED: default, DEFAULT_ONLY: yes}\n  success_exit_codes: [0, 130]\n  restart: on-failure\n  restart_delay_ms: 750\n  max_restarts: 4\n  restart_reset_after_ms: 9000\n  shutdown_timeout_ms: 3000\ntasks:\n  api:\n    command: api\n  worker:\n    command: worker\n    env: {SHARED: worker}\n    success_exit_codes: [7]\n    restart: never\n",
        ),
        (
            ConfigFormat::Toml,
            "version = 1\nproject = 'demo'\n[task_defaults]\ncwd = 'work'\nsuccess_exit_codes = [0, 130]\nrestart = 'on-failure'\nrestart_delay_ms = 750\nmax_restarts = 4\nrestart_reset_after_ms = 9000\nshutdown_timeout_ms = 3000\n[task_defaults.env]\nSHARED = 'default'\nDEFAULT_ONLY = 'yes'\n[tasks.api]\ncommand = 'api'\n[tasks.worker]\ncommand = 'worker'\nsuccess_exit_codes = [7]\nrestart = 'never'\n[tasks.worker.env]\nSHARED = 'worker'\n",
        ),
        (
            ConfigFormat::Json,
            r#"{"version":1,"project":"demo","task_defaults":{"cwd":"work","env":{"SHARED":"default","DEFAULT_ONLY":"yes"},"success_exit_codes":[0,130],"restart":"on-failure","restart_delay_ms":750,"max_restarts":4,"restart_reset_after_ms":9000,"shutdown_timeout_ms":3000},"tasks":{"api":{"command":"api"},"worker":{"command":"worker","env":{"SHARED":"worker"},"success_exit_codes":[7],"restart":"never"}}}"#,
        ),
    ];
    let compiled = cases
        .into_iter()
        .map(|(format, input)| load_str(input, format).unwrap())
        .collect::<Vec<_>>();

    assert_eq!(compiled[0].spec, compiled[1].spec);
    assert_eq!(compiled[1].spec, compiled[2].spec);
    assert_eq!(compiled[0].task_defaults, compiled[1].task_defaults);
    let api_id = "api".parse().unwrap();
    let worker_id = "worker".parse().unwrap();
    let api = &compiled[0].spec.tasks[&api_id];
    let worker = &compiled[0].spec.tasks[&worker_id];
    assert_eq!(api.env["SHARED"], "default");
    assert_eq!(api.restart, procora::core::RestartPolicy::OnFailure);
    assert_eq!(
        api.success_exit_codes.iter().copied().collect::<Vec<_>>(),
        [0, 130]
    );
    assert_eq!(worker.env["SHARED"], "worker");
    assert_eq!(worker.env["DEFAULT_ONLY"], "yes");
    assert_eq!(
        worker
            .success_exit_codes
            .iter()
            .copied()
            .collect::<Vec<_>>(),
        [0, 7]
    );
    assert_eq!(
        compiled[0].task_origins[&api_id].field("restart"),
        ValueOrigin::TaskDefaults
    );
    assert_eq!(
        compiled[0].task_origins[&worker_id].field("restart"),
        ValueOrigin::Task
    );
    assert_eq!(
        compiled[0].task_origins[&api_id].env["SHARED"],
        ValueOrigin::TaskDefaults
    );
}

#[test]
// include按字段合并Task默认层，且无Task时仍独立校验非法默认值。
fn included_task_defaults_merge_and_validate_without_tasks() {
    let root = temporary_directory("task-defaults-include");
    fs::write(
        root.join("base.yaml"),
        "task_defaults:\n  env: {BASE: base, SHARED: base}\n  restart: always\n  max_restarts: 2\n",
    )
    .unwrap();
    fs::write(
        root.join("procora.yaml"),
        "include: [base.yaml]\nversion: 1\nproject: demo\ntask_defaults:\n  env: {SHARED: entry}\n  max_restarts: 5\ntasks:\n  api: {command: api}\n",
    )
    .unwrap();
    let compiled = load_path(root.join("procora.yaml")).unwrap();
    let task = compiled.spec.tasks.values().next().unwrap();
    assert_eq!(task.env["BASE"], "base");
    assert_eq!(task.env["SHARED"], "entry");
    assert_eq!(task.max_restarts, 5);
    assert_eq!(task.restart, procora::core::RestartPolicy::Always);

    let error = load_str(
        "version: 1\nproject: demo\ntask_defaults:\n  restart_delay_ms: 0\ntasks: {}\n",
        ConfigFormat::Yaml,
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("task_defaults.restart_delay_ms"), "{error}");
    fs::remove_dir_all(root).unwrap();
}
