//! 命名 profile 的覆盖、Task 准入、来源、include 与 TUI 切换契约。

use std::{
    fs,
    path::PathBuf,
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use procora::{
    config::{ConfigError, ConfigFormat, ValueOrigin, load_path, load_str},
    core::{GraphError, RestartPolicy},
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
        "procora-profile-{label}-{}-{nonce}-{sequence}",
        std::process::id()
    ));
    fs::create_dir_all(&directory).unwrap();
    directory
}

/// 向结构化编辑器发送一个无修饰键。
fn press(editor: &mut ConfigEditor, code: KeyCode) {
    editor.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
}

#[test]
// profile在三种格式中按键覆盖共享层、持久筛选Task并标记精确来源。
fn selected_profile_is_precise_across_formats() {
    let cases = [
        (
            ConfigFormat::Yaml,
            "version: 1\nproject: demo\nprofile: dev\nprofiles:\n  dev:\n    tasks: [api]\n    env: {MODE: dev}\n    task_defaults:\n      env: {LAYER: profile}\n      restart: always\nenv: {BASE: yes, MODE: base}\ntask_defaults:\n  env: {DEFAULT: base, LAYER: base}\n  restart: never\ntasks:\n  api: {command: api}\n  worker: {command: worker}\n",
        ),
        (
            ConfigFormat::Toml,
            "version = 1\nproject = 'demo'\nprofile = 'dev'\n[env]\nBASE = 'yes'\nMODE = 'base'\n[task_defaults]\nrestart = 'never'\n[task_defaults.env]\nDEFAULT = 'base'\nLAYER = 'base'\n[profiles.dev]\ntasks = ['api']\n[profiles.dev.env]\nMODE = 'dev'\n[profiles.dev.task_defaults]\nrestart = 'always'\n[profiles.dev.task_defaults.env]\nLAYER = 'profile'\n[tasks.api]\ncommand = 'api'\n[tasks.worker]\ncommand = 'worker'\n",
        ),
        (
            ConfigFormat::Json,
            r#"{"version":1,"project":"demo","profile":"dev","profiles":{"dev":{"tasks":["api"],"env":{"MODE":"dev"},"task_defaults":{"env":{"LAYER":"profile"},"restart":"always"}}},"env":{"BASE":"yes","MODE":"base"},"task_defaults":{"env":{"DEFAULT":"base","LAYER":"base"},"restart":"never"},"tasks":{"api":{"command":"api"},"worker":{"command":"worker"}}}"#,
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
    assert_eq!(compiled[0].active_profile.as_deref(), Some("dev"));
    assert_eq!(compiled[0].profile_names.len(), 1);
    assert_eq!(compiled[0].spec.tasks.len(), 1);
    assert_eq!(task.restart, RestartPolicy::Always);
    assert_eq!(task.env["BASE"], "yes");
    assert_eq!(task.env["MODE"], "dev");
    assert_eq!(task.env["DEFAULT"], "base");
    assert_eq!(task.env["LAYER"], "profile");
    let origins = &compiled[0].task_origins[&api];
    assert_eq!(origins.field("restart"), ValueOrigin::Profile);
    assert_eq!(origins.env["BASE"], ValueOrigin::ProjectEnv);
    assert_eq!(origins.env["MODE"], ValueOrigin::Profile);
    assert_eq!(origins.env["DEFAULT"], ValueOrigin::TaskDefaults);
    assert_eq!(origins.env["LAYER"], ValueOrigin::Profile);
    assert_eq!(compiled[0].declared_project_env["MODE"], "base");
    assert_eq!(
        compiled[0].declared_task_defaults.restart,
        Some(RestartPolicy::Never)
    );
}

#[test]
// profile继承在三种格式中组合map，并由子层整体替换Task白名单和默认标量。
fn profile_inheritance_is_precise_across_formats() {
    let cases = [
        (
            ConfigFormat::Yaml,
            "version: 1\nproject: demo\nprofile: ci\nprofiles:\n  dev:\n    tasks: [api, worker]\n    env: {MODE: dev, SHARED: base}\n    task_defaults: {restart: on-failure, env: {BASE: yes}}\n  ci:\n    extends: dev\n    tasks: [worker]\n    env: {MODE: ci}\n    task_defaults: {max_restarts: 3}\ntasks:\n  api: {command: api}\n  worker: {command: worker}\n",
        ),
        (
            ConfigFormat::Toml,
            "version = 1\nproject = 'demo'\nprofile = 'ci'\n[profiles.dev]\ntasks = ['api', 'worker']\n[profiles.dev.env]\nMODE = 'dev'\nSHARED = 'base'\n[profiles.dev.task_defaults]\nrestart = 'on-failure'\n[profiles.dev.task_defaults.env]\nBASE = 'yes'\n[profiles.ci]\nextends = 'dev'\ntasks = ['worker']\n[profiles.ci.env]\nMODE = 'ci'\n[profiles.ci.task_defaults]\nmax_restarts = 3\n[tasks.api]\ncommand = 'api'\n[tasks.worker]\ncommand = 'worker'\n",
        ),
        (
            ConfigFormat::Json,
            r#"{"version":1,"project":"demo","profile":"ci","profiles":{"dev":{"tasks":["api","worker"],"env":{"MODE":"dev","SHARED":"base"},"task_defaults":{"restart":"on-failure","env":{"BASE":"yes"}}},"ci":{"extends":"dev","tasks":["worker"],"env":{"MODE":"ci"},"task_defaults":{"max_restarts":3}}},"tasks":{"api":{"command":"api"},"worker":{"command":"worker"}}}"#,
        ),
    ];
    let compiled = cases
        .into_iter()
        .map(|(format, input)| load_str(input, format).unwrap())
        .collect::<Vec<_>>();

    assert_eq!(compiled[0].spec, compiled[1].spec);
    assert_eq!(compiled[1].spec, compiled[2].spec);
    assert_eq!(compiled[0].profile_extends["ci"], "dev");
    assert_eq!(compiled[0].spec.tasks.len(), 1);
    let worker = &compiled[0].spec.tasks[&"worker".parse().unwrap()];
    assert_eq!(worker.env["MODE"], "ci");
    assert_eq!(worker.env["SHARED"], "base");
    assert_eq!(worker.env["BASE"], "yes");
    assert_eq!(worker.restart, RestartPolicy::OnFailure);
    assert_eq!(worker.max_restarts, 3);
}

#[test]
// 未使用profile中的未知、自继承和循环引用也返回精确extends路径。
fn invalid_profile_inheritance_is_actionable_when_unused() {
    for (input, expected_path) in [
        (
            "version: 1\nproject: demo\nprofiles: {ci: {extends: missing}}\ntasks: {api: {command: api}}\n",
            "profiles.ci.extends",
        ),
        (
            "version: 1\nproject: demo\nprofiles: {ci: {extends: ci}}\ntasks: {api: {command: api}}\n",
            "profiles.ci.extends",
        ),
        (
            "version: 1\nproject: demo\nprofiles: {a: {extends: b}, b: {extends: a}}\ntasks: {api: {command: api}}\n",
            "profiles.a.extends",
        ),
    ] {
        let error = load_str(input, ConfigFormat::Yaml).unwrap_err();
        let ConfigError::Validation { diagnostics, .. } = error else {
            panic!("profile 继承错误应返回结构化诊断");
        };
        assert!(
            diagnostics.iter().any(|item| item.path == expected_path),
            "{diagnostics:?}"
        );
    }
}

#[test]
// 未知profile、未知或重复Task及未使用profile中的非法默认值都有精确路径。
fn invalid_profiles_are_actionable_and_unused_profiles_are_validated() {
    for (input, expected_path) in [
        (
            "version: 1\nproject: demo\nprofile: missing\ntasks: {api: {command: api}}\n",
            "profile",
        ),
        (
            "version: 1\nproject: demo\nprofiles: {dev: {tasks: [missing]}}\ntasks: {api: {command: api}}\n",
            "profiles.dev.tasks.0",
        ),
        (
            "version: 1\nproject: demo\nprofiles: {dev: {tasks: [api, api]}}\ntasks: {api: {command: api}}\n",
            "profiles.dev.tasks.1",
        ),
        (
            "version: 1\nproject: demo\nprofiles: {broken: {task_defaults: {restart_delay_ms: 0}}}\ntasks: {api: {command: api}}\n",
            "profiles.broken.task_defaults.restart_delay_ms",
        ),
    ] {
        let error = load_str(input, ConfigFormat::Yaml).unwrap_err();
        let ConfigError::Validation { diagnostics, .. } = error else {
            panic!("profile 声明错误应返回结构化诊断");
        };
        assert!(
            diagnostics.iter().any(|item| item.path == expected_path),
            "{diagnostics:?}"
        );
    }
}

#[test]
// 被准入Task不能依赖当前profile排除的Task。
fn profile_admission_rejects_dependency_on_inactive_task() {
    let error = load_str(
        "version: 1\nproject: demo\nprofile: api-only\nprofiles: {api-only: {tasks: [api]}}\ntasks:\n  api:\n    command: api\n    depends_on: {worker: {condition: started}}\n  worker: {command: worker}\n",
        ConfigFormat::Yaml,
    )
    .unwrap_err();
    assert!(matches!(
        error,
        ConfigError::Graph(GraphError::MissingDependency { .. })
    ));
}

#[test]
// 未准入Task仍独立校验，避免切换profile后才暴露损坏定义。
fn inactive_tasks_are_still_validated() {
    let error = load_str(
        "version: 1\nproject: demo\nprofile: api-only\nprofiles: {api-only: {tasks: [api]}}\ntasks:\n  api: {command: api}\n  broken: {}\n",
        ConfigFormat::Yaml,
    )
    .unwrap_err();
    let ConfigError::Validation { diagnostics, .. } = error else {
        panic!("未准入 Task 错误应返回结构化诊断");
    };
    assert!(
        diagnostics
            .iter()
            .any(|item| item.path == "tasks.broken.command")
    );
}

#[test]
// include中的profile路径按声明目录解析，同名profile的map按键组合。
fn included_profiles_merge_and_rebase_defaults() {
    let root = temporary_directory("include");
    fs::create_dir_all(root.join("fragment/work")).unwrap();
    fs::write(
        root.join("fragment/base.yaml"),
        "profiles:\n  dev:\n    env: {FROM_FRAGMENT: yes}\n    task_defaults: {cwd: work}\n",
    )
    .unwrap();
    let entry = root.join("procora.yaml");
    fs::write(
        &entry,
        "include: [fragment/base.yaml]\nversion: 1\nproject: demo\nprofile: dev\nprofiles: {dev: {env: {FROM_ENTRY: yes}}}\ntasks: {api: {command: api}}\n",
    )
    .unwrap();

    let compiled = load_path(&entry).unwrap();
    let api = "api".parse().unwrap();
    assert_eq!(compiled.spec.tasks[&api].env["FROM_FRAGMENT"], "yes");
    assert_eq!(compiled.spec.tasks[&api].env["FROM_ENTRY"], "yes");
    let expected_cwd = fs::canonicalize(root.join("fragment/work")).unwrap();
    assert_eq!(
        compiled.spec.tasks[&api].cwd.as_deref(),
        Some(expected_cwd.as_path())
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
// profile可在入口中继承include声明的基础场景，且路径仍相对声明文件解析。
fn profile_inheritance_resolves_after_include_merge() {
    let root = temporary_directory("include-inheritance");
    fs::create_dir_all(root.join("fragment/work")).unwrap();
    fs::write(
        root.join("fragment/base.yaml"),
        "profiles:\n  common:\n    env: {FROM_BASE: yes}\n    task_defaults: {cwd: work}\n",
    )
    .unwrap();
    let entry = root.join("procora.yaml");
    fs::write(
        &entry,
        "include: [fragment/base.yaml]\nversion: 1\nproject: demo\nprofile: dev\nprofiles: {dev: {extends: common, env: {MODE: dev}}}\ntasks: {api: {command: api}}\n",
    )
    .unwrap();

    let compiled = load_path(&entry).unwrap();
    let api = &compiled.spec.tasks[&"api".parse().unwrap()];
    assert_eq!(api.env["FROM_BASE"], "yes");
    assert_eq!(api.env["MODE"], "dev");
    assert_eq!(
        api.cwd.as_deref(),
        Some(
            fs::canonicalize(root.join("fragment/work"))
                .unwrap()
                .as_path()
        )
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
// TUI切换profile后立即重编译预览，并在三种格式中保留未准入Task和profile定义。
fn tui_switches_profile_without_losing_inactive_tasks() {
    for extension in ["yaml", "toml", "json"] {
        let root = temporary_directory(extension);
        let path = root.join(format!("procora.{extension}"));
        let input = match extension {
            "yaml" => {
                "version: 1\nproject: demo\nprofile: dev\nprofiles:\n  dev: {tasks: [api]}\n  test: {tasks: [worker]}\ntasks:\n  api: {command: api}\n  worker: {command: worker}\n"
            }
            "toml" => {
                "version = 1\nproject = 'demo'\nprofile = 'dev'\n[profiles.dev]\ntasks = ['api']\n[profiles.test]\ntasks = ['worker']\n[tasks.api]\ncommand = 'api'\n[tasks.worker]\ncommand = 'worker'\n"
            }
            "json" => {
                r#"{"version":1,"project":"demo","profile":"dev","profiles":{"dev":{"tasks":["api"]},"test":{"tasks":["worker"]}},"tasks":{"api":{"command":"api"},"worker":{"command":"worker"}}}"#
            }
            _ => unreachable!(),
        };
        fs::write(&path, input).unwrap();
        let mut editor = ConfigEditor::open(&path).unwrap();

        press(&mut editor, KeyCode::Enter);
        for _ in 0..10 {
            press(&mut editor, KeyCode::Tab);
        }
        press(&mut editor, KeyCode::Right);
        press(&mut editor, KeyCode::Enter);
        editor.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));

        let saved = fs::read_to_string(&path).unwrap();
        let compiled = load_path(&path).unwrap();
        assert_eq!(
            compiled.active_profile.as_deref(),
            Some("test"),
            "{extension}"
        );
        assert_eq!(compiled.spec.tasks.len(), 1, "{extension}");
        assert!(compiled.spec.tasks.contains_key(&"worker".parse().unwrap()));
        assert!(saved.contains("api"), "{extension}: {saved}");
        assert!(saved.contains("worker"), "{extension}: {saved}");
        assert_eq!(compiled.profile_names.len(), 2);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
// TUI可结构化重命名profile，并同步继承引用后立即重编译有效预览。
fn tui_edits_profile_inheritance_across_formats() {
    for extension in ["yaml", "toml", "json"] {
        let root = temporary_directory(extension);
        let path = root.join(format!("procora.{extension}"));
        let input = match extension {
            "yaml" => {
                "version: 1\nproject: demo\nprofile: ci\nprofiles:\n  dev:\n    tasks: [worker]\n    env: {BASE: yes}\n  ci:\n    extends: dev\n    env: {MODE: ci}\ntasks:\n  api: {command: api}\n  worker: {command: worker}\n"
            }
            "toml" => {
                "version = 1\nproject = 'demo'\nprofile = 'ci'\n[profiles.dev]\ntasks = ['worker']\n[profiles.dev.env]\nBASE = 'yes'\n[profiles.ci]\nextends = 'dev'\n[profiles.ci.env]\nMODE = 'ci'\n[tasks.api]\ncommand = 'api'\n[tasks.worker]\ncommand = 'worker'\n"
            }
            "json" => {
                r#"{"version":1,"project":"demo","profile":"ci","profiles":{"dev":{"tasks":["worker"],"env":{"BASE":"yes"}},"ci":{"extends":"dev","env":{"MODE":"ci"}}},"tasks":{"api":{"command":"api"},"worker":{"command":"worker"}}}"#
            }
            _ => unreachable!(),
        };
        fs::write(&path, input).unwrap();
        let mut editor = ConfigEditor::open(&path).unwrap();

        // 保留既有 Project → Tasks → Dependencies 导航，再进入新增的 Profiles 区域。
        for _ in 0..3 {
            press(&mut editor, KeyCode::Tab);
        }
        press(&mut editor, KeyCode::Down);
        press(&mut editor, KeyCode::Enter);
        for _ in 0..3 {
            press(&mut editor, KeyCode::Backspace);
        }
        for character in "base".chars() {
            press(&mut editor, KeyCode::Char(character));
        }
        press(&mut editor, KeyCode::Enter);
        editor.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));

        let compiled = load_path(&path).unwrap();
        assert_eq!(
            compiled.active_profile.as_deref(),
            Some("ci"),
            "{extension}"
        );
        assert!(compiled.profile_names.contains("base"), "{extension}");
        assert!(!compiled.profile_names.contains("dev"), "{extension}");
        assert_eq!(compiled.spec.tasks.len(), 1, "{extension}");
        let worker = &compiled.spec.tasks[&"worker".parse().unwrap()];
        assert_eq!(worker.env["BASE"], "yes", "{extension}");
        assert_eq!(worker.env["MODE"], "ci", "{extension}");
        assert!(fs::read_to_string(&path).unwrap().contains("base"));
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
// TUI可新建和删除profile，并区分未声明白名单与显式空白名单。
fn tui_creates_and_deletes_profile() {
    let root = temporary_directory("profile-crud");
    let path = root.join("procora.yaml");
    fs::write(
        &path,
        "version: 1\nproject: demo\ntasks: {api: {command: api}}\n",
    )
    .unwrap();
    let mut editor = ConfigEditor::open(&path).unwrap();

    for _ in 0..3 {
        press(&mut editor, KeyCode::Tab);
    }
    press(&mut editor, KeyCode::Char('n'));
    for character in "empty".chars() {
        press(&mut editor, KeyCode::Char(character));
    }
    press(&mut editor, KeyCode::Tab);
    press(&mut editor, KeyCode::Tab);
    press(&mut editor, KeyCode::Char('['));
    press(&mut editor, KeyCode::Char(']'));
    press(&mut editor, KeyCode::Enter);
    editor.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));

    let compiled = load_path(&path).unwrap();
    assert!(compiled.profile_names.contains("empty"));
    assert!(fs::read_to_string(&path).unwrap().contains("\"tasks\":[]"));

    for _ in 0..3 {
        press(&mut editor, KeyCode::Tab);
    }
    press(&mut editor, KeyCode::Char('d'));
    press(&mut editor, KeyCode::Char('d'));
    editor.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
    assert!(load_path(&path).unwrap().profile_names.is_empty());
    fs::remove_dir_all(root).unwrap();
}

#[test]
// TUI拒绝删除仍被其他profile继承的基础声明。
fn tui_refuses_to_delete_referenced_profile() {
    let root = temporary_directory("profile-delete-reference");
    let path = root.join("procora.yaml");
    fs::write(
        &path,
        "version: 1\nproject: demo\nprofiles: {base: {}, child: {extends: base}}\ntasks: {api: {command: api}}\n",
    )
    .unwrap();
    let mut editor = ConfigEditor::open(&path).unwrap();

    for _ in 0..3 {
        press(&mut editor, KeyCode::Tab);
    }
    press(&mut editor, KeyCode::Char('d'));
    press(&mut editor, KeyCode::Char('d'));

    assert!(editor.message().contains("仍被 child 继承"));
    assert_eq!(load_path(&path).unwrap().profile_names.len(), 2);
    fs::remove_dir_all(root).unwrap();
}

#[test]
// CLI有效配置同时公开活动profile、可选名称、准入Task和逐字段来源。
fn cli_effective_config_explains_selected_profile() {
    let root = temporary_directory("cli");
    let path = root.join("procora.yaml");
    fs::write(
        &path,
        "version: 1\nproject: demo\nprofile: dev\nprofiles:\n  base:\n    task_defaults: {restart: always}\n  dev:\n    extends: base\n    tasks: [api]\ntasks:\n  api: {command: api}\n  worker: {command: worker}\n",
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
    assert_eq!(value["active_profile"], "dev");
    assert_eq!(value["profiles"], serde_json::json!(["base", "dev"]));
    assert_eq!(value["profile_extends"], serde_json::json!({"dev": "base"}));
    assert!(value["tasks"].get("api").is_some());
    assert!(value["tasks"].get("worker").is_none());
    assert_eq!(value["origins"]["api"]["fields"]["restart"], "profile");
    fs::remove_dir_all(root).unwrap();
}
