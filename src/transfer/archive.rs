use std::{
    collections::BTreeSet,
    fs,
    io::{self, Read},
    path::{Component, Path, PathBuf},
};

use anyhow::{Context, bail};
use flate2::{Compression, read::GzDecoder, write::GzEncoder};
use sha2::{Digest, Sha256};

use crate::config::{DeployBinaries, SelectedDeployBinary, UploadKind};

/// 已在本机完整生成、可供认证回退重复发送的归档。
pub(crate) struct PreparedArchive {
    path: PathBuf,
    pub(crate) kind: UploadKind,
    pub(crate) archive_bytes: u64,
    pub(crate) content_bytes: u64,
    pub(crate) sha256: String,
}

impl PreparedArchive {
    /// 打开压缩归档供 SSH 子进程读取。
    pub(crate) fn open(&self) -> io::Result<fs::File> {
        fs::File::open(&self.path)
    }
}

impl Drop for PreparedArchive {
    /// 尽力删除仅属于本次客户端调用的临时归档。
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// 把单文件或目录内容构造成拒绝符号链接的 gzip tar。
pub(crate) fn prepare(source: &Path) -> anyhow::Result<PreparedArchive> {
    let metadata = fs::symlink_metadata(source)
        .with_context(|| format!("无法读取上传来源 `{}`", source.display()))?;
    if metadata.file_type().is_symlink() {
        bail!("上传来源不能是符号链接：{}", source.display());
    }
    let kind = if metadata.is_file() {
        UploadKind::File
    } else if metadata.is_dir() {
        UploadKind::Directory
    } else {
        bail!("上传来源必须是普通文件或目录：{}", source.display());
    };
    let path = temporary_archive_path()?;
    let result = (|| {
        let file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        let encoder = GzEncoder::new(file, Compression::fast());
        let mut builder = tar::Builder::new(encoder);
        let content_bytes = match kind {
            UploadKind::File => {
                builder.append_path_with_name(source, "payload")?;
                metadata.len()
            }
            UploadKind::Directory => {
                append_directory(&mut builder, source, source, &BTreeSet::new())?
            }
        };
        let encoder = builder.into_inner()?;
        let file = encoder.finish()?;
        file.sync_all()?;
        let archive_bytes = file.metadata()?.len();
        drop(file);
        let sha256 = hash_file(&path)?;
        Ok(PreparedArchive {
            path: path.clone(),
            kind,
            archive_bytes,
            content_bytes,
            sha256,
        })
    })();
    if result.is_err() {
        let _ = fs::remove_file(path);
    }
    result
}

/// 构造只包含远端目标平台所选二进制的完整Service归档。
pub(crate) fn prepare_deploy(
    source: &Path,
    binaries: &DeployBinaries,
    selected: &[SelectedDeployBinary],
) -> anyhow::Result<PreparedArchive> {
    if binaries.is_empty() {
        return prepare(source);
    }
    let metadata = fs::symlink_metadata(source)
        .with_context(|| format!("无法读取部署来源 `{}`", source.display()))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        bail!("带二进制矩阵的部署来源必须是普通 Service 目录");
    }
    let exclusions = deploy_exclusions(source, binaries);
    let path = temporary_archive_path()?;
    let result = (|| {
        let file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        let encoder = GzEncoder::new(file, Compression::fast());
        let mut builder = tar::Builder::new(encoder);
        let mut content_bytes = append_directory(&mut builder, source, source, &exclusions)?;
        for binary in selected {
            content_bytes = content_bytes.saturating_add(append_binary(&mut builder, binary)?);
        }
        let encoder = builder.into_inner()?;
        let file = encoder.finish()?;
        file.sync_all()?;
        let archive_bytes = file.metadata()?.len();
        drop(file);
        let sha256 = hash_file(&path)?;
        Ok(PreparedArchive {
            path: path.clone(),
            kind: UploadKind::Directory,
            archive_bytes,
            content_bytes,
            sha256,
        })
    })();
    if result.is_err() {
        let _ = fs::remove_file(path);
    }
    result
}

/// 按名称稳定排序递归追加普通文件和目录。
fn append_directory(
    builder: &mut tar::Builder<GzEncoder<fs::File>>,
    root: &Path,
    directory: &Path,
    exclusions: &BTreeSet<PathBuf>,
) -> anyhow::Result<u64> {
    let mut entries = fs::read_dir(directory)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(fs::DirEntry::file_name);
    let mut total = 0_u64;
    for entry in entries {
        if entry.file_name() == ".procora" {
            continue;
        }
        let path = entry.path();
        let simplified = crate::platform::simplify_path(&path);
        if exclusions.contains(&simplified) {
            continue;
        }
        let relative = path.strip_prefix(root).expect("递归路径保持在来源根内");
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() {
            bail!("目录包含不支持的符号链接：{}", path.display());
        }
        if metadata.is_dir() {
            builder.append_dir(relative, &path)?;
            total = total.saturating_add(append_directory(builder, root, &path, exclusions)?);
        } else if metadata.is_file() {
            builder.append_path_with_name(&path, relative)?;
            total = total.saturating_add(metadata.len());
        } else {
            bail!("目录包含不支持的特殊文件：{}", path.display());
        }
    }
    Ok(total)
}

/// 返回全部平台源文件和稳定target，避免归档夹带其他平台产物或旧目标。
fn deploy_exclusions(source: &Path, binaries: &DeployBinaries) -> BTreeSet<PathBuf> {
    binaries
        .values()
        .flat_map(|binary| {
            let target = crate::platform::simplify_path(&source.join(&binary.target));
            std::iter::once(target)
                .chain(binary.variants.iter().filter_map(|variant| {
                    variant
                        .target
                        .as_ref()
                        .map(|target| crate::platform::simplify_path(&source.join(target)))
                }))
                .chain(
                    binary
                        .variants
                        .iter()
                        .map(|variant| crate::platform::simplify_path(&variant.source)),
                )
        })
        .collect()
}

/// 以固定可执行权限把一个选中本地产物写入稳定release路径。
fn append_binary(
    builder: &mut tar::Builder<GzEncoder<fs::File>>,
    binary: &SelectedDeployBinary,
) -> anyhow::Result<u64> {
    let metadata = fs::symlink_metadata(&binary.source).with_context(|| {
        format!(
            "二进制 `{}` 的选中产物不存在：{}",
            binary.name,
            binary.source.display()
        )
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        bail!(
            "二进制 `{}` 的选中产物必须是普通文件：{}",
            binary.name,
            binary.source.display()
        );
    }
    let mut header = tar::Header::new_gnu();
    header.set_size(metadata.len());
    header.set_mode(0o755);
    header.set_mtime(
        metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |duration| duration.as_secs()),
    );
    header.set_cksum();
    let mut file = fs::File::open(&binary.source)?;
    builder
        .append_data(&mut header, &binary.target, &mut file)
        .with_context(|| {
            format!(
                "无法把二进制 `{}` 写入 `{}`",
                binary.name,
                binary.target.display()
            )
        })?;
    Ok(metadata.len())
}

/// 安全展开归档并返回其中普通文件的总字节数。
pub(crate) fn unpack(
    archive_path: &Path,
    output: &Path,
    kind: UploadKind,
    max_bytes: u64,
) -> anyhow::Result<u64> {
    fs::create_dir(output)?;
    let decoder = GzDecoder::new(fs::File::open(archive_path)?);
    let mut archive = tar::Archive::new(decoder);
    let mut total = 0_u64;
    let mut entries = 0_u64;
    for entry in archive.entries()? {
        entries += 1;
        if entries > 100_000 {
            bail!("上传归档超过 100000 个条目上限");
        }
        let mut entry = entry?;
        let relative = entry.path()?.into_owned();
        if !safe_entry(&relative) {
            bail!("上传归档包含不安全路径 `{}`", relative.display());
        }
        if kind == UploadKind::File && relative != Path::new("payload") {
            bail!("单文件上传归档包含意外条目 `{}`", relative.display());
        }
        let target = output.join(&relative);
        let entry_kind = entry.header().entry_type();
        if entry_kind.is_dir() {
            fs::create_dir_all(&target)?;
        } else if entry_kind.is_file() {
            total = total.saturating_add(entry.header().size()?);
            if total > max_bytes {
                bail!("上传内容超过目标上限 {max_bytes} 字节");
            }
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            entry.unpack(&target)?;
        } else {
            bail!("上传归档包含不支持的条目 `{}`", relative.display());
        }
    }
    if kind == UploadKind::File && !output.join("payload").is_file() {
        bail!("单文件上传归档缺少 payload");
    }
    Ok(total)
}

/// 计算归档文件的 SHA-256。
pub(crate) fn hash_file(path: &Path) -> anyhow::Result<String> {
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
    Ok(format!("{:x}", digest.finalize()))
}

/// 生成排他创建的本机临时归档路径。
fn temporary_archive_path() -> anyhow::Result<PathBuf> {
    for _ in 0..16 {
        let candidate = crate::platform::temp_dir().join(format!(
            "procora-upload-{}-{}.tar.gz",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    bail!("无法分配上传临时文件名")
}

/// 只允许普通相对条目且永不接收 `.procora`。
fn safe_entry(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && path.components().all(|component| match component {
            Component::Normal(value) => value != ".procora",
            _ => false,
        })
}
