//! 操作系统识别、标准目录与可选平台能力。

use std::path::Path;

use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

pub mod autostart;

#[cfg(all(target_os = "linux", feature = "systemd"))]
pub mod systemd;

/// Procora 明确支持的操作系统类别。
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PlatformKind {
    /// Linux 原生环境。
    Linux,
    /// macOS 原生环境。
    MacOs,
    /// Windows 原生环境。
    Windows,
    /// 编译成功但不在正式支持矩阵内的平台。
    Unsupported,
}

/// 当前平台可向上层承诺的能力集合。
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PlatformCapabilities {
    /// 当前操作系统类别。
    pub platform: PlatformKind,
    /// 是否能可靠建立受管进程集合。
    pub managed_process_tree: bool,
    /// 是否可以启用 systemd 集成。
    pub systemd: bool,
}

/// 返回编译目标的基础平台能力。
pub const fn capabilities() -> PlatformCapabilities {
    let platform = if cfg!(target_os = "linux") {
        PlatformKind::Linux
    } else if cfg!(target_os = "macos") {
        PlatformKind::MacOs
    } else if cfg!(target_os = "windows") {
        PlatformKind::Windows
    } else {
        PlatformKind::Unsupported
    };
    PlatformCapabilities {
        platform,
        managed_process_tree: matches!(
            platform,
            PlatformKind::Linux | PlatformKind::MacOs | PlatformKind::Windows
        ),
        systemd: cfg!(all(target_os = "linux", feature = "systemd")),
    }
}

/// 返回当前用户的 Procora 数据目录。
pub fn data_dir() -> Option<&'static Path> {
    static PROJECT_DIRS: std::sync::OnceLock<Option<ProjectDirs>> = std::sync::OnceLock::new();
    PROJECT_DIRS
        .get_or_init(|| ProjectDirs::from("dev", "procora", "Procora"))
        .as_ref()
        .map(ProjectDirs::data_dir)
}
