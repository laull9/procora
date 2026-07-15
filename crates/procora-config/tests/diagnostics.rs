//! 原始配置规范化、字段路径和多错误诊断测试。

use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use procora_config::{ConfigError, ConfigFormat, load_path, load_str};

#[test]
fn 独立语义错误会一次返回() {
    let error = load_str(
        "version: 2\nproject: bad/name\ntasks:\n  one:\n    command: ''\n    shutdown_timeout_ms: 0\n  two:\n    command: ''\n    depends_on:\n      missing: {}\n",
        ConfigFormat::Yaml,
    )
    .unwrap_err();
    let ConfigError::Validation { diagnostics, .. } = error else {
        panic!("应返回结构化语义诊断");
    };
    assert!(diagnostics.len() >= 6);
    assert!(diagnostics.iter().any(|item| item.path == "project"));
    assert!(
        diagnostics
            .iter()
            .any(|item| item.path == "tasks.two.depends_on.missing")
    );
}

#[test]
fn 三种格式的类型错误都包含精确字段路径() {
    let cases = [
        (
            ConfigFormat::Yaml,
            "version: 1\nproject: demo\ntasks:\n  api:\n    command: [42]\n",
        ),
        (
            ConfigFormat::Toml,
            "version = 1\nproject = 'demo'\n[tasks.api]\ncommand = [42]\n",
        ),
        (
            ConfigFormat::Json,
            r#"{"version":1,"project":"demo","tasks":{"api":{"command":[42]}}}"#,
        ),
    ];
    for (format, input) in cases {
        let error = load_str(input, format).unwrap_err().to_string();
        assert!(error.contains("tasks.api.command"), "{format}: {error}");
        assert!(
            error.contains('行') || error.contains("line "),
            "{format}: {error}"
        );
    }
}

#[test]
fn 相对工作目录以配置文件目录为基准规范化() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory = std::env::temp_dir().join(format!("procora-config-path-{nonce}"));
    fs::create_dir_all(&directory).unwrap();
    let config = directory.join("procora.yaml");
    fs::write(
        &config,
        "version: 1\nproject: demo\ntasks:\n  api:\n    command: echo\n    cwd: ./sub/../app\n",
    )
    .unwrap();

    let compiled = load_path(&config).unwrap();
    let cwd = compiled.spec.tasks.values().next().unwrap().cwd.clone();
    assert_eq!(
        cwd,
        Some(
            fs::canonicalize(&directory)
                .unwrap()
                .join(PathBuf::from("app"))
        )
    );
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn 生命周期时间拒绝可能冻结中心服务器的极端值() {
    let error = load_str(
        "version: 1\nproject: demo\ntasks:\n  app:\n    command: echo\n    restart_delay_ms: 30001\n    shutdown_timeout_ms: 300001\n",
        ConfigFormat::Yaml,
    )
    .expect_err("极端生命周期时间必须被拒绝");
    let message = error.to_string();
    assert!(message.contains("restart_delay_ms"));
    assert!(message.contains("shutdown_timeout_ms"));
}
