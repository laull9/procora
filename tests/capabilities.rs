//! 平台基础能力探测与目标系统契约测试。

use procora::platform::{PlatformKind, capabilities, data_dir};

/// Linux 构建必须暴露 Linux 平台与对应 systemd feature 状态。
#[cfg(target_os = "linux")]
#[test]
fn linux能力与编译feature保持一致() {
    let capabilities = capabilities();

    assert_eq!(capabilities.platform, PlatformKind::Linux);
    assert_eq!(capabilities.systemd, cfg!(feature = "systemd"));
}

/// macOS 构建不能误报 systemd 能力。
#[cfg(target_os = "macos")]
#[test]
fn macos能力不包含systemd() {
    let capabilities = capabilities();

    assert_eq!(capabilities.platform, PlatformKind::MacOs);
    assert!(!capabilities.systemd);
}

/// Windows 构建不能误报 systemd 能力。
#[cfg(windows)]
#[test]
fn windows能力不包含systemd() {
    let capabilities = capabilities();

    assert_eq!(capabilities.platform, PlatformKind::Windows);
    assert!(!capabilities.systemd);
}

#[test]
fn 支持平台声明受管进程树能力() {
    let capabilities = capabilities();

    assert!(matches!(
        capabilities.platform,
        PlatformKind::Linux | PlatformKind::MacOs | PlatformKind::Windows
    ));
    assert!(capabilities.managed_process_tree);
    assert!(data_dir().is_some());
}
