//! Procora 包清单读取与完整内容验证。

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    path::Path,
};

use anyhow::{Context, bail};
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::manifest::PackageManifest;

const MAX_MANIFEST_BYTES: u64 = 4 * 1024 * 1024;
const MAX_PACKAGE_ENTRIES: u64 = 100_000;

/// 已读取并完成结构校验的包信息。
#[derive(Clone, Debug, Serialize)]
pub struct PackageInfo {
    /// 稳定包清单。
    pub manifest: PackageManifest,
    /// 清单内容摘要，即逻辑包身份。
    pub package_digest: String,
    /// 外层 `.pcpkg` 文件大小。
    pub package_bytes: u64,
}

/// 只读取首个清单并完成结构校验。
///
/// # Errors
///
/// 当文件不是有效 `.pcpkg`、清单过大或结构不兼容时返回错误。
pub fn inspect(path: &Path) -> anyhow::Result<PackageInfo> {
    let package_bytes = fs::metadata(path)
        .with_context(|| format!("无法读取 Procora 包：{}", path.display()))?
        .len();
    let file = fs::File::open(path)?;
    let decoder = zstd::Decoder::new(file).context("Procora 包不是有效 zstd 流")?;
    let mut archive = tar::Archive::new(decoder);
    let mut entries = archive.entries()?;
    let mut entry = entries.next().context("Procora 包为空")??;
    let path = entry.path()?.into_owned();
    if path != Path::new("manifest.json") || !entry.header().entry_type().is_file() {
        bail!("Procora 包第一个条目必须是普通文件 manifest.json");
    }
    let size = entry.header().size()?;
    if size == 0 || size > MAX_MANIFEST_BYTES {
        bail!("Procora 包清单大小必须在 1..={MAX_MANIFEST_BYTES} 字节内");
    }
    let mut bytes = Vec::with_capacity(usize::try_from(size).context("包清单无法装入内存")?);
    entry.read_to_end(&mut bytes)?;
    let manifest: PackageManifest =
        serde_json::from_slice(&bytes).context("Procora 包清单不是有效 JSON")?;
    manifest.validate()?;
    Ok(PackageInfo {
        manifest,
        package_digest: format!("sha256:{:x}", Sha256::digest(&bytes)),
        package_bytes,
    })
}

/// 流式验证清单引用的全部 Blob，无需展开到文件系统。
///
/// # Errors
///
/// 当条目、大小或任一 Blob 的 SHA-256 与清单不一致时返回错误。
pub fn verify(path: &Path) -> anyhow::Result<PackageInfo> {
    let info = inspect(path)?;
    let expected = expected_blobs(&info.manifest)?;
    let file = fs::File::open(path)?;
    let decoder = zstd::Decoder::new(file)?;
    let mut archive = tar::Archive::new(decoder);
    let mut seen = BTreeSet::new();
    let mut entries = 0_u64;
    for entry in archive.entries()? {
        entries += 1;
        if entries > MAX_PACKAGE_ENTRIES {
            bail!("Procora 包超过 {MAX_PACKAGE_ENTRIES} 个条目");
        }
        let mut entry = entry?;
        if !entry.header().entry_type().is_file() {
            bail!("Procora 包只允许普通文件条目");
        }
        let path = entry.path()?.into_owned();
        if path == Path::new("manifest.json") {
            if entries != 1 {
                bail!("Procora 包包含重复 manifest.json");
            }
            continue;
        }
        let path_text = path.to_str().context("Procora 包容器路径必须使用 UTF-8")?;
        if path_text.starts_with("signatures/") {
            if entry.header().size()? > 1024 * 1024 {
                bail!("Procora 包单个签名条目不能超过 1 MiB");
            }
            continue;
        }
        let blob = blob_from_archive_path(path_text)?;
        let expected_bytes = expected
            .get(&blob)
            .with_context(|| format!("Procora 包包含清单未引用的 Blob `{blob}`"))?;
        if !seen.insert(blob.clone()) {
            bail!("Procora 包包含重复 Blob `{blob}`");
        }
        if entry.header().size()? != *expected_bytes {
            bail!("Procora 包 Blob `{blob}` 的大小与清单不一致");
        }
        let actual = hash_reader(&mut entry)?;
        if actual != blob {
            bail!("Procora 包 Blob `{blob}` 内容摘要不匹配，实际 `{actual}`");
        }
    }
    let missing = info
        .manifest
        .referenced_blobs()
        .into_iter()
        .filter(|blob| !seen.contains(*blob))
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        bail!("Procora 包缺少清单引用的 Blob：{}", missing.join("、"));
    }
    Ok(info)
}

/// 汇总 Blob 的唯一预期长度并拒绝同摘要长度矛盾。
fn expected_blobs(manifest: &PackageManifest) -> anyhow::Result<BTreeMap<String, u64>> {
    let mut expected = BTreeMap::new();
    for (blob, bytes) in manifest
        .files
        .iter()
        .map(|file| (&file.blob, file.bytes))
        .chain(manifest.binaries.values().flat_map(|binary| {
            binary
                .variants
                .values()
                .map(|variant| (&variant.blob, variant.bytes))
        }))
    {
        if let Some(existing) = expected.insert(blob.clone(), bytes)
            && existing != bytes
        {
            bail!("Procora 包 Blob `{blob}` 在清单中声明了不同长度");
        }
    }
    Ok(expected)
}

/// 从稳定容器路径还原 Blob 内容地址。
pub(super) fn blob_from_archive_path(path: &str) -> anyhow::Result<String> {
    let Some(rest) = path.strip_prefix("blobs/sha256/") else {
        bail!("Procora 包包含未知条目 `{path}`");
    };
    let Some((prefix, suffix)) = rest.split_once('/') else {
        bail!("Procora 包 Blob 路径无效：`{path}`");
    };
    if prefix.len() != 2
        || suffix.len() != 62
        || !prefix
            .bytes()
            .chain(suffix.bytes())
            .all(|byte| byte.is_ascii_hexdigit())
    {
        bail!("Procora 包 Blob 路径无效：`{path}`");
    }
    Ok(format!(
        "sha256:{}{}",
        prefix.to_ascii_lowercase(),
        suffix.to_ascii_lowercase()
    ))
}

/// 流式计算一个归档条目的内容地址。
fn hash_reader(reader: &mut impl Read) -> anyhow::Result<String> {
    let mut digest = Sha256::new();
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(format!("sha256:{:x}", digest.finalize()))
}
