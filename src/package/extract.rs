//! Procora 包的平台选择与安全物化。

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

use anyhow::{Context, bail};
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::{
    manifest::{PackageBinaryVariant, PackageManifest, portable_collision_key},
    read::{blob_from_archive_path, verify},
};

/// 成功物化的平台 release 摘要。
#[derive(Clone, Debug, Serialize)]
pub struct PackageExtractResult {
    /// 实际选择的平台。
    pub platform: crate::config::DeployPlatform,
    /// 由物化文件映射计算的稳定 release 身份。
    pub release_digest: String,
    /// 写入的普通文件数量。
    pub files: usize,
    /// 写入的未压缩总字节数。
    pub content_bytes: u64,
}

/// 验证包并把适用于目标平台的完整 Service 物化到新目录。
///
/// # Errors
///
/// 当包校验失败、平台不受支持、路径冲突或目标已经存在时返回错误。
pub fn extract(
    package: &Path,
    output: &Path,
    platform: crate::config::DeployPlatform,
) -> anyhow::Result<PackageExtractResult> {
    let platform = platform.normalized().map_err(anyhow::Error::msg)?;
    let info = verify(package)?;
    if output.exists() {
        bail!("包解压目标已存在，拒绝覆盖：{}", output.display());
    }
    let plan = materialization_plan(&info.manifest, &platform)?;
    let result = (|| {
        fs::create_dir(output)?;
        materialize(package, output, &plan)?;
        let release_digest = release_digest(&plan)?;
        Ok(PackageExtractResult {
            platform,
            release_digest,
            files: plan.values().map(Vec::len).sum(),
            content_bytes: plan
                .values()
                .flat_map(|files| files.iter())
                .map(|file| file.bytes)
                .sum(),
        })
    })();
    if result.is_err() && output.exists() {
        let _ = fs::remove_dir_all(output);
    }
    result
}

/// 一个 Blob 需要物化到的目标文件。
#[derive(Clone, Debug, Serialize)]
struct MaterializedFile {
    path: String,
    blob: String,
    bytes: u64,
    executable: bool,
}

/// 生成公共文件和当前平台二进制的无冲突物化计划。
fn materialization_plan(
    manifest: &PackageManifest,
    platform: &crate::config::DeployPlatform,
) -> anyhow::Result<BTreeMap<String, Vec<MaterializedFile>>> {
    let mut plan = BTreeMap::<String, Vec<MaterializedFile>>::new();
    let mut paths = BTreeSet::new();
    for file in &manifest.files {
        add_file(
            &mut plan,
            &mut paths,
            MaterializedFile {
                path: file.path.clone(),
                blob: file.blob.clone(),
                bytes: file.bytes,
                executable: file.executable,
            },
        )?;
    }
    for (name, binary) in &manifest.binaries {
        let variant = select_variant(name, &binary.variants, platform)?;
        add_file(
            &mut plan,
            &mut paths,
            MaterializedFile {
                path: variant.target.clone(),
                blob: variant.blob.clone(),
                bytes: variant.bytes,
                executable: true,
            },
        )?;
    }
    Ok(plan)
}

/// 插入物化文件并拒绝跨平台路径冲突。
fn add_file(
    plan: &mut BTreeMap<String, Vec<MaterializedFile>>,
    paths: &mut BTreeSet<String>,
    file: MaterializedFile,
) -> anyhow::Result<()> {
    if !paths.insert(portable_collision_key(&file.path)) {
        bail!("包物化路径发生冲突：{}", file.path);
    }
    plan.entry(file.blob.clone()).or_default().push(file);
    Ok(())
}

/// 按现有 binaries 规则选择最高优先级平台变体。
fn select_variant<'a>(
    name: &str,
    variants: &'a BTreeMap<String, PackageBinaryVariant>,
    platform: &crate::config::DeployPlatform,
) -> anyhow::Result<&'a PackageBinaryVariant> {
    let mut selected = None;
    for (selector, variant) in variants {
        let Some(specificity) = platform
            .selector_specificity(selector)
            .map_err(anyhow::Error::msg)?
        else {
            continue;
        };
        if selected.is_none_or(|(best, _)| specificity > best) {
            selected = Some((specificity, variant));
        }
    }
    selected.map(|(_, variant)| variant).with_context(|| {
        format!(
            "包内二进制 `{name}` 不支持平台 `{}`；包含：{}",
            platform.key(),
            variants.keys().cloned().collect::<Vec<_>>().join("、")
        )
    })
}

/// 逐 Blob 写入首个目标并复制到内容相同的其他目标。
fn materialize(
    package: &Path,
    output: &Path,
    plan: &BTreeMap<String, Vec<MaterializedFile>>,
) -> anyhow::Result<()> {
    let file = fs::File::open(package)?;
    let decoder = zstd::Decoder::new(file)?;
    let mut archive = tar::Archive::new(decoder);
    let mut written = BTreeSet::new();
    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        let path_text = path.to_str().context("Procora 包容器路径必须使用 UTF-8")?;
        if path_text == "manifest.json" || path_text.starts_with("signatures/") {
            continue;
        }
        let blob = blob_from_archive_path(path_text)?;
        let Some(destinations) = plan.get(&blob) else {
            continue;
        };
        let first = destinations.first().expect("物化计划不会保存空目标");
        let first_path = output_path(output, &first.path);
        if let Some(parent) = first_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut target = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&first_path)?;
        let mut digest = Sha256::new();
        let mut bytes = 0_u64;
        let mut buffer = vec![0_u8; 64 * 1024];
        loop {
            let read = entry.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            target.write_all(&buffer[..read])?;
            digest.update(&buffer[..read]);
            bytes = bytes.saturating_add(read as u64);
        }
        target.sync_all()?;
        let actual = format!("sha256:{:x}", digest.finalize());
        if bytes != first.bytes || actual != blob {
            bail!("物化 Blob `{blob}` 时内容发生变化");
        }
        set_executable(&first_path, first.executable)?;
        for destination in &destinations[1..] {
            let path = output_path(output, &destination.path);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(&first_path, &path)?;
            set_executable(&path, destination.executable)?;
        }
        written.insert(blob);
    }
    let missing = plan
        .keys()
        .filter(|blob| !written.contains(*blob))
        .cloned()
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        bail!("包物化时缺少 Blob：{}", missing.join("、"));
    }
    Ok(())
}

/// 从 `/` 分隔清单路径构造本机路径。
fn output_path(output: &Path, portable: &str) -> PathBuf {
    portable
        .split('/')
        .fold(output.to_path_buf(), |path, segment| path.join(segment))
}

/// 计算不受外层包压缩和其他平台内容影响的 release 身份。
fn release_digest(plan: &BTreeMap<String, Vec<MaterializedFile>>) -> anyhow::Result<String> {
    let mut files = plan
        .values()
        .flat_map(|files| files.iter().cloned())
        .collect::<Vec<_>>();
    files.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(format!(
        "sha256:{:x}",
        Sha256::digest(serde_json::to_vec(&files)?)
    ))
}

/// 在 Unix 上应用受限的普通或可执行文件模式。
#[cfg(unix)]
fn set_executable(path: &Path, executable: bool) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(
        path,
        fs::Permissions::from_mode(if executable { 0o755 } else { 0o644 }),
    )?;
    Ok(())
}

/// Windows 文件可执行性由扩展名和调用方式决定。
#[cfg(not(unix))]
fn set_executable(_: &Path, _: bool) -> anyhow::Result<()> {
    Ok(())
}
