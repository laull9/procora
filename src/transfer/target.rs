use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, bail};

use crate::config::UploadKind;
use crate::protocol::UploadTargetViewDto;

/// 一次上传从已注册服务配置解析出的最终目标。
pub(crate) struct ResolvedTarget {
    pub(crate) selector: String,
    pub(crate) root: PathBuf,
    pub(crate) path: PathBuf,
    pub(crate) kind: UploadKind,
    pub(crate) max_bytes: u64,
}

/// 从 `service::name` 或 `service::task::name` 解析已声明目标。
pub(crate) fn resolve(selector: &str) -> anyhow::Result<ResolvedTarget> {
    let (service_name, key) = parse_selector(selector)?;
    let upload = crate::cli::api::resolve_upload_target(service_name, &key)?;
    let relative = upload
        .path
        .strip_prefix(&upload.root)
        .context("Center 返回的上传目标不在 Service 根目录内")?;
    ensure_safe_parent(
        &upload.root,
        relative.parent().unwrap_or_else(|| Path::new("")),
    )?;
    let destination = upload.path;
    if fs::symlink_metadata(&destination).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        bail!("上传目标 `{}` 是符号链接，拒绝覆盖", destination.display());
    }
    Ok(ResolvedTarget {
        selector: selector.to_owned(),
        root: upload.root,
        path: destination,
        kind: upload.kind,
        max_bytes: upload.max_bytes,
    })
}

/// 列出 Center 当前已生效定义中的全部上传目标。
pub(crate) fn list() -> anyhow::Result<Vec<UploadTargetViewDto>> {
    crate::cli::api::list_upload_targets()
}

/// 校验选择器各段并返回服务名称与编译目标键。
fn parse_selector(selector: &str) -> anyhow::Result<(&str, String)> {
    let segments = selector.split("::").collect::<Vec<_>>();
    if !matches!(segments.len(), 2 | 3) || segments.iter().any(|value| !valid_id(value)) {
        bail!("上传目标应为 `service::name` 或 `service::task::name`");
    }
    let key = if segments.len() == 2 {
        segments[1].to_owned()
    } else {
        format!("{}::{}", segments[1], segments[2])
    };
    Ok((segments[0], key))
}

/// 判断选择器段能否作为稳定配置标识。
fn valid_id(value: &str) -> bool {
    !matches!(value, "" | "." | "..")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
}

/// 逐级创建目标父目录并拒绝经过已有符号链接。
fn ensure_safe_parent(root: &Path, relative: &Path) -> anyhow::Result<()> {
    let mut current = root.to_path_buf();
    for component in relative.components() {
        current.push(component.as_os_str());
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                bail!("上传目标父路径 `{}` 是符号链接", current.display());
            }
            Ok(metadata) if !metadata.is_dir() => {
                bail!("上传目标父路径 `{}` 不是目录", current.display());
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(&current)?;
            }
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}
