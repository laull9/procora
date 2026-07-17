//! Git 来源的参数校验、资源计量与缓存指纹支持。

use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::{self, Read},
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use sha2::{Digest, Sha256};

use crate::{
    config::ConfigFormat,
    core::{RestartPolicy, TaskSpec},
};

use super::{GitSourceError, GitSourceLimits};

/// 同一进程内临时目录的去重序列。
static NEXT_STAGING_ID: AtomicU64 = AtomicU64::new(1);

/// 自动清理未提交的 Git 获取暂存目录。
pub(super) struct StagingDirectory {
    pub(super) path: PathBuf,
}

impl StagingDirectory {
    /// 创建进程和序列号共同去重的暂存目录。
    pub(super) fn new(cache_root: &Path) -> io::Result<Self> {
        let id = NEXT_STAGING_ID.fetch_add(1, Ordering::Relaxed);
        let path = cache_root.join(format!(".git-tmp-{}-{id}", std::process::id()));
        fs::create_dir(&path)?;
        Ok(Self { path })
    }
}

impl Drop for StagingDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

/// 创建 Git 缓存跨进程排他锁文件。
pub(super) fn open_lock(cache_root: &Path) -> io::Result<File> {
    let path = cache_root.join("git-source.lock");
    if fs::symlink_metadata(&path).is_ok_and(|metadata| !metadata.is_file()) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Git 缓存锁不是普通文件",
        ));
    }
    OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)
}

/// 创建或验证缓存内部目录没有借助链接逃逸根目录。
pub(super) fn require_cache_directory(
    cache_root: &Path,
    directory: &Path,
) -> Result<(), GitSourceError> {
    match fs::symlink_metadata(directory) {
        Ok(metadata) if !metadata.is_dir() => {
            return Err(GitSourceError::UnsafeCheckout(directory.to_path_buf()));
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => fs::create_dir(directory)?,
        Err(error) => return Err(error.into()),
    }
    let canonical_root = fs::canonicalize(cache_root)?;
    let canonical_directory = fs::canonicalize(directory)?;
    if !canonical_directory.starts_with(canonical_root) {
        return Err(GitSourceError::UnsafeCheckout(directory.to_path_buf()));
    }
    Ok(())
}

/// 构造不继承环境且不经过 shell 的 Git 任务规范。
pub(super) fn git_task(args: Vec<String>, cwd: &Path) -> TaskSpec {
    TaskSpec {
        command: "git".to_owned(),
        args,
        cwd: Some(cwd.to_path_buf()),
        env: BTreeMap::from([
            ("GCM_INTERACTIVE".to_owned(), "Never".to_owned()),
            ("GIT_CONFIG_GLOBAL".to_owned(), null_device().to_owned()),
            ("GIT_CONFIG_NOSYSTEM".to_owned(), "1".to_owned()),
            ("GIT_TERMINAL_PROMPT".to_owned(), "0".to_owned()),
            ("LC_ALL".to_owned(), "C".to_owned()),
        ]),
        healthcheck: None,
        success_exit_codes: BTreeSet::from([0]),
        depends_on: BTreeMap::new(),
        restart: RestartPolicy::Never,
        restart_delay_ms: 500,
        max_restarts: 0,
        restart_reset_after_ms: 60_000,
        shutdown_timeout_ms: 100,
    }
}

/// 校验资源边界没有被配置成无效或互相矛盾的值。
pub(super) fn validate_limits(limits: &GitSourceLimits) -> Result<(), GitSourceError> {
    if limits.command_timeout.is_zero()
        || limits.archive_bytes == 0
        || limits.checkout_bytes == 0
        || limits.repository_bytes == 0
        || limits.cache_bytes < limits.checkout_bytes
        || limits.checkout_files == 0
    {
        return Err(GitSourceError::Invalid(
            "Git 资源边界必须为正且缓存不小于 checkout".to_owned(),
        ));
    }
    Ok(())
}

/// 只接受不会触发本地或扩展协议的远端仓库写法。
pub(super) fn validate_remote(repository: &str) -> Result<(), GitSourceError> {
    let https = repository
        .strip_prefix("https://")
        .is_some_and(valid_https_remote);
    let ssh = repository
        .strip_prefix("ssh://")
        .is_some_and(valid_ssh_remote)
        || is_scp_repository(repository);
    let external_helper = repository.contains("::") && !repository.contains("://");
    if repository.starts_with('-') || external_helper || !(https || ssh) {
        return Err(GitSourceError::Invalid(
            "远端仓库只支持无内嵌凭据的 https://、ssh:// 或 SCP 写法".to_owned(),
        ));
    }
    Ok(())
}

/// 校验 HTTPS authority 不携带凭据、控制字符或空主机。
fn valid_https_remote(value: &str) -> bool {
    let authority = value.split('/').next().unwrap_or_default();
    !authority.is_empty()
        && !authority.contains('@')
        && remote_text_safe(value)
        && !authority.starts_with('-')
}

/// 校验 SSH authority 中主机不会被底层 ssh 当作选项。
fn valid_ssh_remote(value: &str) -> bool {
    let (authority, path) = value.split_once('/').unwrap_or((value, ""));
    let host_port = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    let host = host_port.split(':').next().unwrap_or_default();
    !host.is_empty()
        && !host.starts_with('-')
        && !path.is_empty()
        && remote_text_safe(value)
        && authority
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"@.-_:[]".contains(&byte))
}

/// 判断仓库是否使用 `user@host:path` 的 SCP 写法。
fn is_scp_repository(value: &str) -> bool {
    !value.contains("://")
        && value.split_once(':').is_some_and(|(authority, path)| {
            let host = authority
                .rsplit_once('@')
                .map_or(authority, |(_, host)| host);
            !host.is_empty()
                && !host.starts_with('-')
                && !authority.contains('/')
                && !path.is_empty()
                && !path.starts_with([':', '-'])
                && remote_text_safe(value)
                && authority
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"@.-_[]".contains(&byte))
        })
}

/// 拒绝远端文本中的空白、控制字符和反斜杠歧义。
fn remote_text_safe(value: &str) -> bool {
    !value.contains('\\')
        && !value
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
}

/// 限制 Git 引用字符，阻止选项和 refspec 注入。
pub(super) fn validate_reference(reference: &str) -> Result<(), GitSourceError> {
    let safe = !reference.is_empty()
        && reference.len() <= 256
        && !reference.starts_with(['-', '/', '.'])
        && !reference.ends_with(['/', '.'])
        && !reference.contains("..")
        && !reference.contains("//")
        && reference
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"/-_.".contains(&byte));
    if !safe {
        return Err(GitSourceError::Invalid("Git 引用包含不安全字符".to_owned()));
    }
    Ok(())
}

/// 配置入口必须是普通相对声明式文件，远端 Python 永不执行。
pub(super) fn validate_config_path(path: &Path) -> Result<(), GitSourceError> {
    if path.is_absolute()
        || path.as_os_str().is_empty()
        || !path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
        || ConfigFormat::from_path(path).is_none()
    {
        return Err(GitSourceError::Invalid(
            "配置入口必须是不含点或父目录的相对 YAML/TOML/JSON 路径".to_owned(),
        ));
    }
    Ok(())
}

/// 返回路径的 UTF-8 文本，否则拒绝有损传给 Git。
pub(super) fn path_text(path: &Path) -> Result<&str, GitSourceError> {
    path.to_str()
        .ok_or_else(|| GitSourceError::Invalid(format!("路径 `{}` 不是 UTF-8", path.display())))
}

/// 检查路径可以无损传递给当前基于 `TaskSpec` 的辅助命令。
pub(super) fn require_unicode(path: &Path, label: &str) -> Result<(), GitSourceError> {
    path.to_str()
        .is_some()
        .then_some(())
        .ok_or_else(|| GitSourceError::Invalid(format!("{label}必须是 UTF-8")))
}

/// 返回 Git 可接受的本地仓库路径文本，并移除 Windows 扩展路径前缀。
pub(super) fn git_local_repository_text(path: &Path) -> String {
    let text = path.to_str().expect("本地仓库路径已验证为 UTF-8");
    #[cfg(windows)]
    {
        if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
            return format!(r"\\{rest}");
        }
        if let Some(rest) = text.strip_prefix(r"\\?\") {
            return rest.to_owned();
        }
    }
    text.to_owned()
}

/// 当前平台传给 Git 的空文件和禁用 hooks 路径。
pub(super) const fn null_device() -> &'static str {
    if cfg!(windows) { "NUL" } else { "/dev/null" }
}

/// 判断 Git 提交输出是否为 SHA-1 或 SHA-256 完整哈希。
pub(super) fn valid_commit(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// 递归计算目录字节，不跟随符号链接。
pub(super) fn directory_usage(path: &Path) -> io::Result<u64> {
    let mut total = 0_u64;
    let mut pending = vec![path.to_path_buf()];
    while let Some(current) = pending.pop() {
        for entry in fs::read_dir(current)? {
            let entry = entry?;
            let metadata = match fs::symlink_metadata(entry.path()) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error),
            };
            if metadata.is_dir() {
                pending.push(entry.path());
            } else if metadata.is_file() {
                total = total.saturating_add(metadata.len());
            }
        }
    }
    Ok(total)
}

/// 对单个目录实施总字节上限。
pub(super) fn enforce_directory_bytes(
    path: &Path,
    limit: u64,
    label: &str,
) -> Result<(), GitSourceError> {
    let bytes = directory_usage(path)?;
    if bytes > limit {
        return Err(GitSourceError::Limit(format!("{label}超过 {limit} 字节")));
    }
    Ok(())
}

/// 生成 checkout 路径、内容和可执行位的稳定指纹并实施边界。
pub(super) fn checkout_fingerprint(
    root: &Path,
    byte_limit: u64,
    file_limit: usize,
) -> Result<String, GitSourceError> {
    let mut files = Vec::new();
    collect_checkout_files(root, root, &mut files)?;
    if files.len() > file_limit {
        return Err(GitSourceError::Limit(format!(
            "checkout 文件数超过 {file_limit}"
        )));
    }
    files.sort_by(|left, right| left.0.cmp(&right.0));
    let mut total = 0_u64;
    let mut digest = Sha256::new();
    for (relative, path) in files {
        let metadata = fs::metadata(&path)?;
        total = total.saturating_add(metadata.len());
        if total > byte_limit {
            return Err(GitSourceError::Limit(format!(
                "checkout 超过 {byte_limit} 字节"
            )));
        }
        digest.update(relative.to_string_lossy().as_bytes());
        digest.update([0]);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            digest.update((metadata.permissions().mode() & 0o111).to_le_bytes());
        }
        let mut file = File::open(path)?;
        let mut buffer = vec![0_u8; 64 * 1024].into_boxed_slice();
        loop {
            let count = file.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            digest.update(&buffer[..count]);
        }
        digest.update([0]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

/// 递归收集 checkout 内普通文件，不跟随其他文件类型。
fn collect_checkout_files(
    root: &Path,
    directory: &Path,
    files: &mut Vec<(PathBuf, PathBuf)>,
) -> Result<(), GitSourceError> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let metadata = fs::symlink_metadata(entry.path())?;
        if metadata.is_dir() {
            collect_checkout_files(root, &entry.path(), files)?;
        } else if metadata.is_file() {
            let path = entry.path();
            let relative = path.strip_prefix(root).expect("递归路径保持在 checkout 内");
            files.push((relative.to_path_buf(), path));
        } else {
            return Err(GitSourceError::UnsafeCheckout(entry.path()));
        }
    }
    Ok(())
}

/// 把仓库标识散列为不会包含路径字符的缓存目录名。
pub(super) fn repository_id(repository: &str) -> String {
    format!("{:x}", Sha256::digest(repository.as_bytes()))
}

/// 组合来源、不可变提交和本地配置闭包修订。
pub(super) fn git_revision(repository: &str, commit: &str, local_revision: Option<&str>) -> String {
    let mut digest = Sha256::new();
    digest.update(b"procora-git-definition-v1\0");
    digest.update(repository.as_bytes());
    digest.update([0]);
    digest.update(commit.as_bytes());
    digest.update([0]);
    digest.update(local_revision.unwrap_or("missing").as_bytes());
    format!("{:x}", digest.finalize())
}
