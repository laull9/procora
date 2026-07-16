use procora_config::DiscoveredProject;
use procora_source::{DependencyManager, ResolvedDependency, SourceError};

/// 中心内存中保留的最大增量事件数量。
pub(crate) const EVENT_CAPACITY: usize = 256;

/// 单次 IPC 日志响应允许分配的最大字节数。
pub(crate) const MAX_LOG_BATCH_BYTES: usize = 1024 * 1024;

/// 同步服务依赖并把任务占位符替换为已验证路径。
pub(crate) fn prepare(
    discovered: &mut DiscoveredProject,
) -> Result<Vec<ResolvedDependency>, SourceError> {
    DependencyManager::new(&discovered.root).prepare(&mut discovered.compiled)
}
