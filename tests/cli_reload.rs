//! 配置候选 CLI 的真实 daemon 往返测试。

use std::{
    fs,
    path::PathBuf,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

#[path = "support/command.rs"]
mod command_support;

use command_support::{remove_directory_when_released, run_background_cli};

/// 创建当前测试独占的临时目录。
fn temporary_directory(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory = std::env::temp_dir().join(format!(
        "procora-cli-reload-{label}-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(&directory).unwrap();
    directory
}

#[test]
fn preview与apply通过完整修订形成显式确认闭环() {
    let home = temporary_directory("home");
    let service = temporary_directory("service");
    let config = service.join("procora.yaml");
    fs::write(&config, "version: 1\nproject: reload-cli\ntasks: {}\n").unwrap();
    let binary = env!("CARGO_BIN_EXE_procora");
    let opened = run_background_cli(
        Command::new(binary)
            .arg("add")
            .arg(&service)
            .env("PROCORA_HOME", &home),
        &home,
        "open",
    );
    assert!(opened.status.success());

    fs::write(
        &config,
        "version: 1\nproject: reload-cli\ntasks:\n  once:\n    command: rustc\n    args: ['--version']\n",
    )
    .unwrap();
    let preview = Command::new(binary)
        .args(["preview", "reload-cli"])
        .env("PROCORA_HOME", &home)
        .output()
        .unwrap();
    assert!(preview.status.success());
    let stdout = String::from_utf8(preview.stdout).unwrap();
    assert!(stdout.contains("新增：once"));
    let revision = stdout
        .lines()
        .find_map(|line| line.strip_prefix("修订："))
        .expect("preview 应输出修订");
    assert_eq!(revision.len(), 64);

    let applied = Command::new(binary)
        .args(["apply", "reload-cli", revision])
        .env("PROCORA_HOME", &home)
        .output()
        .unwrap();
    assert!(applied.status.success());
    assert!(String::from_utf8_lossy(&applied.stdout).contains("1 个任务"));

    let down = Command::new(binary)
        .arg("down")
        .env("PROCORA_HOME", &home)
        .output()
        .unwrap();
    assert!(down.status.success());
    remove_directory_when_released(&home);
    fs::remove_dir_all(service).unwrap();
}
