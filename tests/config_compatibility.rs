//! 旧写法、新简写和各配置前端的长期兼容夹具。

use procora::config::{ConfigError, ConfigFormat, load_str};

/// 一个固定输入格式和夹具内容。
struct CompatibilityCase {
    name: &'static str,
    format: ConfigFormat,
    input: &'static str,
}

/// 所有应产生同一领域规范的兼容表示。
const EQUIVALENT_CASES: &[CompatibilityCase] = &[
    CompatibilityCase {
        name: "legacy-yaml",
        format: ConfigFormat::Yaml,
        input: include_str!("fixtures/config/equivalent/legacy.yaml"),
    },
    CompatibilityCase {
        name: "current-yaml",
        format: ConfigFormat::Yaml,
        input: include_str!("fixtures/config/equivalent/current.yaml"),
    },
    CompatibilityCase {
        name: "current-toml",
        format: ConfigFormat::Toml,
        input: include_str!("fixtures/config/equivalent/current.toml"),
    },
    CompatibilityCase {
        name: "current-json",
        format: ConfigFormat::Json,
        input: include_str!("fixtures/config/equivalent/current.json"),
    },
    CompatibilityCase {
        name: "python-output-json",
        format: ConfigFormat::Json,
        input: include_str!("fixtures/config/equivalent/python-output.json"),
    },
    CompatibilityCase {
        name: "task-templates-yaml",
        format: ConfigFormat::Yaml,
        input: include_str!("fixtures/config/equivalent/templates.yaml"),
    },
    CompatibilityCase {
        name: "profile-yaml",
        format: ConfigFormat::Yaml,
        input: include_str!("fixtures/config/equivalent/profile.yaml"),
    },
    CompatibilityCase {
        name: "profile-inheritance-yaml",
        format: ConfigFormat::Yaml,
        input: include_str!("fixtures/config/equivalent/profile-inheritance.yaml"),
    },
    CompatibilityCase {
        name: "variables-yaml",
        format: ConfigFormat::Yaml,
        input: include_str!("fixtures/config/equivalent/variables.yaml"),
    },
    CompatibilityCase {
        name: "dependency-list-yaml",
        format: ConfigFormat::Yaml,
        input: include_str!("fixtures/config/equivalent/dependency-list.yaml"),
    },
    CompatibilityCase {
        name: "dependency-scalar-yaml",
        format: ConfigFormat::Yaml,
        input: include_str!("fixtures/config/equivalent/dependency-scalar.yaml"),
    },
    CompatibilityCase {
        name: "readable-durations-yaml",
        format: ConfigFormat::Yaml,
        input: include_str!("fixtures/config/equivalent/durations.yaml"),
    },
];

#[test]
// 旧版重复声明和新版默认层在所有前端中保持同一运行语义与任务图。
fn equivalent_fixtures_produce_same_project_and_graph() {
    let first = load_str(EQUIVALENT_CASES[0].input, EQUIVALENT_CASES[0].format).unwrap();
    for case in &EQUIVALENT_CASES[1..] {
        let compiled = load_str(case.input, case.format)
            .unwrap_or_else(|error| panic!("{} 加载失败：{error}", case.name));
        assert_eq!(compiled.spec, first.spec, "{} 领域语义变化", case.name);
        assert_eq!(
            compiled.graph.start_order(),
            first.graph.start_order(),
            "{} 任务图变化",
            case.name
        );
    }
}

#[test]
// 无Task时非法默认层夹具仍返回稳定字段路径而不是被静默忽略。
fn invalid_fixture_keeps_task_default_diagnostics() {
    let error = load_str(
        include_str!("fixtures/config/invalid/task-defaults.yaml"),
        ConfigFormat::Yaml,
    )
    .unwrap_err();
    let ConfigError::Validation { diagnostics, .. } = error else {
        panic!("非法默认层应返回结构化校验错误");
    };
    assert!(
        diagnostics
            .iter()
            .any(|item| item.path == "task_defaults.restart_delay_ms")
    );
    assert!(
        diagnostics
            .iter()
            .any(|item| item.path == "task_defaults.success_exit_codes")
    );
}

#[test]
// Task运行目录接受直观的dir输入别名，并统一进入cwd领域字段。
fn task_dir_alias_normalizes_to_cwd() {
    for (format, input) in [
        (
            ConfigFormat::Yaml,
            "version: 1\nproject: demo\ntasks:\n  api:\n    command: api\n    dir: work\n",
        ),
        (
            ConfigFormat::Toml,
            "version = 1\nproject = 'demo'\n[tasks.api]\ncommand = 'api'\ndir = 'work'\n",
        ),
        (
            ConfigFormat::Json,
            r#"{"version":1,"project":"demo","tasks":{"api":{"command":"api","dir":"work"}}}"#,
        ),
    ] {
        let compiled = load_str(input, format).unwrap();
        let task = compiled.spec.tasks.values().next().unwrap();
        assert_eq!(task.cwd.as_deref(), Some(std::path::Path::new("work")));
    }
}

#[test]
// 同时声明dir和cwd会被当成重复字段拒绝，避免运行目录含糊。
fn task_rejects_dir_and_cwd_together() {
    let error = load_str(
        "version: 1\nproject: demo\ntasks:\n  api:\n    command: api\n    dir: one\n    cwd: two\n",
        ConfigFormat::Yaml,
    )
    .unwrap_err();

    assert!(error.to_string().contains("duplicate field"));
}
