//! 受管进程树资源聚合测试。

use procora::monitor::SystemMonitor;

#[test]
// 当前进程可以作为资源树根节点采样。
fn current_process_can_be_sampled_as_resource_root() {
    let mut monitor = SystemMonitor::new();
    let pid = std::process::id();

    let tree = monitor.snapshot_tree(pid).expect("当前测试进程应存在");

    assert_eq!(tree.pid, pid);
    assert!(tree.memory_bytes > 0);
}

#[test]
// 不存在的根进程返回不可用。
fn missing_root_process_returns_unavailable() {
    let mut monitor = SystemMonitor::new();

    assert!(monitor.snapshot_tree(u32::MAX).is_none());
}
