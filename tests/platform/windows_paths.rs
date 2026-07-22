//! Windows 路径展示与系统内建命令回归测试。

#![cfg(windows)]

use std::{
    fs,
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};

use procora::{
    config::discover_path,
    daemon::ServiceHost,
    platform::simplify_path,
    protocol::{SnapshotSourceDto, TaskStatusDto},
};
use uuid::Uuid;

/// 创建当前测试独占的临时服务目录。
fn temporary_service(label: &str) -> PathBuf {
    let directory =
        std::env::temp_dir().join(format!("procora-windows-{label}-{}", Uuid::new_v4()));
    fs::create_dir_all(&directory).unwrap();
    directory
}

#[test]
// 扩展驱动器与unc路径转换为常规展示路径。
fn extended_drive_and_unc_paths_are_simplified() {
    assert_eq!(
        simplify_path(std::path::Path::new(r"\\?\C:\Users\tester\service")),
        PathBuf::from(r"C:\Users\tester\service")
    );
    assert_eq!(
        simplify_path(std::path::Path::new(r"\\?\UNC\server\share\service")),
        PathBuf::from(r"\\server\share\service")
    );
}

#[test]
// 配置发现不会暴露windows扩展路径前缀。
fn config_discovery_hides_windows_verbatim_prefix() {
    let service = temporary_service("path");
    fs::write(
        service.join("procora.yaml"),
        "version: 1\nproject: windows-path\ntasks: {}\n",
    )
    .unwrap();

    let discovered = discover_path(&service).unwrap();
    assert!(!discovered.root.to_string_lossy().starts_with(r"\\?\"));
    assert!(
        !discovered
            .config_path
            .to_string_lossy()
            .starts_with(r"\\?\")
    );

    fs::remove_dir_all(service).unwrap();
}

#[test]
// echo内建命令可完成依赖任务图。
fn echo_builtin_completes_dependency_graph() {
    let service = temporary_service("echo");
    let config = service.join("procora.yaml");
    fs::write(
        &config,
        "version: 1\nproject: download\ntasks:\n  prepare:\n    command: echo\n    args: ['Preparing...']\n  app:\n    command: echo\n    args: ['Running app...']\n    depends_on:\n      prepare:\n        condition: completed_successfully\n",
    )
    .unwrap();
    let discovered = discover_path(&config).unwrap();
    let mut host = ServiceHost::from_compiled_at(discovered.compiled, &discovered.root);

    host.start().unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let snapshot = host.snapshot(SnapshotSourceDto::CenterLive, true);
        if snapshot
            .tasks
            .iter()
            .all(|task| task.status == TaskStatusDto::Stopped)
        {
            assert!(
                snapshot
                    .tasks
                    .iter()
                    .all(|task| task.message.as_deref() == Some("Task 已退出，退出码 0"))
            );
            break;
        }
        assert!(Instant::now() < deadline, "echo 任务图没有按时完成");
        thread::sleep(Duration::from_millis(10));
    }

    assert!(
        fs::read_to_string(service.join(".procora/logs/tasks/prepare.log"))
            .unwrap()
            .contains("Preparing...")
    );
    assert!(
        fs::read_to_string(service.join(".procora/logs/tasks/app.log"))
            .unwrap()
            .contains("Running app...")
    );
    host.stop().unwrap();
    fs::remove_dir_all(service).unwrap();
}
