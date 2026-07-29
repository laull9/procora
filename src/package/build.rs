//! Procora 包的确定性构建器。

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
};

use anyhow::{Context, bail};
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use sha2::{Digest, Sha256};

use super::manifest::{
    PACKAGE_FORMAT_V1, PackageBinary, PackageBinaryVariant, PackageConfig, PackageExport,
    PackageFile, PackageManifest, validate_portable_path,
};

/// 包构建时包含的平台范围。
#[derive(Clone, Debug)]
pub enum PackagePlatform {
    /// 包含配置声明的全部平台变体。
    All,
    /// 只包含适用于指定平台的唯一变体。
    Target(crate::config::DeployPlatform),
}

/// 成功构建的包身份与规模摘要。
#[derive(Clone, Debug)]
pub struct PackageBuildResult {
    /// 输出包路径。
    pub path: PathBuf,
    /// 配置中的稳定 Service 名称。
    pub project: String,
    /// 清单内容摘要。
    pub package_digest: String,
    /// 包文件字节数。
    pub package_bytes: u64,
    /// 清单中的普通文件数量。
    pub files: usize,
    /// 打入的二进制变体数量。
    pub binary_variants: usize,
}

/// 构建一个内容寻址的确定性 `.pcpkg`。
///
/// # Errors
///
/// 当配置、普通文件、平台变体、忽略规则或输出路径无效时返回错误。
pub fn build(
    source: &Path,
    output: &Path,
    platform: PackagePlatform,
) -> anyhow::Result<PackageBuildResult> {
    let discovered = crate::config::discover_path(source)
        .with_context(|| format!("无法发现待打包 Service：{}", source.display()))?;
    let output = absolute_output(output)?;
    if output.exists() {
        bail!("Procora 包输出已存在，拒绝覆盖：{}", output.display());
    }
    let ignores = PackageIgnore::load(&discovered.root)?;
    let exclusions = exclusions(&discovered, &output);
    let mut blobs = BTreeMap::<String, PathBuf>::new();
    let mut files = Vec::new();
    collect_files(
        &discovered.root,
        &discovered.root,
        &exclusions,
        &ignores,
        &mut blobs,
        &mut files,
    )?;
    files.sort_by(|left, right| left.path.cmp(&right.path));
    let config_path = portable_relative(&discovered.root, &discovered.config_path)?;
    let config_blob = files
        .iter()
        .find(|file| file.path == config_path)
        .map(|file| file.blob.clone())
        .context("Procora 配置入口被排除在包内容之外")?;
    let binaries = collect_binaries(&discovered.compiled.deploy_binaries, platform, &mut blobs)?;
    let exports = discovered
        .compiled
        .upload_targets
        .iter()
        .map(|(name, export)| {
            Ok((
                name.clone(),
                PackageExport {
                    path: portable_path(&export.path)?,
                    kind: export.kind,
                },
            ))
        })
        .collect::<anyhow::Result<BTreeMap<_, _>>>()?;
    let manifest = PackageManifest {
        format: PACKAGE_FORMAT_V1.to_owned(),
        project: discovered.compiled.spec.project,
        config: PackageConfig {
            source: config_path,
            blob: config_blob,
        },
        files,
        binaries,
        exports,
    };
    manifest.validate()?;
    let manifest_bytes = serde_json::to_vec(&manifest)?;
    let package_digest = format!("sha256:{:x}", Sha256::digest(&manifest_bytes));
    write_package(&output, &manifest_bytes, &blobs)?;
    if let Err(error) = super::read::verify(&output) {
        let _ = fs::remove_file(&output);
        return Err(error).context("构建后的 Procora 包自校验失败");
    }
    let package_bytes = fs::metadata(&output)?.len();
    let binary_variants = manifest
        .binaries
        .values()
        .map(|binary| binary.variants.len())
        .sum();
    Ok(PackageBuildResult {
        path: output,
        project: manifest.project,
        package_digest,
        package_bytes,
        files: manifest.files.len(),
        binary_variants,
    })
}

/// 构建包，并在调用者明确要求时以可恢复备份替换已有普通文件。
///
/// # Errors
///
/// 当既有目标不是普通文件、备份/恢复失败或正常构建失败时返回错误。
pub fn build_replacing(
    source: &Path,
    output: &Path,
    platform: PackagePlatform,
) -> anyhow::Result<PackageBuildResult> {
    let output = absolute_output(output)?;
    if !output.exists() {
        return build(source, &output, platform);
    }
    let metadata = fs::symlink_metadata(&output)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        bail!("只能替换已有普通包文件：{}", output.display());
    }
    let output = crate::platform::canonicalize(&output)?;
    let discovered = crate::config::discover_path(source)
        .with_context(|| format!("无法发现待打包 Service：{}", source.display()))?;
    let parent = output.parent().context("包输出没有父目录")?;
    let backup_directory = if output.starts_with(&discovered.root) {
        discovered.root.join(".procora/package-backups")
    } else {
        parent.to_path_buf()
    };
    fs::create_dir_all(&backup_directory)?;
    let backup = backup_directory.join(format!(".pcpkg-backup-{}", uuid::Uuid::new_v4()));
    fs::rename(&output, &backup)
        .with_context(|| format!("无法为已有包创建可恢复备份：{}", output.display()))?;
    match build(source, &output, platform) {
        Ok(result) => {
            fs::remove_file(&backup)
                .with_context(|| format!("新包已构建，但无法清理备份 `{}`", backup.display()))?;
            Ok(result)
        }
        Err(build_error) => {
            if output.exists() {
                let _ = fs::remove_file(&output);
            }
            fs::rename(&backup, &output).with_context(|| {
                format!(
                    "包构建失败（{build_error:#}），且无法恢复原文件 `{}`",
                    output.display()
                )
            })?;
            Err(build_error)
        }
    }
}

/// 返回构建时不能作为普通 Service 文件重复收集的路径。
fn exclusions(discovered: &crate::config::DiscoveredProject, output: &Path) -> BTreeSet<PathBuf> {
    let mut paths = BTreeSet::from([crate::platform::simplify_path(output)]);
    for binary in discovered.compiled.deploy_binaries.values() {
        paths.insert(crate::platform::simplify_path(
            &discovered.root.join(&binary.target),
        ));
        for variant in &binary.variants {
            paths.insert(crate::platform::simplify_path(&variant.source));
            if let Some(target) = &variant.target {
                paths.insert(crate::platform::simplify_path(
                    &discovered.root.join(target),
                ));
            }
        }
    }
    paths
}

/// 稳定递归收集普通 Service 文件。
fn collect_files(
    root: &Path,
    directory: &Path,
    exclusions: &BTreeSet<PathBuf>,
    ignores: &PackageIgnore,
    blobs: &mut BTreeMap<String, PathBuf>,
    files: &mut Vec<PackageFile>,
) -> anyhow::Result<()> {
    let mut entries = fs::read_dir(directory)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(fs::DirEntry::file_name);
    for entry in entries {
        if matches!(
            entry.file_name().to_str(),
            Some(".procora" | ".git" | ".procoraignore")
        ) {
            continue;
        }
        let path = crate::platform::simplify_path(&entry.path());
        if exclusions.contains(&path) {
            continue;
        }
        let metadata = fs::symlink_metadata(&path)?;
        if ignores.matches(&path, metadata.is_dir()) {
            continue;
        }
        if metadata.file_type().is_symlink() {
            bail!("Service 包不支持符号链接：{}", path.display());
        }
        if metadata.is_dir() {
            collect_files(root, &path, exclusions, ignores, blobs, files)?;
            continue;
        }
        if !metadata.is_file() {
            bail!("Service 包不支持特殊文件：{}", path.display());
        }
        let relative = portable_relative(root, &path)?;
        let blob = hash_file(&path)?;
        blobs.entry(blob.clone()).or_insert_with(|| path.clone());
        files.push(PackageFile {
            path: relative,
            blob,
            bytes: metadata.len(),
            executable: is_executable(&metadata),
        });
    }
    Ok(())
}

/// 以 gitignore 语义加载可选 `.procoraignore`。
struct PackageIgnore {
    matcher: Option<Gitignore>,
}

impl PackageIgnore {
    /// 读取 Service 根目录的忽略规则，不自动继承 `.gitignore`。
    fn load(root: &Path) -> anyhow::Result<Self> {
        let path = root.join(".procoraignore");
        if !path.is_file() {
            return Ok(Self { matcher: None });
        }
        let mut builder = GitignoreBuilder::new(root);
        if let Some(error) = builder.add(&path) {
            return Err(error).context("无法解析 .procoraignore");
        }
        Ok(Self {
            matcher: Some(builder.build().context("无法构建 .procoraignore 匹配器")?),
        })
    }

    /// 判断一个普通文件或目录是否应从包内容排除。
    fn matches(&self, path: &Path, is_dir: bool) -> bool {
        self.matcher.as_ref().is_some_and(|matcher| {
            matcher
                .matched_path_or_any_parents(path, is_dir)
                .is_ignore()
        })
    }
}

/// 把 API 调用者给出的输出路径固定为普通绝对路径。
fn absolute_output(output: &Path) -> anyhow::Result<PathBuf> {
    if output.is_absolute() {
        return Ok(crate::platform::simplify_path(output));
    }
    Ok(crate::platform::current_dir()
        .context("无法读取当前目录")?
        .join(output))
}

/// 收集全部变体或指定平台的唯一变体。
fn collect_binaries(
    binaries: &crate::config::DeployBinaries,
    platform: PackagePlatform,
    blobs: &mut BTreeMap<String, PathBuf>,
) -> anyhow::Result<BTreeMap<String, PackageBinary>> {
    let mut output = binaries
        .keys()
        .map(|name| {
            (
                name.clone(),
                PackageBinary {
                    variants: BTreeMap::new(),
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    match platform {
        PackagePlatform::All => {
            for (name, binary) in binaries {
                for variant in &binary.variants {
                    let target = variant.target.as_ref().unwrap_or(&binary.target);
                    add_binary(
                        output.get_mut(name).expect("二进制名称来自同一集合"),
                        variant.selector.key(),
                        &variant.source,
                        target,
                        blobs,
                    )?;
                }
            }
        }
        PackagePlatform::Target(platform) => {
            for selected in crate::config::select_deploy_binaries(binaries, &platform)
                .map_err(anyhow::Error::msg)?
            {
                add_binary(
                    output
                        .get_mut(&selected.name)
                        .expect("二进制名称来自同一集合"),
                    selected.selector,
                    &selected.source,
                    &selected.target,
                    blobs,
                )?;
            }
        }
    }
    Ok(output)
}

/// 校验并加入一个平台二进制 Blob。
fn add_binary(
    binary: &mut PackageBinary,
    platform: String,
    source: &Path,
    target: &Path,
    blobs: &mut BTreeMap<String, PathBuf>,
) -> anyhow::Result<()> {
    let metadata = fs::symlink_metadata(source)
        .with_context(|| format!("无法读取平台 `{platform}` 的二进制：{}", source.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() == 0 {
        bail!("平台 `{platform}` 的二进制必须是非空普通文件");
    }
    let blob = hash_file(source)?;
    blobs
        .entry(blob.clone())
        .or_insert_with(|| source.to_path_buf());
    binary.variants.insert(
        platform,
        PackageBinaryVariant {
            target: portable_path(target)?,
            blob,
            bytes: metadata.len(),
        },
    );
    Ok(())
}

/// 写入清单优先、条目稳定排序的 zstd tar。
fn write_package(
    output: &Path,
    manifest: &[u8],
    blobs: &BTreeMap<String, PathBuf>,
) -> anyhow::Result<()> {
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    let file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)?;
    let result = (|| {
        let encoder = zstd::Encoder::new(file, 3)?;
        let mut archive = tar::Builder::new(encoder);
        append_bytes(&mut archive, "manifest.json", manifest, 0o644)?;
        for (blob, source) in blobs {
            let digest = blob.strip_prefix("sha256:").expect("构建器只生成 SHA-256");
            let archive_path = format!("blobs/sha256/{}/{}", &digest[..2], &digest[2..]);
            append_file(&mut archive, &archive_path, source)?;
        }
        let encoder = archive.into_inner()?;
        let file = encoder.finish()?;
        file.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(output);
    }
    result
}

/// 写入一个规范化普通文件条目。
fn append_file(
    archive: &mut tar::Builder<zstd::Encoder<'static, fs::File>>,
    path: &str,
    source: &Path,
) -> anyhow::Result<()> {
    let metadata = fs::metadata(source)?;
    let mut header = deterministic_header(metadata.len(), 0o644);
    let mut file = fs::File::open(source)?;
    archive.append_data(&mut header, path, &mut file)?;
    Ok(())
}

/// 写入一个内存中的规范化普通文件条目。
fn append_bytes(
    archive: &mut tar::Builder<zstd::Encoder<'static, fs::File>>,
    path: &str,
    bytes: &[u8],
    mode: u32,
) -> anyhow::Result<()> {
    let mut header = deterministic_header(bytes.len() as u64, mode);
    archive.append_data(&mut header, path, bytes)?;
    Ok(())
}

/// 创建不携带宿主身份和时间的 tar 头。
fn deterministic_header(size: u64, mode: u32) -> tar::Header {
    let mut header = tar::Header::new_gnu();
    header.set_size(size);
    header.set_mode(mode);
    header.set_uid(0);
    header.set_gid(0);
    header.set_mtime(0);
    header.set_cksum();
    header
}

/// 计算文件内容地址。
fn hash_file(path: &Path) -> anyhow::Result<String> {
    let mut file = fs::File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(format!("sha256:{:x}", digest.finalize()))
}

/// 把已有路径编码为 Service 根目录下的可移植路径。
fn portable_relative(root: &Path, path: &Path) -> anyhow::Result<String> {
    let relative = path
        .strip_prefix(root)
        .with_context(|| format!("包内容不在 Service 根目录内：{}", path.display()))?;
    portable_path(relative)
}

/// 把普通相对 Path 转换为 `/` 分隔文本。
fn portable_path(path: &Path) -> anyhow::Result<String> {
    let mut parts = Vec::new();
    for component in path.components() {
        let Component::Normal(part) = component else {
            bail!("包路径必须是普通相对路径：{}", path.display());
        };
        parts.push(part.to_str().context("包路径必须使用 UTF-8")?.to_owned());
    }
    let path = parts.join("/");
    validate_portable_path(&path)?;
    Ok(path)
}

/// 判断 Unix 普通文件是否声明任一可执行位。
#[cfg(unix)]
fn is_executable(metadata: &fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;

    metadata.permissions().mode() & 0o111 != 0
}

/// Windows 普通文件的可执行语义由 binaries 清单决定。
#[cfg(not(unix))]
const fn is_executable(_: &fs::Metadata) -> bool {
    false
}
