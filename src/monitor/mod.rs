//! 受管任务进程的跨平台资源采样。

use std::collections::BTreeSet;

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
}

impl SystemMonitor {
    /// 创建空的资源采样器。
    pub fn new() -> Self {
        Self {
            system: System::new(),
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
                cpu_percent: process.cpu_usage(),
                memory_bytes: process.memory(),
                read_bytes: disk.total_read_bytes,
                written_bytes: disk.total_written_bytes,
            }
        })
    }

    /// 刷新并聚合顶层进程及全部可识别后代的资源快照。
    pub fn snapshot_tree(&mut self, pid: u32) -> Option<ResourceSnapshot> {
        let root = Pid::from_u32(pid);
        self.system.refresh_processes(ProcessesToUpdate::All, true);
        self.system.process(root)?;
        let mut included = BTreeSet::from([root]);
        loop {
            let before = included.len();
            for (candidate, process) in self.system.processes() {
                if process
                    .parent()
                    .is_some_and(|parent| included.contains(&parent))
                {
                    included.insert(*candidate);
                }
            }
            if included.len() == before {
                break;
            }
        }
        let mut snapshot = ResourceSnapshot {
            pid,
            cpu_percent: 0.0,
            memory_bytes: 0,
            read_bytes: 0,
            written_bytes: 0,
        };
        for process_pid in included {
            let process = self.system.process(process_pid)?;
            let disk = process.disk_usage();
            snapshot.cpu_percent += process.cpu_usage();
            snapshot.memory_bytes = snapshot.memory_bytes.saturating_add(process.memory());
            snapshot.read_bytes = snapshot.read_bytes.saturating_add(disk.total_read_bytes);
            snapshot.written_bytes = snapshot
                .written_bytes
                .saturating_add(disk.total_written_bytes);
        }
        Some(snapshot)
    }
}

impl Default for SystemMonitor {
    fn default() -> Self {
        Self::new()
    }
}
