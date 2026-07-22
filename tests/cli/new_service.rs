//! 总览向导创建最小托管配置的集成契约。

use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use procora::{cli::api::initialize_managed_config, config::discover_path, platform::canonicalize};

/// 创建当前测试独占的临时服务目录。
fn temporary_directory() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory = std::env::temp_dir().join(format!(
        "procora-new-service-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(&directory).unwrap();
    directory
}

#[test]
// 新服务向导生成可立即发现的空Task配置，并且绝不覆盖已有入口。
fn managed_config_is_valid_and_exclusive() {
    let directory = temporary_directory();

    let path = initialize_managed_config(&directory, "guided-service").unwrap();
    let discovered = discover_path(&directory).unwrap();
    assert_eq!(canonicalize(&path).unwrap(), discovered.config_path);
    assert_eq!(discovered.compiled.spec.project, "guided-service");
    assert!(discovered.compiled.spec.tasks.is_empty());

    let original = fs::read_to_string(&path).unwrap();
    assert!(initialize_managed_config(&directory, "replacement").is_err());
    assert_eq!(fs::read_to_string(&path).unwrap(), original);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
// 无效服务名称在创建任何文件前被拒绝。
fn managed_config_rejects_invalid_name_without_side_effects() {
    let directory = temporary_directory();

    assert!(initialize_managed_config(&directory, "invalid service").is_err());
    assert!(!directory.join("procora.yaml").exists());
    fs::remove_dir_all(directory).unwrap();
}
