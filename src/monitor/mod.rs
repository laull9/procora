//! 受管任务进程的跨平台资源采样。

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use sysinfo::{Pid, ProcessesToUpdate, System};

/// 单个进程在一个采样时刻的资源快照。
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct ResourceSnapshot {
    /// 顶层进程标识。
    pub pid: u32,
    /// 当前 CPU 使用率。
    pub cpu_percent: f32,
    /// 常驻内存字节数。
    pub memory_bytes: u64,
    /// 从进程启动以来累计读取字节数。
    pub read_bytes: u64,
    /// 从进程启动以来累计写入字节数。
    pub written_bytes: u64,
}

/// 基于 sysinfo 的进程资源采样器。
#[derive(Debug)]
pub struct SystemMonitor {
    system: System,
    logical_cpu_count: u16,
}

impl SystemMonitor {
    /// 创建空的资源采样器。
    pub fn new() -> Self {
        Self {
            system: System::new(),
            logical_cpu_count: std::thread::available_parallelism()
                .map_or(1, |count| u16::try_from(count.get()).unwrap_or(u16::MAX)),
        }
    }

    /// 刷新并返回指定进程的快照。
    pub fn snapshot(&mut self, pid: u32) -> Option<ResourceSnapshot> {
        let pid = Pid::from_u32(pid);
        self.system
            .refresh_processes(ProcessesToUpdate::Some(&[pid]), true);
        self.system.process(pid).map(|process| {
            let disk = process.disk_usage();
            ResourceSnapshot {
                pid: pid.as_u32(),
                cpu_percent: normalize_cpu_percent(process.cpu_usage(), self.logical_cpu_count),
                memory_bytes: process.memory(),
                read_bytes: disk.total_read_bytes,
                written_bytes: disk.total_written_bytes,
            }
        })
    }

    /// 刷新并聚合顶层进程及全部可识别后代的资源快照。
    pub fn snapshot_tree(&mut self, pid: u32) -> Option<ResourceSnapshot> {
        self.snapshot_trees(&[pid]).remove(&pid)
    }

    /// 一次刷新后批量聚合多个顶层进程及其可识别后代。
    ///
    /// 不存在的根进程不会出现在结果中；嵌套根会分别获得完整子树聚合值。
    pub fn snapshot_trees(&mut self, pids: &[u32]) -> BTreeMap<u32, ResourceSnapshot> {
        if pids.is_empty() {
            return BTreeMap::new();
        }
        self.system.refresh_processes(ProcessesToUpdate::All, true);
        let mut children = BTreeMap::<Pid, Vec<Pid>>::new();
        for (candidate, process) in self.system.processes() {
            if let Some(parent) = process.parent() {
                children.entry(parent).or_default().push(*candidate);
            }
        }

        pids.iter()
            .copied()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .filter_map(|pid| {
                let root = Pid::from_u32(pid);
                self.system.process(root)?;
                let mut snapshot = ResourceSnapshot {
                    pid,
                    cpu_percent: 0.0,
                    memory_bytes: 0,
                    read_bytes: 0,
                    written_bytes: 0,
                };
                let mut pending = vec![root];
                while let Some(process_pid) = pending.pop() {
                    let Some(process) = self.system.process(process_pid) else {
                        continue;
                    };
                    let disk = process.disk_usage();
                    snapshot.cpu_percent += process.cpu_usage();
                    snapshot.memory_bytes = snapshot.memory_bytes.saturating_add(process.memory());
                    snapshot.read_bytes = snapshot.read_bytes.saturating_add(disk.total_read_bytes);
                    snapshot.written_bytes = snapshot
                        .written_bytes
                        .saturating_add(disk.total_written_bytes);
                    if let Some(descendants) = children.get(&process_pid) {
                        pending.extend(descendants);
                    }
                }
                snapshot.cpu_percent =
                    normalize_cpu_percent(snapshot.cpu_percent, self.logical_cpu_count);
                Some((pid, snapshot))
            })
            .collect()
    }
}

/// 把 sysinfo 以单核为 100% 的进程值换算为整机可用 CPU 容量占比。
fn normalize_cpu_percent(cpu_percent: f32, logical_cpu_count: u16) -> f32 {
    (cpu_percent.max(0.0) / f32::from(logical_cpu_count.max(1))).min(100.0)
}

impl Default for SystemMonitor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::normalize_cpu_percent;

    #[test]
    // 多核进程值按可用逻辑CPU总容量归一化并限制在百分之百以内。
    fn cpu_percent_uses_total_available_capacity() {
        for (raw, cores, expected) in [
            (250.0, 4, 62.5),
            (800.0, 4, 100.0),
            (-1.0, 4, 0.0),
            (50.0, 0, 50.0),
        ] {
            assert!((normalize_cpu_percent(raw, cores) - expected).abs() < f32::EPSILON);
        }
    }
}
