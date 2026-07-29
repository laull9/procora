//! Procora 包的稳定清单模型。

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};

use crate::config::UploadKind;

/// 当前支持的 Procora 包格式。
pub const PACKAGE_FORMAT_V1: &str = "procora.package/v1";

/// 一个可独立验证和按平台物化的 Service 包清单。
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PackageManifest {
    /// 包格式及其主版本。
    pub format: String,
    /// 配置中的稳定 Service 名称。
    pub project: String,
    /// 包内 Procora 配置入口。
    pub config: PackageConfig,
    /// 不依赖运行平台的普通文件。
    pub files: Vec<PackageFile>,
    /// 逻辑二进制及其平台变体。
    pub binaries: BTreeMap<String, PackageBinary>,
    /// 可供 push 选择的包内导出项。
    pub exports: BTreeMap<String, PackageExport>,
}

/// 包内配置入口及其内容摘要。
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PackageConfig {
    /// 使用 `/` 分隔的 Service 相对路径。
    pub source: String,
    /// 配置文件 Blob 的 SHA-256。
    pub blob: String,
}

/// 清单映射到一个普通文件的 Blob。
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PackageFile {
    /// 物化到 Service 中的可移植相对路径。
    pub path: String,
    /// `sha256:<hex>` 内容地址。
    pub blob: String,
    /// 未压缩文件字节数。
    pub bytes: u64,
    /// 物化后是否赋予可执行权限。
    #[serde(default, skip_serializing_if = "is_false")]
    pub executable: bool,
}

/// 一个逻辑二进制的全部已打包变体。
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PackageBinary {
    /// 规范化平台键到具体产物的映射。
    pub variants: BTreeMap<String, PackageBinaryVariant>,
}

/// 一个已经写入包的具体平台二进制。
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PackageBinaryVariant {
    /// 物化到 Service 中的稳定相对路径。
    pub target: String,
    /// `sha256:<hex>` 内容地址。
    pub blob: String,
    /// 未压缩文件字节数。
    pub bytes: u64,
}

/// 从配置上传目标派生的包内导出项。
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PackageExport {
    /// 包内可被 push 物化的路径。
    pub path: String,
    /// 导出项要求的文件类型。
    pub kind: UploadKind,
}

impl PackageManifest {
    /// 校验清单结构、路径、摘要和平台选择边界。
    pub(crate) fn validate(&self) -> anyhow::Result<()> {
        if self.format != PACKAGE_FORMAT_V1 {
            bail!(
                "不支持 Procora 包格式 `{}`；当前支持 `{PACKAGE_FORMAT_V1}`",
                self.format
            );
        }
        let _: crate::core::ServiceName = self.project.parse()?;
        validate_portable_path(&self.config.source)?;
        validate_blob(&self.config.blob)?;

        let mut paths = BTreeSet::new();
        for file in &self.files {
            validate_portable_path(&file.path)?;
            validate_blob(&file.blob)?;
            if !paths.insert(portable_collision_key(&file.path)) {
                bail!("包内文件路径发生跨平台冲突：{}", file.path);
            }
        }
        let config = self
            .files
            .iter()
            .find(|file| file.path == self.config.source)
            .context("包清单的配置入口不在 files 中")?;
        if config.blob != self.config.blob {
            bail!("包清单的配置入口摘要与 files 不一致");
        }
        for (name, binary) in &self.binaries {
            if binary.variants.is_empty() {
                bail!("包内二进制 `{name}` 没有平台变体");
            }
            for (platform, variant) in &binary.variants {
                crate::config::DeployPlatform::validate_selector_key(platform)
                    .map_err(anyhow::Error::msg)?;
                validate_portable_path(&variant.target)?;
                validate_blob(&variant.blob)?;
            }
        }
        for (name, export) in &self.exports {
            if name.is_empty() {
                bail!("包内导出项名称不能为空");
            }
            validate_portable_path(&export.path)?;
        }
        Ok(())
    }

    /// 返回清单引用的全部 Blob 内容地址。
    pub(crate) fn referenced_blobs(&self) -> BTreeSet<&str> {
        self.files
            .iter()
            .map(|file| file.blob.as_str())
            .chain(self.binaries.values().flat_map(|binary| {
                binary
                    .variants
                    .values()
                    .map(|variant| variant.blob.as_str())
            }))
            .collect()
    }
}

/// 校验包内路径在 Unix 与 Windows 上都具有唯一普通文件语义。
pub(crate) fn validate_portable_path(path: &str) -> anyhow::Result<()> {
    if path.is_empty() || path.starts_with('/') || path.contains('\\') {
        bail!("包内路径必须是使用 `/` 分隔的非空相对路径：`{path}`");
    }
    for segment in path.split('/') {
        if segment.is_empty()
            || matches!(segment, "." | ".." | ".procora")
            || segment.ends_with(['.', ' '])
            || segment
                .chars()
                .any(|character| character.is_control() || r#"<>:"|?*"#.contains(character))
        {
            bail!("包内路径包含不可移植片段：`{path}`");
        }
        let base = segment.split('.').next().unwrap_or_default();
        if matches!(
            base.to_ascii_uppercase().as_str(),
            "CON"
                | "PRN"
                | "AUX"
                | "NUL"
                | "COM1"
                | "COM2"
                | "COM3"
                | "COM4"
                | "COM5"
                | "COM6"
                | "COM7"
                | "COM8"
                | "COM9"
                | "LPT1"
                | "LPT2"
                | "LPT3"
                | "LPT4"
                | "LPT5"
                | "LPT6"
                | "LPT7"
                | "LPT8"
                | "LPT9"
        ) {
            bail!("包内路径使用了 Windows 保留名称：`{path}`");
        }
    }
    Ok(())
}

/// 返回用于检测 Windows 大小写路径冲突的稳定键。
pub(crate) fn portable_collision_key(path: &str) -> String {
    path.to_ascii_lowercase()
}

/// 校验一个标准 SHA-256 内容地址。
pub(crate) fn validate_blob(blob: &str) -> anyhow::Result<()> {
    let Some(digest) = blob.strip_prefix("sha256:") else {
        bail!("包内 Blob 地址必须使用 `sha256:<hex>`");
    };
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        bail!("包内 Blob 地址不是完整 SHA-256：`{blob}`");
    }
    Ok(())
}

/// 判断布尔字段是否可以从稳定清单中省略。
#[allow(clippy::trivially_copy_pass_by_ref)]
const fn is_false(value: &bool) -> bool {
    !*value
}
