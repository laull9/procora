use std::{
    collections::BTreeMap,
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use crate::config::{CompiledProject, DependencyKind, ManagedDependencySpec};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use super::{archive, download, placeholders, verify};

/// 依赖下载、解包、缓存或版本验证错误。
#[derive(Debug, Error)]
pub enum SourceError {
    /// 文件系统操作失败。
    #[error(transparent)]
    Io(#[from] io::Error),
    /// ZIP 归档无法读取。
    #[error(transparent)]
    Zip(#[from] zip::result::ZipError),
    /// 本地版本清单无法读写。
    #[error(transparent)]
    Manifest(#[from] serde_json::Error),
    /// 远端或本地来源下载失败。
    #[error("下载 `{location}` 失败：{message}")]
    Download {
        /// 配置中的来源。
        location: String,
        /// 底层下载器消息。
        message: String,
    },
    /// 归档包含可能逃逸安装根目录的条目。
    #[error("归档包含不安全路径 `{0}`")]
    UnsafeArchive(String),
    /// 下载内容与声明摘要不一致。
    #[error("依赖 `{name}` 的 SHA-256 不匹配：期望 {expected}，实际 {actual}")]
    Checksum {
        /// 依赖名称。
        name: String,
        /// 声明摘要。
        expected: String,
        /// 实际摘要。
        actual: String,
    },
    /// 已安装文件或目录与版本清单中的内容指纹不一致。
    #[error("依赖 `{name}` 的已安装内容已变化")]
    Integrity {
        /// 发生变化的依赖名称。
        name: String,
    },
    /// 无法确定或访问最终管理路径。
    #[error("依赖管理路径无效：{0}")]
    ManagedPath(String),
    /// 真实版本命令失败或输出不匹配。
    #[error("版本验证命令 `{}` 失败：{message}", command.display())]
    Verify {
        /// 实际执行的程序。
        command: PathBuf,
        /// 失败原因。
        message: String,
    },
}

/// 已安装并验证的单个项目依赖。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedDependency {
    /// 配置中的依赖名称。
    pub name: String,
    /// 声明并验证的版本。
    pub version: String,
    /// 供任务占位符使用的绝对路径。
    pub path: PathBuf,
    /// 本次是否发生了下载和安装。
    pub installed: bool,
}

/// 服务目录内的依赖下载与版本管理器。
#[derive(Clone, Debug)]
pub struct DependencyManager {
    service_root: PathBuf,
}

/// 安装目录内用于离线复核的版本清单。
#[derive(Debug, Deserialize, Serialize)]
struct InstallManifest {
    name: String,
    source: String,
    #[serde(default)]
    mirrors: Vec<String>,
    version: String,
    sha256: String,
    managed_path: PathBuf,
    managed_sha256: String,
}

/// 为同一进程内的并发安装生成不同临时目录后缀。
static NEXT_STAGING_ID: AtomicU64 = AtomicU64::new(1);

impl DependencyManager {
    /// 创建绑定到单个服务目录的依赖管理器。
    pub fn new(service_root: impl Into<PathBuf>) -> Self {
        Self {
            service_root: service_root.into(),
        }
    }

    /// 同步全部声明依赖并替换任务中的依赖路径占位符。
    ///
    /// # Errors
    ///
    /// 当下载、摘要、解包、路径选择或版本命令失败时返回错误。
    pub fn prepare(
        &self,
        compiled: &mut CompiledProject,
    ) -> Result<Vec<ResolvedDependency>, SourceError> {
        let resolved = self.sync(&compiled.dependencies)?;
        placeholders::apply(compiled, &resolved);
        Ok(resolved)
    }

    /// 下载缺失或无效的依赖，并复核全部版本清单和版本命令。
    ///
    /// # Errors
    ///
    /// 当任一依赖无法得到有效安装时返回错误。
    pub fn sync(
        &self,
        dependencies: &BTreeMap<String, ManagedDependencySpec>,
    ) -> Result<Vec<ResolvedDependency>, SourceError> {
        dependencies
            .iter()
            .map(|(name, spec)| self.sync_one(name, spec))
            .collect()
    }

    /// 仅离线复核现有依赖，不触发下载。
    ///
    /// # Errors
    ///
    /// 当缓存缺失、清单不匹配或版本命令失败时返回错误。
    pub fn check(
        &self,
        dependencies: &BTreeMap<String, ManagedDependencySpec>,
    ) -> Result<Vec<ResolvedDependency>, SourceError> {
        dependencies
            .iter()
            .map(|(name, spec)| self.check_one(name, spec, false))
            .collect()
    }

    /// 同步单个依赖，优先使用已验证缓存。
    fn sync_one(
        &self,
        name: &str,
        spec: &ManagedDependencySpec,
    ) -> Result<ResolvedDependency, SourceError> {
        if let Ok(resolved) = self.check_one(name, spec, false) {
            return Ok(resolved);
        }
        let _install_lock = self.acquire_install_lock(name)?;
        if let Ok(resolved) = self.check_one(name, spec, false) {
            return Ok(resolved);
        }
        let install_root = self.install_root(name, &spec.version);
        let parent = install_root.parent().expect("安装路径包含父目录");
        fs::create_dir_all(parent)?;
        let staging_id = NEXT_STAGING_ID.fetch_add(1, Ordering::Relaxed);
        let staging = parent.join(format!(".tmp-{}-{staging_id}-{name}", std::process::id()));
        if staging.exists() {
            fs::remove_dir_all(&staging)?;
        }
        fs::create_dir_all(&staging)?;
        let result = self.install_into(name, spec, &staging);
        if result.is_err() {
            let _ = fs::remove_dir_all(&staging);
            return result;
        }
        Self::activate_install(name, &staging, &install_root)?;
        let mut resolved = self.check_one(name, spec, true)?;
        resolved.installed = true;
        Ok(resolved)
    }

    /// 在临时目录完成下载、解包、路径选择和清单写入。
    fn install_into(
        &self,
        name: &str,
        spec: &ManagedDependencySpec,
        staging: &Path,
    ) -> Result<ResolvedDependency, SourceError> {
        let filename = download::source_filename(&spec.source, name);
        let downloaded = staging.join(".download");
        let sources = std::iter::once(&spec.source)
            .chain(&spec.mirrors)
            .map(|source| resolve_local_source(&self.service_root, source))
            .collect::<Vec<_>>();
        let mut ssh = spec.ssh.clone();
        ssh.identity_file = ssh
            .identity_file
            .map(|path| resolve_service_path(&self.service_root, path));
        ssh.known_hosts_file = ssh
            .known_hosts_file
            .map(|path| resolve_service_path(&self.service_root, path));
        let actual_checksum = download::fetch(&sources, &downloaded, &spec.download, &ssh)?.sha256;
        if let Some(expected) = spec.checksum.as_deref() {
            let expected = expected.strip_prefix("sha256:").unwrap_or(expected);
            if !actual_checksum.eq_ignore_ascii_case(expected) {
                return Err(SourceError::Checksum {
                    name: name.to_owned(),
                    expected: expected.to_owned(),
                    actual: actual_checksum,
                });
            }
        }
        let content = archive::materialize(
            &downloaded,
            &filename,
            &staging.join("content"),
            spec.unpack,
        )?;
        fs::remove_file(downloaded)?;
        let managed = select_managed_path(name, spec, &content)?;
        verify::ensure_kind(&managed, spec.kind)?;
        let relative = managed
            .strip_prefix(staging)
            .map_err(|_| SourceError::ManagedPath(managed.display().to_string()))?
            .to_path_buf();
        let managed_sha256 = fingerprint(&managed)?;
        verify::run_version_check(staging, &managed, &spec.version, spec.verify.as_ref())?;
        let manifest = InstallManifest {
            name: name.to_owned(),
            source: spec.source.clone(),
            mirrors: spec.mirrors.clone(),
            version: spec.version.clone(),
            sha256: actual_checksum,
            managed_path: relative,
            managed_sha256,
        };
        fs::write(
            staging.join("manifest.json"),
            serde_json::to_vec_pretty(&manifest)?,
        )?;
        Ok(ResolvedDependency {
            name: name.to_owned(),
            version: spec.version.clone(),
            path: managed,
            installed: true,
        })
    }

    /// 根据版本清单、目标路径和可选命令验证现有安装。
    fn check_one(
        &self,
        name: &str,
        spec: &ManagedDependencySpec,
        installed: bool,
    ) -> Result<ResolvedDependency, SourceError> {
        let install_root = self.install_root(name, &spec.version);
        let manifest: InstallManifest =
            serde_json::from_slice(&fs::read(install_root.join("manifest.json"))?)?;
        if manifest.name != name
            || manifest.source != spec.source
            || manifest.mirrors != spec.mirrors
            || manifest.version != spec.version
            || spec.checksum.as_deref().is_some_and(|expected| {
                !manifest
                    .sha256
                    .eq_ignore_ascii_case(expected.strip_prefix("sha256:").unwrap_or(expected))
            })
            || !archive::safe_relative(&manifest.managed_path)
        {
            return Err(SourceError::ManagedPath(format!(
                "`{name}` 的版本清单与配置不一致"
            )));
        }
        let managed = install_root.join(&manifest.managed_path);
        if !managed.exists() {
            return Err(SourceError::ManagedPath(managed.display().to_string()));
        }
        if fingerprint(&managed)? != manifest.managed_sha256 {
            return Err(SourceError::Integrity {
                name: name.to_owned(),
            });
        }
        verify::ensure_kind(&managed, spec.kind)?;
        verify::run_version_check(&install_root, &managed, &spec.version, spec.verify.as_ref())?;
        Ok(ResolvedDependency {
            name: name.to_owned(),
            version: spec.version.clone(),
            path: managed,
            installed,
        })
    }

    /// 计算固定版本对应的安装目录。
    fn install_root(&self, name: &str, version: &str) -> PathBuf {
        self.service_root
            .join(".procora/dependencies")
            .join(name)
            .join(version)
    }

    /// 获取单依赖跨线程、跨进程安装锁，避免并发删除或覆盖缓存。
    fn acquire_install_lock(&self, name: &str) -> Result<fs::File, SourceError> {
        let directory = self.service_root.join(".procora/dependencies/.locks");
        fs::create_dir_all(&directory)?;
        let lock = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(directory.join(format!("{name}.lock")))?;
        lock.lock_exclusive()?;
        Ok(lock)
    }

    /// 用可回滚的同卷重命名激活完整暂存安装。
    fn activate_install(
        name: &str,
        staging: &Path,
        install_root: &Path,
    ) -> Result<(), SourceError> {
        let backup = install_root.with_file_name(format!(
            ".backup-{}-{}-{name}",
            std::process::id(),
            NEXT_STAGING_ID.fetch_add(1, Ordering::Relaxed)
        ));
        let had_previous = install_root.exists();
        if had_previous {
            fs::rename(install_root, &backup)?;
        }
        if let Err(error) = fs::rename(staging, install_root) {
            if had_previous {
                let _ = fs::rename(&backup, install_root);
            }
            return Err(error.into());
        }
        if had_previous {
            let _ = fs::remove_dir_all(backup);
        }
        Ok(())
    }
}

/// 计算文件的十六进制 SHA-256。
fn sha256(path: &Path) -> Result<String, SourceError> {
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

/// 为单文件或完整目录生成稳定的内容指纹。
fn fingerprint(path: &Path) -> Result<String, SourceError> {
    if path.is_file() {
        return sha256(path);
    }
    if !path.is_dir() {
        return Err(SourceError::ManagedPath(path.display().to_string()));
    }
    let mut files = Vec::new();
    collect_files(path, path, &mut files)?;
    files.sort_by(|left, right| left.0.cmp(&right.0));
    let mut digest = Sha256::new();
    for (relative, file) in files {
        digest.update(relative.to_string_lossy().as_bytes());
        digest.update([0]);
        digest.update(sha256(&file)?.as_bytes());
        digest.update([0]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

/// 递归收集目录内普通文件及其相对路径。
fn collect_files(
    root: &Path,
    directory: &Path,
    files: &mut Vec<(PathBuf, PathBuf)>,
) -> Result<(), SourceError> {
    for entry in fs::read_dir(directory)? {
        let path = entry?.path();
        if path.is_dir() {
            collect_files(root, &path, files)?;
        } else if path.is_file() {
            let relative = path
                .strip_prefix(root)
                .map_err(|_| SourceError::ManagedPath(path.display().to_string()))?
                .to_path_buf();
            files.push((relative, path));
        }
    }
    Ok(())
}

/// 相对本地来源以服务目录为基准解析。
fn resolve_local_source(root: &Path, source: &str) -> String {
    if source.contains("://") || source.contains(":/") || Path::new(source).is_absolute() {
        source.to_owned()
    } else {
        root.join(source).to_string_lossy().into_owned()
    }
}

/// 相对 SSH 辅助文件以服务目录为基准解析。
fn resolve_service_path(root: &Path, path: PathBuf) -> PathBuf {
    if path.is_absolute() {
        path
    } else {
        root.join(path)
    }
}

/// 按显式路径、类型和单根目录规则选择最终管理对象。
fn select_managed_path(
    name: &str,
    spec: &ManagedDependencySpec,
    content: &Path,
) -> Result<PathBuf, SourceError> {
    if let Some(path) = spec.path.as_ref() {
        if !archive::safe_relative(path) {
            return Err(SourceError::ManagedPath(path.display().to_string()));
        }
        let selected = content.join(path);
        return selected
            .exists()
            .then_some(selected)
            .ok_or_else(|| SourceError::ManagedPath(path.display().to_string()));
    }
    let entries = fs::read_dir(content)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()?;
    match spec.kind {
        DependencyKind::Directory => Ok(single_entry(entries).unwrap_or_else(|| content.into())),
        DependencyKind::Binary | DependencyKind::File => select_file(name, content),
        DependencyKind::Auto => Ok(single_entry(entries).unwrap_or_else(|| content.into())),
    }
}

/// 当目录只有一个可见条目时返回它。
fn single_entry(mut entries: Vec<PathBuf>) -> Option<PathBuf> {
    (entries.len() == 1).then(|| entries.remove(0))
}

/// 递归选择同名文件，或唯一普通文件。
fn select_file(name: &str, root: &Path) -> Result<PathBuf, SourceError> {
    let mut pending = vec![root.to_path_buf()];
    let mut files = Vec::new();
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(directory)? {
            let path = entry?.path();
            if path.is_dir() {
                pending.push(path);
            } else if path.is_file() {
                files.push(path);
            }
        }
    }
    files
        .iter()
        .find(|path| path.file_stem().is_some_and(|stem| stem == name))
        .cloned()
        .or_else(|| single_entry(files))
        .ok_or_else(|| SourceError::ManagedPath(format!("无法在归档中自动确定 `{name}`")))
}
