//! 固定提交、资源有界且只生成候选的 Git 任务定义源。

mod support;

use std::{
    fs, io,
    path::{Path, PathBuf},
    time::Duration,
};

use fs2::FileExt;
use thiserror::Error;

use crate::{
    config::{CompiledProject, ConfigError},
    process::run_bounded_command_monitored,
};

use super::{LocalFileSource, SourceError, archive};
use support::{
    StagingDirectory, checkout_fingerprint, directory_usage, enforce_directory_bytes,
    git_local_repository_text, git_revision, git_task, null_device, open_lock, path_text,
    repository_id, require_cache_directory, require_unicode, valid_commit, validate_config_path,
    validate_limits, validate_reference, validate_remote,
};

/// Git 命令与仓库资源的默认硬边界。
#[derive(Clone, Debug)]
pub struct GitSourceLimits {
    /// 每条 Git 命令最长执行时间。
    pub command_timeout: Duration,
    /// `git archive` 允许保留的最大字节数。
    pub archive_bytes: usize,
    /// 展开后普通文件的最大总字节数。
    pub checkout_bytes: u64,
    /// 单次临时对象仓库的最大总字节数。
    pub repository_bytes: u64,
    /// 持久 checkout 缓存的最大总字节数。
    pub cache_bytes: u64,
    /// 展开后最多允许的普通文件数量。
    pub checkout_files: usize,
}

impl Default for GitSourceLimits {
    fn default() -> Self {
        Self {
            command_timeout: Duration::from_secs(30),
            archive_bytes: 40 * 1024 * 1024,
            checkout_bytes: 32 * 1024 * 1024,
            repository_bytes: 128 * 1024 * 1024,
            cache_bytes: 256 * 1024 * 1024,
            checkout_files: 4096,
        }
    }
}

/// 已固定到不可变提交且尚未应用的 Git 配置候选。
#[derive(Debug)]
pub struct GitDefinitionCandidate {
    /// 用户声明的远端或本地仓库标识。
    pub repository: String,
    /// 本次查询使用的分支、标签或提交引用。
    pub reference: String,
    /// Git 解析出的完整不可变提交哈希。
    pub commit: String,
    /// 组合提交和配置闭包后的确认修订。
    pub revision: String,
    /// 只读语义的本地物化根目录。
    pub checkout_root: PathBuf,
    /// checkout 内实际编译的配置入口。
    pub config_path: PathBuf,
    /// 完整配置编译结果；失败候选不会被静默丢弃。
    pub compiled: Result<CompiledProject, ConfigError>,
}

/// Git 来源初始化、获取、物化或缓存完整性错误。
#[derive(Debug, Error)]
pub enum GitSourceError {
    /// 来源、引用或路径不满足安全约束。
    #[error("Git 来源参数无效：{0}")]
    Invalid(String),
    /// 本地文件系统操作失败。
    #[error(transparent)]
    Io(#[from] io::Error),
    /// Git 子命令失败或越过资源边界。
    #[error("Git {operation} 失败：{message}")]
    Git {
        /// 便于定位的 Git 操作名称。
        operation: &'static str,
        /// 有界 stderr 或进程边界诊断。
        message: String,
    },
    /// 临时对象库、归档、checkout 或持久缓存超过上限。
    #[error("Git 来源资源超过边界：{0}")]
    Limit(String),
    /// 同一提交的本地不可变缓存已经被修改。
    #[error("Git 提交缓存 `{0}` 已被修改，拒绝复用")]
    CacheIntegrity(PathBuf),
    /// checkout 中出现 Git 归档不会创建的链接或特殊文件。
    #[error("Git checkout 包含不安全或被修改的文件类型 `{0}`")]
    UnsafeCheckout(PathBuf),
    /// 确认时重新获取的来源修订已经变化。
    #[error("Git 候选修订已变化：期望 {expected}，实际 {actual}")]
    StaleRevision {
        /// 用户预览并确认的旧修订。
        expected: String,
        /// 重新获取后计算的新修订。
        actual: String,
    },
    /// 安全解包 Git 归档失败。
    #[error(transparent)]
    Materialize(#[from] SourceError),
}

/// 仓库传输类型决定本地 file 协议是否可用。
#[derive(Clone, Debug)]
enum Repository {
    Remote(String),
    Local(PathBuf),
}

/// 只获取、固定和编译候选，不直接启动 Task 的 Git 定义源。
#[derive(Clone, Debug)]
pub struct GitSource {
    repository: Repository,
    reference: String,
    config_path: PathBuf,
    cache_root: PathBuf,
    limits: GitSourceLimits,
}

impl GitSource {
    /// 创建仅允许 HTTPS、SSH URL 或 SCP 写法的远端来源。
    ///
    /// # Errors
    ///
    /// 当 URL、引用、配置路径或缓存路径不安全时返回错误。
    pub fn remote(
        repository: impl Into<String>,
        reference: impl Into<String>,
        config_path: impl Into<PathBuf>,
        cache_root: impl Into<PathBuf>,
    ) -> Result<Self, GitSourceError> {
        let repository = repository.into();
        validate_remote(&repository)?;
        Self::build(
            Repository::Remote(repository),
            reference.into(),
            config_path.into(),
            cache_root.into(),
        )
    }

    /// 创建用户显式信任的本地仓库来源。
    ///
    /// # Errors
    ///
    /// 当仓库不可访问，或引用和配置路径不安全时返回错误。
    pub fn local(
        repository: impl AsRef<Path>,
        reference: impl Into<String>,
        config_path: impl Into<PathBuf>,
        cache_root: impl Into<PathBuf>,
    ) -> Result<Self, GitSourceError> {
        let repository = fs::canonicalize(repository.as_ref())?;
        require_unicode(&repository, "本地仓库路径")?;
        Self::build(
            Repository::Local(repository),
            reference.into(),
            config_path.into(),
            cache_root.into(),
        )
    }

    /// 覆盖默认资源边界，主要供受限环境和故障测试使用。
    #[must_use]
    pub fn with_limits(mut self, limits: GitSourceLimits) -> Self {
        self.limits = limits;
        self
    }

    /// 获取引用、固定提交、物化受限 checkout 并完整编译候选。
    ///
    /// 该方法不会注册服务、准备管理依赖或启动 Task。
    ///
    /// # Errors
    ///
    /// 当 Git、文件系统、归档或缓存完整性检查失败时返回错误。
    pub fn fetch_candidate(&self) -> Result<GitDefinitionCandidate, GitSourceError> {
        validate_limits(&self.limits)?;
        fs::create_dir_all(&self.cache_root)?;
        let lock = open_lock(&self.cache_root)?;
        lock.lock_exclusive()?;
        let result = enforce_directory_bytes(
            &self.cache_root,
            self.limits.cache_bytes,
            "持久 checkout 缓存",
        )
        .and_then(|()| self.fetch_locked());
        FileExt::unlock(&lock)?;
        result
    }

    /// 重新获取来源，并且只在修订仍与预览值一致时返回候选。
    ///
    /// # Errors
    ///
    /// 当获取失败或远端引用、提交内容、配置闭包已经变化时返回错误。
    pub fn confirm_candidate(
        &self,
        expected_revision: &str,
    ) -> Result<GitDefinitionCandidate, GitSourceError> {
        let candidate = self.fetch_candidate()?;
        if candidate.revision != expected_revision {
            return Err(GitSourceError::StaleRevision {
                expected: expected_revision.to_owned(),
                actual: candidate.revision,
            });
        }
        Ok(candidate)
    }

    /// 完成已经持有缓存锁的一次候选获取。
    fn fetch_locked(&self) -> Result<GitDefinitionCandidate, GitSourceError> {
        let staging = StagingDirectory::new(&self.cache_root)?;
        let (commit, archive) = self.fetch_archive(&staging)?;
        let archive_path = staging.path.join("source.tar");
        fs::write(&archive_path, archive)?;
        let content = staging.path.join("content");
        archive::materialize(
            &archive_path,
            "source.tar",
            &content,
            crate::config::UnpackMode::Auto,
        )?;
        fs::remove_file(archive_path)?;
        let checkout_fingerprint = checkout_fingerprint(
            &content,
            self.limits.checkout_bytes,
            self.limits.checkout_files,
        )?;
        let checkout_root = self.install_checkout(&content, &commit, &checkout_fingerprint)?;
        let config_path = checkout_root.join(&self.config_path);
        let local_candidate = LocalFileSource::new(&config_path).read_candidate();
        let revision = git_revision(
            &self.repository_text(),
            &commit,
            local_candidate
                .revision
                .as_ref()
                .map(super::DefinitionRevision::as_str),
        );
        Ok(GitDefinitionCandidate {
            repository: self.repository_text(),
            reference: self.reference.clone(),
            commit,
            revision,
            checkout_root,
            config_path,
            compiled: local_candidate.compiled,
        })
    }

    /// 在临时裸仓库中获取引用、解析提交并导出有界 tar。
    fn fetch_archive(
        &self,
        staging: &StagingDirectory,
    ) -> Result<(String, Vec<u8>), GitSourceError> {
        let repository_dir = staging.path.join("repository.git");
        self.git(
            "初始化",
            ["init", "--bare", "--quiet", path_text(&repository_dir)?],
            1024,
        )?;
        self.git(
            "设置远端",
            [
                "-C",
                path_text(&repository_dir)?,
                "remote",
                "add",
                "origin",
                &self.repository_text(),
            ],
            1024,
        )?;
        self.fetch_reference(&repository_dir)?;
        enforce_directory_bytes(
            &repository_dir,
            self.limits.repository_bytes,
            "临时对象仓库",
        )?;
        let commit_output = self.git(
            "解析提交",
            [
                "-C",
                path_text(&repository_dir)?,
                "rev-parse",
                "--verify",
                "FETCH_HEAD^{commit}",
            ],
            256,
        )?;
        let commit = String::from_utf8_lossy(&commit_output)
            .trim()
            .to_ascii_lowercase();
        if !valid_commit(&commit) {
            return Err(GitSourceError::Git {
                operation: "解析提交",
                message: "Git 未返回完整十六进制提交哈希".to_owned(),
            });
        }
        let archive = self.git(
            "导出提交",
            [
                "-C",
                path_text(&repository_dir)?,
                "archive",
                "--format=tar",
                &commit,
            ],
            self.limits.archive_bytes,
        )?;
        fs::remove_dir_all(&repository_dir)?;
        Ok((commit, archive))
    }

    /// 校验通用参数并构造来源。
    fn build(
        repository: Repository,
        reference: String,
        config_path: PathBuf,
        cache_root: PathBuf,
    ) -> Result<Self, GitSourceError> {
        validate_reference(&reference)?;
        validate_config_path(&config_path)?;
        require_unicode(&cache_root, "缓存路径")?;
        Ok(Self {
            repository,
            reference,
            config_path,
            cache_root,
            limits: GitSourceLimits::default(),
        })
    }

    /// 运行一条关闭交互、全局配置和 hooks 的有界 Git 命令。
    fn git<'a>(
        &self,
        operation: &'static str,
        args: impl IntoIterator<Item = &'a str>,
        stdout_limit: usize,
    ) -> Result<Vec<u8>, GitSourceError> {
        self.git_monitored(operation, args, stdout_limit, || Ok(()))
    }

    /// 运行 Git 命令并在等待期间实施调用方资源检查。
    fn git_monitored<'a>(
        &self,
        operation: &'static str,
        args: impl IntoIterator<Item = &'a str>,
        stdout_limit: usize,
        monitor: impl FnMut() -> Result<(), String>,
    ) -> Result<Vec<u8>, GitSourceError> {
        let protocol = if matches!(self.repository, Repository::Local(_)) {
            "always"
        } else {
            "never"
        };
        let mut command_args = vec![
            "-c".to_owned(),
            "credential.helper=".to_owned(),
            "-c".to_owned(),
            format!("core.hooksPath={}", null_device()),
            "-c".to_owned(),
            format!("protocol.file.allow={protocol}"),
        ];
        command_args.extend(args.into_iter().map(str::to_owned));
        let task = git_task(command_args, &self.cache_root);
        let output = run_bounded_command_monitored(
            &task,
            self.limits.command_timeout,
            stdout_limit,
            1024 * 1024,
            monitor,
        )
        .map_err(|error| GitSourceError::Git {
            operation,
            message: error.to_string(),
        })?;
        if !output.status.success() {
            return Err(GitSourceError::Git {
                operation,
                message: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            });
        }
        Ok(output.stdout)
    }

    /// 获取引用并在对象库增长期间实施硬字节上限。
    fn fetch_reference(&self, repository_dir: &Path) -> Result<(), GitSourceError> {
        self.git_monitored(
            "获取引用",
            [
                "-C",
                path_text(repository_dir)?,
                "fetch",
                "--force",
                "--depth=1",
                "--filter=blob:none",
                "--no-tags",
                "origin",
                &self.reference,
            ],
            1024 * 1024,
            || {
                let bytes = directory_usage(repository_dir).map_err(|error| error.to_string())?;
                (bytes <= self.limits.repository_bytes)
                    .then_some(())
                    .ok_or_else(|| {
                        format!("临时对象仓库超过 {} 字节", self.limits.repository_bytes)
                    })
            },
        )?;
        Ok(())
    }

    /// 原子安装或验证相同提交的不可变 checkout。
    fn install_checkout(
        &self,
        content: &Path,
        commit: &str,
        expected_fingerprint: &str,
    ) -> Result<PathBuf, GitSourceError> {
        let repository_root = self
            .cache_root
            .join("checkouts")
            .join(repository_id(&self.repository_text()));
        require_cache_directory(&self.cache_root, &self.cache_root.join("checkouts"))?;
        require_cache_directory(&self.cache_root, &repository_root)?;
        let target = repository_root.join(commit);
        if fs::symlink_metadata(&target).is_ok() {
            require_cache_directory(&self.cache_root, &target)?;
            let actual = checkout_fingerprint(
                &target,
                self.limits.checkout_bytes,
                self.limits.checkout_files,
            )?;
            if actual != expected_fingerprint {
                return Err(GitSourceError::CacheIntegrity(target));
            }
            return Ok(target);
        }
        let persistent = directory_usage(&self.cache_root)?.saturating_sub(directory_usage(
            content.parent().expect("content 位于暂存目录"),
        )?);
        let checkout_size = directory_usage(content)?;
        if persistent.saturating_add(checkout_size) > self.limits.cache_bytes {
            return Err(GitSourceError::Limit(format!(
                "持久 checkout 缓存超过 {} 字节",
                self.limits.cache_bytes
            )));
        }
        fs::rename(content, &target)?;
        Ok(target)
    }

    /// 返回适合 Git 参数和审计展示的仓库文本。
    fn repository_text(&self) -> String {
        match &self.repository {
            Repository::Remote(value) => value.clone(),
            Repository::Local(path) => git_local_repository_text(path),
        }
    }
}
