//! 可读时长语法、旧毫秒兼容、组合层与结构化编辑器测试。

use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use procora::{
    config::{ConfigError, ConfigFormat, load_str},
    tui::ConfigEditor,
};

/// 并行测试创建独占配置时使用的进程内序号。
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// 创建当前测试独占的 YAML 路径。
fn temporary_config() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "procora-duration-{}-{nonce}-{sequence}.yaml",
        std::process::id()
    ))
}

/// 向配置表单发送一个无修饰键。
fn press(editor: &mut ConfigEditor, code: KeyCode) {
    editor.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
}

/// 清空当前弹窗字段。
fn clear_field(editor: &mut ConfigEditor, characters: usize) {
    for _ in 0..characters {
        press(editor, KeyCode::Backspace);
    }
}

/// 向当前弹窗字段逐字输入。
fn type_text(editor: &mut ConfigEditor, value: &str) {
    for character in value.chars() {
        press(editor, KeyCode::Char(character));
    }
}

#[test]
// YAML、TOML 与 JSON 的可读时长在默认层、profile、模板和健康检查中语义一致。
fn readable_durations_are_equivalent_across_formats_and_layers() {
    let cases = [
        (
            ConfigFormat::Yaml,
            "version: 1\nproject: demo\nprofile: fast\ntask_defaults:\n  restart_delay: 750ms\n  restart_reset_after: 1m30s\n  shutdown_timeout: 5s\nprofiles:\n  fast:\n    task_defaults:\n      shutdown_timeout: 3s\ntask_templates:\n  worker:\n    command: worker\n    restart_delay: 2s\ntasks:\n  api:\n    extends: worker\n    restart_reset_after: 2m\n    healthcheck:\n      command: checker\n      initial_delay: 250ms\n      period: 1m30s\n      timeout: 1s500ms\n",
        ),
        (
            ConfigFormat::Toml,
            "version = 1\nproject = 'demo'\nprofile = 'fast'\n[task_defaults]\nrestart_delay = '750ms'\nrestart_reset_after = '1m30s'\nshutdown_timeout = '5s'\n[profiles.fast.task_defaults]\nshutdown_timeout = '3s'\n[task_templates.worker]\ncommand = 'worker'\nrestart_delay = '2s'\n[tasks.api]\nextends = 'worker'\nrestart_reset_after = '2m'\n[tasks.api.healthcheck]\ncommand = 'checker'\ninitial_delay = '250ms'\nperiod = '1m30s'\ntimeout = '1s500ms'\n",
        ),
        (
            ConfigFormat::Json,
            r#"{"version":1,"project":"demo","profile":"fast","task_defaults":{"restart_delay":"750ms","restart_reset_after":"1m30s","shutdown_timeout":"5s"},"profiles":{"fast":{"task_defaults":{"shutdown_timeout":"3s"}}},"task_templates":{"worker":{"command":"worker","restart_delay":"2s"}},"tasks":{"api":{"extends":"worker","restart_reset_after":"2m","healthcheck":{"command":"checker","initial_delay":"250ms","period":"1m30s","timeout":"1s500ms"}}}}"#,
        ),
    ];
    let compiled = cases
        .into_iter()
        .map(|(format, input)| load_str(input, format).unwrap())
        .collect::<Vec<_>>();

    assert_eq!(compiled[0].spec, compiled[1].spec);
    assert_eq!(compiled[1].spec, compiled[2].spec);
    let task = compiled[0].spec.tasks.values().next().unwrap();
    let health = task.healthcheck.as_ref().unwrap();
    assert_eq!(task.restart_delay_ms, 2_000);
    assert_eq!(task.restart_reset_after_ms, 120_000);
    assert_eq!(task.shutdown_timeout_ms, 3_000);
    assert_eq!(health.initial_delay_ms, 250);
    assert_eq!(health.period_ms, 90_000);
    assert_eq!(health.timeout_ms, 1_500);
}

#[test]
// 新字段与旧_ms整数写法规范化为完全相同的领域配置。
fn readable_fields_are_equivalent_to_legacy_milliseconds() {
    let readable = load_str(
        "version: 1\nproject: demo\ntask_defaults:\n  restart_delay: 750ms\n  restart_reset_after: 9s\n  shutdown_timeout: 3s\ntasks:\n  api:\n    command: api\n    restart_delay: 2s\n    restart_reset_after: 1m\n    shutdown_timeout: 5s\n    healthcheck:\n      command: checker\n      initial_delay: 250ms\n      period: 10s\n      timeout: 1s\n",
        ConfigFormat::Yaml,
    )
    .unwrap();
    let legacy = load_str(
        "version: 1\nproject: demo\ntask_defaults:\n  restart_delay_ms: 750\n  restart_reset_after_ms: 9000\n  shutdown_timeout_ms: 3000\ntasks:\n  api:\n    command: api\n    restart_delay_ms: 2000\n    restart_reset_after_ms: 60000\n    shutdown_timeout_ms: 5000\n    healthcheck:\n      command: checker\n      initial_delay_ms: 250\n      period_ms: 10000\n      timeout_ms: 1000\n",
        ConfigFormat::Yaml,
    )
    .unwrap();

    assert_eq!(readable.spec, legacy.spec);
    assert_eq!(readable.task_defaults, legacy.task_defaults);
}

#[test]
// 时长字符串拒绝缺失单位、空白、重复或乱序单位、未知单位和溢出。
fn invalid_duration_syntax_is_rejected_strictly() {
    for value in [
        "1",
        "1.5s",
        " 1s",
        "1 s",
        "1s2s",
        "1ms2s",
        "1d",
        "18446744073709551615h",
    ] {
        let input = format!(
            "version: 1\nproject: demo\ntasks:\n  api:\n    command: api\n    restart_delay: '{value}'\n"
        );
        let error = load_str(&input, ConfigFormat::Yaml).unwrap_err();
        assert!(
            matches!(error, ConfigError::Parse { .. }),
            "`{value}` 应在解析阶段失败：{error}"
        );
    }
}

#[test]
// 同时声明新旧字段会被识别为冲突，不会依赖输入顺序静默覆盖。
fn readable_and_legacy_aliases_cannot_be_declared_together() {
    for input in [
        "version: 1\nproject: demo\ntasks:\n  api:\n    command: api\n    restart_delay: 1s\n    restart_delay_ms: 1000\n",
        "version: 1\nproject: demo\ntasks:\n  api:\n    command: api\n    healthcheck:\n      command: checker\n      period: 1s\n      period_ms: 1000\n",
    ] {
        let error = load_str(input, ConfigFormat::Yaml).unwrap_err().to_string();
        assert!(error.contains("duplicate field"), "冲突诊断不明确：{error}");
    }
}

#[test]
// 可读时长完成解析后仍经过既有运行边界校验，并保留稳定毫秒字段路径。
fn readable_durations_keep_runtime_limit_validation() {
    let error = load_str(
        "version: 1\nproject: demo\ntasks:\n  api:\n    command: api\n    restart_delay: 31s\n    healthcheck:\n      command: checker\n      period: 6m\n",
        ConfigFormat::Yaml,
    )
    .unwrap_err();
    let ConfigError::Validation { diagnostics, .. } = error else {
        panic!("越界时长应返回结构化校验诊断");
    };
    assert!(
        diagnostics
            .iter()
            .any(|item| item.path == "tasks.api.restart_delay_ms")
    );
    assert!(
        diagnostics
            .iter()
            .any(|item| item.path == "tasks.api.healthcheck.period_ms")
    );
}

#[test]
// TUI可直接编辑组合时长，并以新字段和稳定紧凑值保存。
fn form_edits_and_saves_readable_durations() {
    let path = temporary_config();
    fs::write(
        &path,
        "version: 1\nproject: demo\ntasks:\n  api:\n    command: api\n    restart_delay: 500ms\n    healthcheck:\n      command: checker\n      period: 10s\n      timeout: 1s\n",
    )
    .unwrap();
    let mut editor = ConfigEditor::open(&path).unwrap();

    press(&mut editor, KeyCode::Tab);
    press(&mut editor, KeyCode::Enter);
    for _ in 0..9 {
        press(&mut editor, KeyCode::Tab);
    }
    clear_field(&mut editor, 5);
    type_text(&mut editor, "1s500ms");
    press(&mut editor, KeyCode::Enter);

    press(&mut editor, KeyCode::Char('h'));
    for _ in 0..5 {
        press(&mut editor, KeyCode::Tab);
    }
    clear_field(&mut editor, 3);
    type_text(&mut editor, "1m30s");
    press(&mut editor, KeyCode::Enter);
    editor.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));

    let saved = fs::read_to_string(&path).unwrap();
    let compiled = load_str(&saved, ConfigFormat::Yaml).unwrap();
    let task = compiled.spec.tasks.values().next().unwrap();
    assert!(saved.contains("restart_delay: \"1500ms\""));
    assert!(saved.contains("period: \"90s\""));
    assert!(!saved.contains("restart_delay_ms:"));
    assert!(!saved.contains("period_ms:"));
    assert_eq!(task.restart_delay_ms, 1_500);
    assert_eq!(task.healthcheck.as_ref().unwrap().period_ms, 90_000);
    fs::remove_file(path).unwrap();
}

#[test]
// 三种结构化格式都会把旧毫秒声明稳定写回为带单位的新字段。
fn form_normalizes_readable_durations_across_formats() {
    for (extension, format, input) in [
        (
            "yaml",
            ConfigFormat::Yaml,
            "version: 1\nproject: demo\ntasks:\n  api:\n    command: api\n    restart_delay_ms: 750\n",
        ),
        (
            "toml",
            ConfigFormat::Toml,
            "version = 1\nproject = 'demo'\n[tasks.api]\ncommand = 'api'\nrestart_delay_ms = 750\n",
        ),
        (
            "json",
            ConfigFormat::Json,
            r#"{"version":1,"project":"demo","tasks":{"api":{"command":"api","restart_delay_ms":750}}}"#,
        ),
    ] {
        let path = temporary_config().with_extension(extension);
        fs::write(&path, input).unwrap();
        let mut editor = ConfigEditor::open(&path).unwrap();
        editor.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));

        let saved = fs::read_to_string(&path).unwrap();
        let compiled = load_str(&saved, format).unwrap();
        assert!(saved.contains("restart_delay"), "{extension}: {saved}");
        assert!(saved.contains("750ms"), "{extension}: {saved}");
        assert!(!saved.contains("restart_delay_ms"), "{extension}: {saved}");
        assert_eq!(
            compiled
                .spec
                .tasks
                .values()
                .next()
                .unwrap()
                .restart_delay_ms,
            750
        );
        fs::remove_file(path).unwrap();
    }
}
