//! 无运行副作用的 CLI 易用性命令测试。

use std::{
    fs,
    path::PathBuf,
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use serde_json::json;

/// 同进程并行测试的临时目录序号。
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// 创建 CLI 易用性测试独占目录。
fn temporary_directory() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("系统时间应晚于 Unix 纪元")
        .as_nanos();
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let directory = std::env::temp_dir().join(format!(
        "procora-cli-usability-{}-{nonce}-{sequence}",
        std::process::id()
    ));
    fs::create_dir_all(&directory).expect("应能创建测试目录");
    directory
}

#[test]
// completions可以输出无需运行服务的shell脚本。
fn completions_generate_shell_scripts_without_server() {
    for shell in ["bash", "zsh", "fish", "powershell"] {
        let output = Command::new(env!("CARGO_BIN_EXE_procora"))
            .args(["completions", shell])
            .output()
            .expect("补全命令应能执行");
        assert!(output.status.success(), "{shell} 补全生成失败");
        assert!(!output.stdout.is_empty(), "{shell} 补全不应为空");
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("procora"),
            "{shell} 补全应包含命令名"
        );
    }
}

#[test]
// config输出包含默认值和规范化路径的有效_json。
fn config_outputs_valid_json_with_defaults_and_paths() {
    let directory = temporary_directory();
    fs::create_dir_all(directory.join("work")).expect("应能创建工作目录");
    fs::write(
        directory.join("procora.yaml"),
        "version: 1\nproject: effective\nenv:\n  RUST_LOG: info\ntask_defaults:\n  env:\n    TASK_SCOPE: shared\n  restart: on-failure\ntasks:\n  api:\n    command: [api, '--mode', 'hello world']\n    cwd: ./work\n",
    )
    .expect("应能写入配置");

    let output = Command::new(env!("CARGO_BIN_EXE_procora"))
        .args(["config", "."])
        .current_dir(&directory)
        .output()
        .expect("有效配置命令应能执行");

    assert!(output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).expect("输出应为 JSON");
    assert_eq!(value["project"], "effective");
    assert_eq!(value["env"]["RUST_LOG"], "info");
    assert_eq!(value["tasks"]["api"]["command"], "api");
    assert_eq!(
        value["tasks"]["api"]["args"],
        json!(["--mode", "hello world"])
    );
    assert_eq!(value["tasks"]["api"]["env"]["RUST_LOG"], "info");
    assert_eq!(value["tasks"]["api"]["env"]["TASK_SCOPE"], "shared");
    assert_eq!(value["tasks"]["api"]["restart"], "on-failure");
    assert_eq!(value["task_defaults"]["restart"], "on-failure");
    assert_eq!(value["tasks"]["api"]["success_exit_codes"], json!([0]));
    assert_eq!(value["origins"]["api"]["fields"]["command"], "task");
    assert_eq!(value["origins"]["api"]["fields"]["args"], "task");
    assert_eq!(
        value["origins"]["api"]["fields"]["restart"],
        "task_defaults"
    );
    assert_eq!(value["origins"]["api"]["env"]["RUST_LOG"], "project_env");
    assert_eq!(
        value["origins"]["api"]["env"]["TASK_SCOPE"],
        "task_defaults"
    );
    assert!(PathBuf::from(value["tasks"]["api"]["cwd"].as_str().unwrap()).is_absolute());
    fs::remove_dir_all(directory).expect("应能清理测试目录");
}
