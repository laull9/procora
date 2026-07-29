//! `push` 使用包内声明式导出项的临时物化。

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, bail};

/// 一个在上传完成前保持存活的包内导出来源。
pub(super) struct PackagePushSource {
    /// 交给现有 push 归档器的普通文件或目录。
    pub(super) source: PathBuf,
    /// 未显式指定远端 target 时使用的直觉化选择器。
    pub(super) default_target: String,
    /// 保持临时根目录存活并在调用结束后清理。
    _temporary: TemporaryExport,
}

/// 包导出临时目录的清理守卫。
struct TemporaryExport(PathBuf);

impl Drop for TemporaryExport {
    /// 上传结束后尽力清理已验证的临时物化目录。
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// 验证包、选择平台并返回一个声明式导出项。
pub(super) fn materialize(
    package: &Path,
    entry: &str,
    platform: &str,
) -> anyhow::Result<PackagePushSource> {
    if !crate::package::is_package_path(package) {
        bail!("`--package-entry` 的来源必须是 `.pcpkg` 文件");
    }
    let info = crate::package::inspect(package)?;
    let export = info
        .manifest
        .exports
        .get(entry)
        .with_context(|| {
            format!(
                "包没有导出项 `{entry}`；可用：{}",
                info.manifest
                    .exports
                    .keys()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("、")
            )
        })?
        .clone();
    let platform = if platform == "current" {
        crate::config::DeployPlatform::current()
            .normalized()
            .map_err(anyhow::Error::msg)?
    } else {
        crate::config::DeployPlatform::parse_key(platform)
            .map_err(anyhow::Error::msg)
            .with_context(|| format!("无效包导出平台 `{platform}`"))?
    };
    let root =
        crate::platform::temp_dir().join(format!("procora-push-package-{}", uuid::Uuid::new_v4()));
    crate::package::extract(package, &root, platform)?;
    let source = export
        .path
        .split('/')
        .fold(root.clone(), |path, segment| path.join(segment));
    let metadata = match fs::symlink_metadata(&source)
        .with_context(|| format!("包导出项 `{entry}` 在所选平台物化后不存在"))
    {
        Ok(metadata) => metadata,
        Err(error) => {
            let _ = fs::remove_dir_all(&root);
            return Err(error);
        }
    };
    if metadata.file_type().is_symlink()
        || match export.kind {
            crate::config::UploadKind::File => !metadata.is_file(),
            crate::config::UploadKind::Directory => !metadata.is_dir(),
        }
    {
        let _ = fs::remove_dir_all(&root);
        bail!("包导出项 `{entry}` 的物化类型与清单不一致");
    }
    Ok(PackagePushSource {
        source,
        default_target: format!("{}::{entry}", info.manifest.project),
        _temporary: TemporaryExport(root),
    })
}
