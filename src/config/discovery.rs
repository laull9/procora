use std::{
    fs,
    path::{Path, PathBuf},
};

use thiserror::Error;

use super::{CompiledProject, ConfigError, ConfigFormat, load_path};

/// 服务配置发现完成后的规范化结果。
#[derive(Debug)]
pub struct DiscoveredProject {
    /// 服务所在的规范化目录。
    pub root: PathBuf,
    /// 实际加载的规范化配置文件路径。
    pub config_path: PathBuf,
    /// 已通过完整校验的项目配置。
    pub compiled: CompiledProject,
}

/// 从路径发现服务配置时可能出现的错误。
#[derive(Debug, Error)]
pub enum DiscoveryError {
    /// 输入路径不存在或无法规范化。
    #[error("无法访问服务路径 `{path}`: {source}")]
    Access {
        /// 用户传入的路径。
        path: PathBuf,
        /// 文件系统错误。
        source: std::io::Error,
    },
    /// 目录内容无法读取。
    #[error("无法扫描服务目录 `{path}`: {source}")]
    ReadDirectory {
        /// 要扫描的目录。
        path: PathBuf,
        /// 文件系统错误。
        source: std::io::Error,
    },
    /// 显式指定的配置文件无效。
    #[error("配置文件 `{path}` 无效: {source}")]
    InvalidExplicit {
        /// 显式指定的配置文件。
        path: PathBuf,
        /// 配置编译错误。
        source: ConfigError,
    },
    /// 目录中没有约定名称的配置文件。
    #[error("服务目录 `{0}` 中没有 procora.yaml、procora.yml、procora.toml 或 procora.json")]
    NotFound(PathBuf),
    /// 目录中存在候选文件，但没有一个是合法 Procora 配置。
    #[error("服务目录 `{directory}` 中没有合法的 Procora 配置: {details}")]
    NoValidConfig {
        /// 被扫描的服务目录。
        directory: PathBuf,
        /// 各候选文件的失败摘要。
        details: String,
    },
    /// 目录中存在多个合法配置，无法安全选择。
    #[error("服务目录 `{directory}` 中存在多个合法配置，请显式指定其中一个: {candidates}")]
    Ambiguous {
        /// 被扫描的服务目录。
        directory: PathBuf,
        /// 按路径排序的合法候选摘要。
        candidates: String,
    },
    /// 输入既不是文件也不是目录。
    #[error("服务路径 `{0}` 既不是配置文件也不是目录")]
    UnsupportedPath(PathBuf),
}

/// 从显式配置文件或服务目录中发现并编译唯一配置。
///
/// # Errors
///
/// 当路径不可访问、配置无效、目录没有合法配置或存在多个合法配置时返回错误。
pub fn discover_path(path: impl AsRef<Path>) -> Result<DiscoveredProject, DiscoveryError> {
    let input = path.as_ref();
    let canonical = fs::canonicalize(input).map_err(|source| DiscoveryError::Access {
        path: input.to_path_buf(),
        source,
    })?;
    if canonical.is_file() {
        return discover_explicit(canonical);
    }
    if canonical.is_dir() {
        return discover_directory(canonical);
    }
    Err(DiscoveryError::UnsupportedPath(canonical))
}

/// 加载用户显式指定的配置文件。
fn discover_explicit(config_path: PathBuf) -> Result<DiscoveredProject, DiscoveryError> {
    let compiled = load_path(&config_path).map_err(|source| DiscoveryError::InvalidExplicit {
        path: config_path.clone(),
        source,
    })?;
    let root = config_path
        .parent()
        .expect("规范化文件路径应当具有父目录")
        .to_path_buf();
    Ok(DiscoveredProject {
        root,
        config_path,
        compiled,
    })
}

/// 扫描目录中的 `procora.*` 并选择唯一能完整编译的配置文件。
fn discover_directory(root: PathBuf) -> Result<DiscoveredProject, DiscoveryError> {
    let entries = fs::read_dir(&root).map_err(|source| DiscoveryError::ReadDirectory {
        path: root.clone(),
        source,
    })?;
    let mut candidates = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path.file_stem().is_some_and(|stem| stem == "procora")
                && ConfigFormat::from_path(path).is_some()
        })
        .collect::<Vec<_>>();
    candidates.sort();
    if candidates.is_empty() {
        return Err(DiscoveryError::NotFound(root));
    }

    let mut valid = Vec::new();
    let mut invalid = Vec::new();
    for path in candidates {
        match load_path(&path) {
            Ok(compiled) => valid.push((path, compiled)),
            Err(error) => invalid.push(format!("{}: {error}", path.display())),
        }
    }
    match valid.len() {
        0 => Err(DiscoveryError::NoValidConfig {
            directory: root,
            details: invalid.join("; "),
        }),
        1 => {
            let (config_path, compiled) = valid.pop().expect("长度已经确认");
            Ok(DiscoveredProject {
                root,
                config_path,
                compiled,
            })
        }
        _ => Err(DiscoveryError::Ambiguous {
            directory: root,
            candidates: valid
                .iter()
                .map(|(path, _)| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", "),
        }),
    }
}
