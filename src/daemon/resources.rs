//! 单服务宿主的慢周期进程树资源缓存。

use std::{
    collections::{BTreeMap, BTreeSet},
    time::{Duration, Instant},
};

use crate::monitor::{ResourceSnapshot, SystemMonitor};

/// 默认资源采样周期，独立于状态和日志快照频率。
const RESOURCE_SAMPLE_INTERVAL: Duration = Duration::from_secs(1);

/// 缓存同一宿主全部活动 Task 的批量资源快照。
#[derive(Debug, Default)]
pub(super) struct ResourceCache {
    roots: BTreeSet<u32>,
    snapshots: BTreeMap<u32, ResourceSnapshot>,
    sampled_at: Option<Instant>,
}

impl ResourceCache {
    /// 按慢周期刷新活动根集合并返回当前快照。
    pub(super) fn snapshots(
        &mut self,
        monitor: &mut SystemMonitor,
        roots: BTreeSet<u32>,
    ) -> &BTreeMap<u32, ResourceSnapshot> {
        if roots.is_empty() {
            self.invalidate();
            return &self.snapshots;
        }
        let now = Instant::now();
        if self.needs_refresh(&roots, now) {
            let pids = roots.iter().copied().collect::<Vec<_>>();
            self.snapshots = monitor.snapshot_trees(&pids);
            self.roots = roots;
            self.sampled_at = Some(now);
        }
        &self.snapshots
    }

    /// Task 运行身份变化后强制下次请求重新采样。
    pub(super) fn invalidate(&mut self) {
        self.roots.clear();
        self.snapshots.clear();
        self.sampled_at = None;
    }

    /// 判断根集合或慢周期是否要求重新读取系统状态。
    fn needs_refresh(&self, roots: &BTreeSet<u32>, now: Instant) -> bool {
        self.sampled_at.is_none_or(|sampled_at| {
            self.roots != *roots || now.duration_since(sampled_at) >= RESOURCE_SAMPLE_INTERVAL
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    // 相同根集合在慢周期内复用缓存，根变化、超时或失效会重新采样。
    fn refresh_policy_tracks_roots_interval_and_invalidation() {
        let now = Instant::now();
        let roots = BTreeSet::from([1, 2]);
        let mut cache = ResourceCache::default();
        assert!(cache.needs_refresh(&roots, now));

        cache.roots.clone_from(&roots);
        cache.sampled_at = Some(now);
        assert!(!cache.needs_refresh(&roots, now + Duration::from_millis(999)));
        assert!(cache.needs_refresh(&roots, now + RESOURCE_SAMPLE_INTERVAL));
        assert!(cache.needs_refresh(&BTreeSet::from([1, 3]), now));

        cache.invalidate();
        assert!(cache.needs_refresh(&roots, now));
    }
}
