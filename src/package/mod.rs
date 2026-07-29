//! 可独立构建、验证并按平台物化的 Procora Service 包。

mod build;
mod extract;
mod manifest;
mod read;

pub use build::{PackageBuildResult, PackagePlatform, build};
pub use extract::{PackageExtractResult, extract};
pub use manifest::{
    PACKAGE_FORMAT_V1, PackageBinary, PackageBinaryVariant, PackageConfig, PackageExport,
    PackageFile, PackageManifest,
};
pub use read::{PackageInfo, inspect, verify};

use std::path::Path;

/// 判断用户路径是否显式指向 `.pcpkg` 文件。
pub fn is_package_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("pcpkg"))
}
