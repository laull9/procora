//! 裸机部署的平台选择、归档构造与预检修订。

use std::path::{Component, Path};

use anyhow::{Context, bail};
use sha2::{Digest, Sha256};

use crate::config::{DeployBinaries, DeployPlatform, select_deploy_binaries};

use super::{
    archive::{self, PreparedArchive},
    deploy_platform::resolve_remote_platform,
    deploy_protocol::DeployBinaryMetadata,
    deploy_report::{DeployBinaryChoice, DeployPreview},
    deploy_wire::MAX_DEPLOY_BYTES,
    remote_auth::SshAuth,
};

/// 已完成平台选择和归档构造的部署输入。
pub(super) struct PreparedDeployment {
    pub(super) archive: PreparedArchive,
    pub(super) target_platform: DeployPlatform,
    pub(super) binaries: Vec<DeployBinaryMetadata>,
    pub(super) choices: Vec<DeployBinaryChoice>,
}

/// 探测平台、选择唯一变体并构造不夹带其他平台产物的归档。
#[allow(clippy::too_many_arguments)]
pub(super) fn prepare_deployment(
    source: &Path,
    binaries: &DeployBinaries,
    ssh_target: &str,
    configured_remote_bin: Option<&str>,
    remote_bin: &mut String,
    auth: &mut SshAuth,
    batch: bool,
) -> anyhow::Result<PreparedDeployment> {
    let platform =
        resolve_remote_platform(ssh_target, configured_remote_bin, remote_bin, auth, batch)?;
    let selected = if binaries.is_empty() {
        Vec::new()
    } else {
        select_deploy_binaries(binaries, &platform).map_err(anyhow::Error::msg)?
    };
    let archive = archive::prepare_deploy(source, binaries, &selected)?;
    if archive.archive_bytes > MAX_DEPLOY_BYTES || archive.content_bytes > MAX_DEPLOY_BYTES {
        bail!("部署包压缩前后都不能超过 {MAX_DEPLOY_BYTES} 字节");
    }
    let metadata = deploy_binary_metadata(&selected)?;
    let choices = selected
        .into_iter()
        .zip(&metadata)
        .map(|(binary, metadata)| DeployBinaryChoice {
            name: metadata.name.clone(),
            selector: metadata.selector.clone(),
            source: binary.source,
            target: metadata.target.clone(),
            bytes: metadata.bytes,
            sha256: metadata.sha256.clone(),
        })
        .collect();
    Ok(PreparedDeployment {
        archive,
        target_platform: platform,
        binaries: metadata,
        choices,
    })
}

/// 从已探测且已归档的输入生成可防止TOCTOU的稳定预检修订。
#[allow(clippy::too_many_arguments)]
pub(super) fn build_preview(
    source: &Path,
    project: &str,
    config_path: &Path,
    ssh_target: &str,
    remote_bin: &str,
    timeout_ms: u64,
    stable_for_ms: u64,
    keep: u32,
    prepared: &PreparedDeployment,
) -> anyhow::Result<DeployPreview> {
    let mut preview = DeployPreview {
        project: project.to_owned(),
        source: source.to_path_buf(),
        config_path: portable_relative_path(config_path)?,
        ssh_target: ssh_target.to_owned(),
        remote_bin: remote_bin.to_owned(),
        target_platform: prepared.target_platform.clone(),
        binaries: prepared.choices.clone(),
        content_bytes: prepared.archive.content_bytes,
        archive_bytes: prepared.archive.archive_bytes,
        archive_sha256: prepared.archive.sha256.clone(),
        timeout_ms,
        stable_for_ms,
        keep,
        revision: String::new(),
    };
    let bytes = serde_json::to_vec(&preview)?;
    preview.revision = format!("{:x}", Sha256::digest(bytes));
    Ok(preview)
}

/// 计算即将提交的本地产物摘要，供远端对release内容复核。
fn deploy_binary_metadata(
    selected: &[crate::config::SelectedDeployBinary],
) -> anyhow::Result<Vec<DeployBinaryMetadata>> {
    selected
        .iter()
        .map(|binary| {
            let metadata = std::fs::metadata(&binary.source).with_context(|| {
                format!(
                    "无法读取二进制 `{}`：{}",
                    binary.name,
                    binary.source.display()
                )
            })?;
            if metadata.len() == 0 {
                bail!("二进制 `{}` 不能为空文件", binary.name);
            }
            Ok(DeployBinaryMetadata {
                name: binary.name.clone(),
                selector: binary.selector.clone(),
                target: portable_relative_path(&binary.target)?,
                bytes: metadata.len(),
                sha256: archive::hash_file(&binary.source)?,
            })
        })
        .collect()
}

/// 把本机配置相对路径编码为与远端平台无关的`/`分隔文本。
pub(super) fn portable_relative_path(path: &Path) -> anyhow::Result<String> {
    let mut segments = Vec::new();
    for component in path.components() {
        let Component::Normal(segment) = component else {
            bail!("部署配置入口必须是普通相对路径");
        };
        let segment = segment.to_str().context("部署配置入口必须是 UTF-8")?;
        if !portable_segment(segment) {
            bail!("部署配置入口包含不可移植的路径字符");
        }
        segments.push(segment);
    }
    if segments.is_empty() {
        bail!("部署配置入口不能为空");
    }
    Ok(segments.join("/"))
}

/// 拒绝`Windows`与`Unix`之间含义不一致的路径片段。
fn portable_segment(segment: &str) -> bool {
    !segment.is_empty()
        && !segment.ends_with(['.', ' '])
        && !segment
            .chars()
            .any(|character| character.is_control() || r#"\/<>:"|?*"#.contains(character))
}
