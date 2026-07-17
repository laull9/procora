//! 任务定义来源与本地配置变更监听。

mod archive;
mod download;
mod git;
mod manager;
mod placeholders;
mod revision;
mod verify;

use std::path::{Path, PathBuf};

use crate::config::{CompiledProject, ConfigError, load_path};
use notify::{Event, EventHandler, RecommendedWatcher, RecursiveMode, Watcher};

pub use git::{GitDefinitionCandidate, GitSource, GitSourceError, GitSourceLimits};
pub use manager::{DependencyManager, ResolvedDependency, SourceError};
pub use revision::{DefinitionCandidate, DefinitionRevision, LocalFileMonitor};

/// 以单个本地配置文件为入口的任务定义源。
#[derive(Clone, Debug)]
pub struct LocalFileSource {
    path: PathBuf,
}

impl LocalFileSource {
    /// 创建指向指定配置入口的定义源。
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// 返回配置入口路径。
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 读取并完整编译当前配置内容。
    ///
    /// # Errors
    ///
    /// 当配置无法读取、解析、校验或编译时返回错误。
    pub fn load(&self) -> Result<CompiledProject, ConfigError> {
        load_path(&self.path)
    }

    /// 监听入口父目录并把涉及入口的原始事件交给上层处理。
    ///
    /// # Errors
    ///
    /// 当平台监听器无法创建或无法监听目标路径时返回错误。
    pub fn watch<H>(&self, mut event_handler: H) -> notify::Result<RecommendedWatcher>
    where
        H: EventHandler,
    {
        let target = absolute_path(&self.path);
        let callback_target = target.clone();
        let mut watcher = notify::recommended_watcher(move |event: notify::Result<Event>| {
            let relevant = event.as_ref().map_or(true, |event| {
                event.paths.is_empty()
                    || event
                        .paths
                        .iter()
                        .any(|path| absolute_path(path) == callback_target)
            });
            if relevant {
                event_handler.handle_event(event);
            }
        })?;
        let parent = target.parent().unwrap_or_else(|| Path::new("."));
        watcher.watch(parent, RecursiveMode::NonRecursive)?;
        Ok(watcher)
    }
}

/// 返回不要求目标已经存在的绝对监听路径。
fn absolute_path(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    }
}
