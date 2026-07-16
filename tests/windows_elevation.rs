//! Windows UAC 提权命令的静态契约测试。

#[cfg(target_os = "windows")]
use std::{fs, process::Command};

#[cfg(target_os = "windows")]
use uuid::Uuid;

#[cfg(target_os = "windows")]
#[test]
fn 自启动提权脚本显式唤起uac并等待结果() {
    let script = procora::cli::elevation_script_for_test();

    assert!(script.contains("-Verb RunAs"));
    assert!(script.contains("-Wait"));
    assert!(script.contains("-PassThru"));
    assert!(script.contains("-WindowStyle Hidden"));
    assert!(script.contains("__elevated-autostart"));
}

#[cfg(target_os = "windows")]
#[test]
fn 提权子进程会把完整错误写回父进程() {
    let result = std::env::temp_dir().join(format!("procora-uac-test-{}.result", Uuid::new_v4()));
    let output = Command::new(env!("CARGO_BIN_EXE_procora"))
        .args([
            "__elevated-autostart",
            "invalid",
            "--result",
            &result.to_string_lossy(),
        ])
        .output()
        .unwrap();

    assert!(output.status.success());
    let content = fs::read_to_string(&result).unwrap();
    assert!(content.starts_with("error\n"));
    assert!(content.contains("未知的 Windows 提权操作"));
    fs::remove_file(result).unwrap();
}
