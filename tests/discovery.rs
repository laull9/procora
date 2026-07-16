//! 服务目录配置自动发现测试。

use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use procora::{
    config::{DiscoveryError, discover_path},
    platform::canonicalize,
};

/// 创建当前测试独占的临时目录。
fn temporary_directory(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory = std::env::temp_dir().join(format!(
        "procora-config-{label}-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(&directory).unwrap();
    directory
}

/// 返回具有指定服务名称的最小合法配置。
fn valid_config(name: &str) -> String {
    format!("version: 1\nproject: {name}\ntasks:\n  task:\n    command: echo\n")
}

#[test]
fn 目录只选择唯一合法配置() {
    let directory = temporary_directory("unique");
    fs::write(directory.join("procora.yaml"), valid_config("demo")).unwrap();
    fs::write(directory.join("package.json"), "{\"name\":\"other\"}").unwrap();

    let discovered = discover_path(&directory).unwrap();

    assert_eq!(discovered.compiled.spec.project, "demo");
    assert_eq!(
        discovered.config_path,
        canonicalize(directory.join("procora.yaml")).unwrap()
    );
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn 多个procora配置要求显式选择() {
    let directory = temporary_directory("ambiguous");
    let yaml = directory.join("procora.yaml");
    fs::write(&yaml, valid_config("first")).unwrap();
    fs::write(
        directory.join("procora.json"),
        r#"{"version":1,"project":"second","tasks":{"task":{"command":"echo"}}}"#,
    )
    .unwrap();

    let error = discover_path(&directory).unwrap_err();
    assert!(matches!(error, DiscoveryError::Ambiguous { .. }));

    let discovered = discover_path(&yaml).unwrap();
    assert_eq!(discovered.compiled.spec.project, "first");
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn 没有合法配置时保留候选失败原因() {
    let directory = temporary_directory("invalid");
    fs::write(directory.join("procora.toml"), "name = 'not-procora'").unwrap();

    let error = discover_path(&directory).unwrap_err();
    assert!(matches!(error, DiscoveryError::NoValidConfig { .. }));
    assert!(error.to_string().contains("procora.toml"));
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn 目录忽略非procora名称的配置文件() {
    let directory = temporary_directory("ignore-other-configs");
    fs::write(directory.join("procora.yaml"), valid_config("demo")).unwrap();
    fs::write(directory.join("other.yaml"), valid_config("other")).unwrap();
    fs::write(directory.join("package.json"), "{\"version\": 1}").unwrap();

    let discovered = discover_path(&directory).unwrap();

    assert_eq!(discovered.compiled.spec.project, "demo");
    assert_eq!(
        discovered.config_path,
        canonicalize(directory.join("procora.yaml")).unwrap()
    );
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn 只有其他名称配置时报告未找到() {
    let directory = temporary_directory("only-other-configs");
    fs::write(directory.join("service.yaml"), valid_config("other")).unwrap();

    let error = discover_path(&directory).unwrap_err();

    assert!(matches!(error, DiscoveryError::NotFound(_)));
    fs::remove_dir_all(directory).unwrap();
}
