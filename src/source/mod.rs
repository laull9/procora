//! 任务定义来源与本地配置变更监听。

mod archive;
mod download;
mod manager;
mod verify;

use std::path::{Path, PathBuf};

use crate::config::{CompiledProject, ConfigError, load_path};
use notify::{EventHandler, RecommendedWatcher, RecursiveMode, Watcher};

pub use manager::{DependencyManager, ResolvedDependency, SourceError};

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

    /// 监听入口文件并把原始文件系统事件交给上层去抖和重读。
    ///
    /// # Errors
    ///
    /// 当平台监听器无法创建或无法监听目标路径时返回错误。
    pub fn watch<H>(&self, event_handler: H) -> notify::Result<RecommendedWatcher>
    where
        H: EventHandler,
    {
        let mut watcher = notify::recommended_watcher(event_handler)?;
        watcher.watch(&self.path, RecursiveMode::NonRecursive)?;
        Ok(watcher)
    }
}
