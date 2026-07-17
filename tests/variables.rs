//! 项目变量的解析、字段覆盖、诊断、include 与 TUI 无展开写回契约。

use std::{
    fs,
    path::PathBuf,
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use procora::{
    config::{ConfigError, ConfigFormat, load_path, load_str},
    core::HealthCheckProbe,
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
        "procora-vars-{label}-{}-{nonce}-{sequence}",
        std::process::id()
    ));
    fs::create_dir_all(&directory).unwrap();
    directory
}

/// 向结构化编辑器发送一个无修饰键。
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
// 变量链、转义、argv边界、默认环境和健康检查在三种格式中产生相同语义。
fn variables_are_precise_across_formats() {
    let cases = [
        (
            ConfigFormat::Yaml,
            "version: 1\nproject: demo\nvars:\n  BIN: runner\n  ROOT: workspace\n  PREFIX: hello\n  MESSAGE: '${vars.PREFIX} world'\n  LITERAL: '$${vars.BIN}'\nenv: {GLOBAL: '${vars.MESSAGE}'}\ntask_defaults:\n  cwd: '${vars.ROOT}/app'\n  env: {ESCAPED: '${vars.LITERAL}'}\ntasks:\n  api:\n    command: ['${vars.BIN}', '--message=${vars.MESSAGE}']\n    env: {LOCAL: '${vars.PREFIX}'}\n    healthcheck:\n      command: '${vars.BIN}'\n      args: [check, '${vars.MESSAGE}']\n",
        ),
        (
            ConfigFormat::Toml,
            "version = 1\nproject = 'demo'\n[vars]\nBIN = 'runner'\nROOT = 'workspace'\nPREFIX = 'hello'\nMESSAGE = '${vars.PREFIX} world'\nLITERAL = '$${vars.BIN}'\n[env]\nGLOBAL = '${vars.MESSAGE}'\n[task_defaults]\ncwd = '${vars.ROOT}/app'\n[task_defaults.env]\nESCAPED = '${vars.LITERAL}'\n[tasks.api]\ncommand = ['${vars.BIN}', '--message=${vars.MESSAGE}']\n[tasks.api.env]\nLOCAL = '${vars.PREFIX}'\n[tasks.api.healthcheck]\ncommand = '${vars.BIN}'\nargs = ['check', '${vars.MESSAGE}']\n",
        ),
        (
            ConfigFormat::Json,
            r#"{"version":1,"project":"demo","vars":{"BIN":"runner","ROOT":"workspace","PREFIX":"hello","MESSAGE":"${vars.PREFIX} world","LITERAL":"$${vars.BIN}"},"env":{"GLOBAL":"${vars.MESSAGE}"},"task_defaults":{"cwd":"${vars.ROOT}/app","env":{"ESCAPED":"${vars.LITERAL}"}},"tasks":{"api":{"command":["${vars.BIN}","--message=${vars.MESSAGE}"],"env":{"LOCAL":"${vars.PREFIX}"},"healthcheck":{"command":"${vars.BIN}","args":["check","${vars.MESSAGE}"]}}}}"#,
        ),
    ];
    let compiled = cases
        .into_iter()
        .map(|(format, input)| load_str(input, format).unwrap())
        .collect::<Vec<_>>();

    assert_eq!(compiled[0].spec, compiled[1].spec);
    assert_eq!(compiled[1].spec, compiled[2].spec);
    let api = "api".parse().unwrap();
    let task = &compiled[0].spec.tasks[&api];
    assert_eq!(task.command, "runner");
    assert_eq!(task.args, ["--message=hello world"]);
    assert_eq!(
        task.cwd.as_deref(),
        Some(std::path::Path::new("workspace/app"))
    );
    assert_eq!(task.env["GLOBAL"], "hello world");
    assert_eq!(task.env["ESCAPED"], "${vars.BIN}");
    assert_eq!(task.env["LOCAL"], "hello");
    let HealthCheckProbe::Exec { command, args, .. } = &task.healthcheck.as_ref().unwrap().probe
    else {
        panic!("应规范化为 exec 健康检查");
    };
    assert_eq!(command, "runner");
    assert_eq!(args, &["check", "hello world"]);
    assert_eq!(compiled[0].resolved_vars["MESSAGE"], "hello world");
    assert_eq!(compiled[0].resolved_vars["LITERAL"], "${vars.BIN}");
    assert_eq!(
        compiled[0].variable_references["tasks.api.command.0"],
        ["BIN".to_owned()].into_iter().collect()
    );
    assert_eq!(
        compiled[0].variable_references["task_defaults.cwd"],
        ["ROOT".to_owned()].into_iter().collect()
    );
}

#[test]
// 非法名称、未知引用、循环和未闭合引用都返回声明字段的精确路径。
fn invalid_variables_are_actionable() {
    for (input, expected_path, expected_message) in [
        (
            "version: 1\nproject: demo\nvars: {'bad name': value}\ntasks: {api: {command: api}}\n",
            "vars.bad name",
            "变量名称",
        ),
        (
            "version: 1\nproject: demo\ntasks: {api: {command: '${vars.MISSING}'}}\n",
            "tasks.api.command",
            "MISSING",
        ),
        (
            "version: 1\nproject: demo\nvars: {a: '${vars.b}', b: '${vars.a}'}\ntasks: {api: {command: api}}\n",
            "vars.a",
            "循环",
        ),
        (
            "version: 1\nproject: demo\nvars: {BIN: runner}\ntasks: {api: {command: '${vars.BIN'}}\n",
            "tasks.api.command",
            "右花括号",
        ),
    ] {
        let error = load_str(input, ConfigFormat::Yaml).unwrap_err();
        let ConfigError::Validation { diagnostics, .. } = error else {
            panic!("变量错误应返回结构化诊断");
        };
        assert!(
            diagnostics.iter().any(|item| {
                item.path == expected_path && item.message.contains(expected_message)
            }),
            "{diagnostics:?}"
        );
    }
}

#[test]
// 普通环境和依赖占位符保持字面量，只有vars命名空间会被解析。
fn unrelated_placeholders_remain_literal() {
    let compiled = load_str(
        "version: 1\nproject: demo\nvars: {BIN: runner}\ntasks:\n  api:\n    command: ['${vars.BIN}', '$HOME', '${dependency.tool}', '$${vars.BIN}']\n",
        ConfigFormat::Yaml,
    )
    .unwrap();
    let task = compiled.spec.tasks.values().next().unwrap();
    assert_eq!(task.args, ["$HOME", "${dependency.tool}", "${vars.BIN}"]);
}

#[test]
// 未使用profile和模板中的变量引用也会校验，避免切换或继承时才失败。
fn unused_declarations_validate_variable_references() {
    for (input, expected_path) in [
        (
            "version: 1\nproject: demo\nprofiles: {later: {env: {MODE: '${vars.MISSING}'}}}\ntasks: {api: {command: api}}\n",
            "profiles.later.env.MODE",
        ),
        (
            "version: 1\nproject: demo\ntask_templates: {later: {command: '${vars.MISSING}'}}\ntasks: {api: {command: api}}\n",
            "task_templates.later.command",
        ),
    ] {
        let error = load_str(input, ConfigFormat::Yaml).unwrap_err();
        let ConfigError::Validation { diagnostics, .. } = error else {
            panic!("未使用声明的变量错误应返回结构化诊断");
        };
        assert!(diagnostics.iter().any(|item| item.path == expected_path));
    }
}

#[test]
// 变量先于profile和模板解析，并覆盖HTTP目标与请求头等字符串字段。
fn variables_compose_with_profiles_templates_and_http_checks() {
    let compiled = load_str(
        "version: 1\nproject: demo\nprofile: dev\nvars: {BIN: runner, MODE: dev, HOST: localhost, TOKEN: secret}\nprofiles:\n  dev:\n    env: {MODE: '${vars.MODE}'}\n    task_defaults:\n      env: {DEFAULT_MODE: '${vars.MODE}'}\ntask_templates:\n  service:\n    command: ['${vars.BIN}', serve]\n    healthcheck:\n      http_get:\n        host: '${vars.HOST}'\n        path: '/${vars.MODE}'\n        headers: {Authorization: 'Bearer ${vars.TOKEN}'}\ntasks:\n  api: {extends: service}\n",
        ConfigFormat::Yaml,
    )
    .unwrap();
    let task = compiled.spec.tasks.values().next().unwrap();
    assert_eq!(task.command, "runner");
    assert_eq!(task.args, ["serve"]);
    assert_eq!(task.env["MODE"], "dev");
    assert_eq!(task.env["DEFAULT_MODE"], "dev");
    let HealthCheckProbe::HttpGet { http_get } = &task.healthcheck.as_ref().unwrap().probe else {
        panic!("应规范化为 HTTP 健康检查");
    };
    assert_eq!(http_get.host, "localhost");
    assert_eq!(http_get.path, "/dev");
    assert_eq!(http_get.headers["Authorization"], "Bearer secret");
    assert!(
        compiled
            .variable_references
            .contains_key("task_templates.service.healthcheck.http_get.path")
    );
}

#[test]
// include变量按文档优先级合并，路径表达式仍以声明文件目录为基准。
fn included_variables_merge_before_path_expansion() {
    let root = temporary_directory("include");
    fs::create_dir_all(root.join("fragment/entry-work")).unwrap();
    fs::write(
        root.join("fragment/base.yaml"),
        "vars: {WORK: base-work}\ntask_defaults: {cwd: '${vars.WORK}'}\n",
    )
    .unwrap();
    let entry = root.join("procora.yaml");
    fs::write(
        &entry,
        "include: [fragment/base.yaml]\nversion: 1\nproject: demo\nvars: {WORK: entry-work}\ntasks: {api: {command: api}}\n",
    )
    .unwrap();

    let compiled = load_path(&entry).unwrap();
    let api = "api".parse().unwrap();
    let expected = fs::canonicalize(root.join("fragment/entry-work")).unwrap();
    assert_eq!(
        compiled.spec.tasks[&api].cwd.as_deref(),
        Some(expected.as_path())
    );
    assert_eq!(compiled.resolved_vars["WORK"], "entry-work");
    fs::remove_dir_all(root).unwrap();
}

#[test]
// 变量可组成环境文件路径，文件参与编译且TUI保存后仍保留路径表达式。
fn variable_env_file_path_is_loaded_and_preserved() {
    let root = temporary_directory("env-file");
    fs::create_dir_all(root.join("config")).unwrap();
    fs::write(root.join("config/task.env"), "FROM_FILE=yes\n").unwrap();
    let path = root.join("procora.yaml");
    fs::write(
        &path,
        "version: 1\nproject: demo\nvars: {ENV_DIR: config, ENV_NAME: task.env}\ntasks:\n  api:\n    command: api\n    env_file: '${vars.ENV_DIR}/${vars.ENV_NAME}'\n",
    )
    .unwrap();

    let mut editor = ConfigEditor::open(&path).unwrap();
    editor.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));

    let saved = fs::read_to_string(&path).unwrap();
    let compiled = load_path(&path).unwrap();
    let task = compiled.spec.tasks.values().next().unwrap();
    assert_eq!(task.env["FROM_FILE"], "yes");
    assert!(saved.contains("${vars.ENV_DIR}/${vars.ENV_NAME}"));
    assert_eq!(
        compiled.variable_references["tasks.api.env_file"],
        ["ENV_DIR".to_owned(), "ENV_NAME".to_owned()]
            .into_iter()
            .collect()
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
// TUI编辑变量后立即重编译预览，三种格式保存时仍保留Task原始引用表达式。
fn tui_edits_variables_without_expanding_task_declarations() {
    for extension in ["yaml", "toml", "json"] {
        let root = temporary_directory(extension);
        let path = root.join(format!("procora.{extension}"));
        let input = match extension {
            "yaml" => {
                "version: 1\nproject: demo\nvars: {BIN: runner}\ntasks:\n  api:\n    command: ['${vars.BIN}', serve]\n    healthcheck: {command: '${vars.BIN}'}\n"
            }
            "toml" => {
                "version = 1\nproject = 'demo'\n[vars]\nBIN = 'runner'\n[tasks.api]\ncommand = ['${vars.BIN}', 'serve']\n[tasks.api.healthcheck]\ncommand = '${vars.BIN}'\n"
            }
            "json" => {
                r#"{"version":1,"project":"demo","vars":{"BIN":"runner"},"tasks":{"api":{"command":["${vars.BIN}","serve"],"healthcheck":{"command":"${vars.BIN}"}}}}"#
            }
            _ => unreachable!(),
        };
        fs::write(&path, input).unwrap();
        let mut editor = ConfigEditor::open(&path).unwrap();

        press(&mut editor, KeyCode::Enter);
        for _ in 0..11 {
            press(&mut editor, KeyCode::Tab);
        }
        for _ in 0..64 {
            press(&mut editor, KeyCode::Backspace);
        }
        type_text(&mut editor, r#"{"BIN":"updated"}"#);
        press(&mut editor, KeyCode::Enter);
        editor.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));

        let saved = fs::read_to_string(&path).unwrap();
        let compiled = load_path(&path).unwrap();
        let task = compiled.spec.tasks.values().next().unwrap();
        assert_eq!(task.command, "updated", "{extension}");
        assert_eq!(task.args, ["serve"], "{extension}");
        assert!(saved.contains("${vars.BIN}"), "{extension}: {saved}");
        assert_eq!(compiled.resolved_vars["BIN"], "updated");
        let HealthCheckProbe::Exec { command, .. } = &task.healthcheck.as_ref().unwrap().probe
        else {
            panic!("应保留 exec 健康检查");
        };
        assert_eq!(command, "updated");
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
// CLI有效配置同时输出声明值、解析值和逐字段直接引用。
fn cli_effective_config_explains_variable_resolution() {
    let root = temporary_directory("cli");
    let path = root.join("procora.yaml");
    fs::write(
        &path,
        "version: 1\nproject: demo\nvars: {BASE: runner, BIN: '${vars.BASE}'}\ntasks: {api: {command: ['${vars.BIN}', serve]}}\n",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_procora"))
        .arg("config")
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["vars"]["BIN"], "${vars.BASE}");
    assert_eq!(value["resolved_vars"]["BIN"], "runner");
    assert_eq!(
        value["variable_references"]["tasks.api.command.0"],
        serde_json::json!(["BIN"])
    );
    assert_eq!(value["tasks"]["api"]["command"], "runner");
    fs::remove_dir_all(root).unwrap();
}
