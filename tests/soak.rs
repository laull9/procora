//! 定时 CI 使用的真实 Task 生命周期长期循环与句柄泄漏门禁。

use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use procora::{
    config::load_path,
    daemon::ServiceHost,
    protocol::{SnapshotSourceDto, TaskStatusDto},
};

/// 当前测试进程内的临时目录去重序列。
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// 创建当前测试独占的临时目录。
fn temporary_directory() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let directory = std::env::temp_dir().join(format!(
        "procora-soak-{}-{nonce}-{sequence}",
        std::process::id()
    ));
    fs::create_dir_all(&directory).unwrap();
    directory
}

#[test]
#[ignore = "由 weekly soak workflow 以 release 模式运行"]
fn 高频真实task启停保持状态闭环且不持续泄漏句柄() {
    let root = temporary_directory();
    let config = root.join("procora.yaml");
    fs::write(&config, long_running_config()).unwrap();
    let compiled = load_path(&config).unwrap();
    let mut host = ServiceHost::from_compiled_at(compiled, &root);

    cycle(&mut host);
    let baseline_descriptors = descriptor_count();
    let cycles = soak_cycles();
    for _ in 0..cycles {
        cycle(&mut host);
    }
    let final_descriptors = descriptor_count();

    if let (Some(baseline), Some(final_count)) = (baseline_descriptors, final_descriptors) {
        assert!(
            final_count <= baseline + 8,
            "{cycles} 次启停后文件描述符从 {baseline} 增长到 {final_count}"
        );
    }
    drop(host);
    fs::remove_dir_all(root).unwrap();
}

/// 完成一次真实进程启动、运行状态观察和反向停止。
fn cycle(host: &mut ServiceHost) {
    host.start().unwrap();
    let running = host.snapshot(SnapshotSourceDto::EmbeddedLive, true);
    assert_eq!(running.tasks.len(), 1);
    assert_eq!(running.tasks[0].status, TaskStatusDto::Running);
    host.stop().unwrap();
    let stopped = host.snapshot(SnapshotSourceDto::EmbeddedLive, false);
    assert_eq!(stopped.tasks[0].status, TaskStatusDto::Stopped);
}

/// 返回定时任务指定的循环数，并限制误配置造成的无界运行。
fn soak_cycles() -> usize {
    std::env::var("PROCORA_SOAK_CYCLES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(256)
        .clamp(1, 10_000)
}

/// 返回跨平台可被立即停止的单 Task 配置。
fn long_running_config() -> &'static str {
    #[cfg(unix)]
    {
        "version: 1\nproject: soak\ntasks:\n  worker:\n    command: sh\n    args: ['-c', 'sleep 30']\n    shutdown_timeout_ms: 100\n"
    }
    #[cfg(windows)]
    {
        "version: 1\nproject: soak\ntasks:\n  worker:\n    command: cmd.exe\n    args: ['/C', 'ping -n 30 127.0.0.1 > NUL']\n    shutdown_timeout_ms: 100\n"
    }
    #[cfg(not(any(unix, windows)))]
    {
        "version: 1\nproject: soak\ntasks:\n  worker:\n    command: rustc\n    args: ['--version']\n"
    }
}

/// 在 Unix 返回当前进程打开的文件描述符数量，其他平台跳过该断言。
fn descriptor_count() -> Option<usize> {
    #[cfg(unix)]
    {
        [Path::new("/proc/self/fd"), Path::new("/dev/fd")]
            .into_iter()
            .find_map(|path| fs::read_dir(path).ok().map(Iterator::count))
    }
    #[cfg(not(unix))]
    {
        None
    }
}
