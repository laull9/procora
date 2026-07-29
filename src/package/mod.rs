//! 可独立构建、验证并按平台物化的 Procora Service 包。

mod build;
mod catalog;
mod extract;
mod manifest;
mod read;

pub use build::{PackageBuildResult, PackagePlatform, build, build_replacing};
pub use catalog::{
    InstalledCatalog, InstalledRelease, InstalledService, StoredPackage, installed_catalog,
};
pub use extract::{PackageExtractResult, extract};
pub use manifest::{
    PACKAGE_FORMAT_V1, PackageBinary, PackageBinaryVariant, PackageConfig, PackageExport,
    PackageFile, PackageManifest,
};
pub use read::{PackageInfo, inspect, verify};

use std::{fs, path::Path};

use anyhow::{Context, bail};

/// 判断用户路径是否显式指向 `.pcpkg` 文件。
pub fn is_package_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("pcpkg"))
}

/// 永久删除一个显式选择的 `.pcpkg` 普通文件或符号链接。
///
/// # Errors
///
/// 当路径不是 `.pcpkg`、目标不是普通文件或符号链接，或文件读取、删除失败时返回错误。
pub fn delete_file(path: &Path) -> anyhow::Result<()> {
    let path = crate::platform::simplify_path(path);
    if !is_package_path(&path) {
        bail!("只能删除扩展名为 `.pcpkg` 的 Procora 包文件");
    }
    let metadata = fs::symlink_metadata(&path)
        .with_context(|| format!("无法读取包文件 `{}`", path.display()))?;
    if !metadata.is_file() && !metadata.file_type().is_symlink() {
        bail!("拒绝删除非普通包文件 `{}`", path.display());
    }
    fs::remove_file(&path).with_context(|| format!("无法删除包文件 `{}`", path.display()))
}
