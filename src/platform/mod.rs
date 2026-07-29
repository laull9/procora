//! 操作系统识别、标准目录与可选平台能力。

use std::{
    io,
    path::{Path, PathBuf},
};

use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

pub mod autostart;
mod text;

pub use text::decode_external_output;

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
    static DATA_DIR: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
    DATA_DIR
        .get_or_init(|| {
            ProjectDirs::from("dev", "procora", "Procora")
                .map(|directories| simplify_path(directories.data_dir()))
        })
        .as_deref()
}

/// 规范化已存在路径，并移除 Windows 自动添加的扩展路径前缀。
///
/// # Errors
///
/// 当路径不存在或无法访问时返回文件系统错误。
pub fn canonicalize(path: impl AsRef<Path>) -> io::Result<PathBuf> {
    std::fs::canonicalize(path).map(|path| simplify_path(&path))
}

/// 返回不会向上层泄漏 Windows 扩展路径前缀的当前目录。
///
/// # Errors
///
/// 当进程当前目录不可访问时返回文件系统错误。
pub fn current_dir() -> io::Result<PathBuf> {
    std::env::current_dir().map(|path| simplify_path(&path))
}

/// 返回不会向外部命令泄漏 Windows 扩展路径前缀的当前可执行文件。
///
/// # Errors
///
/// 当操作系统无法定位当前可执行文件时返回文件系统错误。
pub fn current_exe() -> io::Result<PathBuf> {
    std::env::current_exe().map(|path| simplify_path(&path))
}

/// 返回不会向持久化或外部命令泄漏 Windows 扩展路径前缀的临时目录。
pub fn temp_dir() -> PathBuf {
    simplify_path(&std::env::temp_dir())
}

/// 把 Windows 扩展路径转换为等价的常规驱动器或 UNC 路径。
pub fn simplify_path(path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::{OsStrExt, OsStringExt};

        const VERBATIM_PREFIX: &[u16] = &[92, 92, 63, 92];

        let wide = path.as_os_str().encode_wide().collect::<Vec<_>>();
        let is_verbatim_unc = wide.get(..8).is_some_and(|prefix| {
            prefix[..4] == *VERBATIM_PREFIX
                && matches!(prefix[4], 85 | 117)
                && matches!(prefix[5], 78 | 110)
                && matches!(prefix[6], 67 | 99)
                && prefix[7] == 92
        });
        if is_verbatim_unc {
            let rest = &wide[8..];
            let normalized = [92_u16, 92_u16]
                .into_iter()
                .chain(rest.iter().copied())
                .collect::<Vec<_>>();
            return std::ffi::OsString::from_wide(&normalized).into();
        }
        if let Some(rest) = wide.strip_prefix(VERBATIM_PREFIX)
            && rest.len() >= 3
            && matches!(rest[0], 65..=90 | 97..=122)
            && rest[1] == 58
            && rest[2] == 92
        {
            return std::ffi::OsString::from_wide(rest).into();
        }
    }
    path.to_path_buf()
}
