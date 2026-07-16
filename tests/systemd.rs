//! Linux systemd 用户/系统总线选择与权限失败回滚测试。

#![cfg(target_os = "linux")]

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

/// 当前测试进程内的临时目录去重序列。
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// 创建当前测试独占的临时目录。
fn temporary_directory(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let directory = std::env::temp_dir().join(format!(
        "procora-systemd-{label}-{}-{nonce}-{sequence}",
        std::process::id()
    ));
    fs::create_dir_all(&directory).unwrap();
    directory
}

#[cfg(feature = "systemd")]
#[test]
fn dbus用户与系统范围必须由调用方显式选择() {
    use procora::platform::systemd::SystemdBus;

    assert_eq!(SystemdBus::User.label(), "用户总线");
    assert_eq!(SystemdBus::System.label(), "系统总线");
    let _user_query = procora::platform::systemd::list_unit_names(SystemdBus::User);
    let _system_query = procora::platform::systemd::list_unit_names(SystemdBus::System);
}

#[test]
fn systemctl权限失败会返回诊断并回滚新单元文件() {
    let root = temporary_directory("permission");
    let bin = root.join("bin");
    let config = root.join("config");
    let home = root.join("home");
    let data = root.join("data");
    fs::create_dir(&bin).unwrap();
    fs::create_dir(&config).unwrap();
    fs::create_dir(&home).unwrap();
    fs::create_dir(&data).unwrap();
    write_denied_systemctl(&bin);

    let output = Command::new(env!("CARGO_BIN_EXE_procora"))
        .arg("enable")
        .env("PATH", &bin)
        .env("HOME", &home)
        .env("XDG_CONFIG_HOME", &config)
        .env("PROCORA_HOME", &data)
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Access denied by test"));
    assert!(!config.join("systemd/user/procora.service").exists());
    fs::remove_dir_all(root).unwrap();
}

/// 写入始终模拟 D-Bus 权限拒绝的假 systemctl。
fn write_denied_systemctl(bin: &Path) {
    let path = bin.join("systemctl");
    fs::write(
        &path,
        "#!/bin/sh\nprintf 'Access denied by test\\n' >&2\nexit 1\n",
    )
    .unwrap();
    let mut permissions = fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(path, permissions).unwrap();
}
