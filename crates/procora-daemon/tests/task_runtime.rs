//! `ServiceHost` 真实调度、输出日志、资源与停止闭环测试。

use std::{
    fs,
    path::PathBuf,
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use procora_config::{ConfigFormat, load_str};
use procora_daemon::ServiceHost;
use procora_protocol::{SnapshotSourceDto, TaskStatusDto};

/// 创建当前测试独占的服务目录。
fn temporary_service() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory =
        std::env::temp_dir().join(format!("procora-host-{}-{nonce}", std::process::id()));
    fs::create_dir_all(&directory).unwrap();
    directory
}

#[test]
fn completed依赖会运行真实进程并写入service本地日志() {
    let service = temporary_service();
    let compiled = load_str(
        "version: 1\nproject: runtime\ntasks:\n  prepare:\n    command: rustc\n    args: ['--version']\n  app:\n    command: rustc\n    args: ['--version']\n    depends_on:\n      prepare:\n        condition: completed_successfully\n",
        ConfigFormat::Yaml,
    )
    .unwrap();
    let mut host = ServiceHost::from_compiled_at(compiled, &service);
    host.start().unwrap();

    let snapshot = loop {
        let snapshot = host.snapshot(SnapshotSourceDto::CenterLive, true);
        if snapshot
            .tasks
            .iter()
            .all(|task| task.status == TaskStatusDto::Stopped)
        {
            break snapshot;
        }
        thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(snapshot.tasks.len(), 2);
    for task in ["prepare", "app"] {
        let content = fs::read_to_string(
            service
                .join(".procora/logs/tasks")
                .join(format!("{task}.log")),
        )
        .unwrap();
        assert!(content.starts_with("rustc "));
    }
    fs::remove_dir_all(service).unwrap();
}

#[test]
fn 可重试的创建失败不会击穿service宿主() {
    let service = temporary_service();
    let compiled = load_str(
        "version: 1\nproject: runtime\ntasks:\n  app:\n    command: procora-command-that-does-not-exist\n    restart: on-failure\n    restart_delay_ms: 10\n",
        ConfigFormat::Yaml,
    )
    .unwrap();
    let mut host = ServiceHost::from_compiled_at(compiled, &service);

    host.start().unwrap();
    host.stop().unwrap();

    fs::remove_dir_all(service).unwrap();
}

#[cfg(unix)]
#[test]
fn stop会终止长时间任务并排空最后输出() {
    let service = temporary_service();
    let compiled = load_str(
        "version: 1\nproject: runtime\ntasks:\n  app:\n    command: sh\n    args: ['-c', 'printf started; sleep 30']\n    shutdown_timeout_ms: 100\n",
        ConfigFormat::Yaml,
    )
    .unwrap();
    let mut host = ServiceHost::from_compiled_at(compiled, &service);
    host.start().unwrap();
    thread::sleep(Duration::from_millis(30));
    let running = host.snapshot(SnapshotSourceDto::CenterLive, true);
    assert_eq!(running.tasks[0].status, TaskStatusDto::Running);
    assert!(running.tasks[0].resources.is_some());

    host.stop().unwrap();

    let stopped = host.snapshot(SnapshotSourceDto::CenterLive, false);
    assert_eq!(stopped.tasks[0].status, TaskStatusDto::Stopped);
    let content = fs::read_to_string(service.join(".procora/logs/tasks/app.log")).unwrap();
    assert_eq!(content, "started");
    fs::remove_dir_all(service).unwrap();
}

#[cfg(unix)]
#[test]
fn 顶层进程退出时继承管道的后台后代不会阻塞宿主() {
    let service = temporary_service();
    let compiled = load_str(
        "version: 1\nproject: runtime\ntasks:\n  app:\n    command: sh\n    args: ['-c', 'sleep 30 &']\n",
        ConfigFormat::Yaml,
    )
    .unwrap();
    let mut host = ServiceHost::from_compiled_at(compiled, &service);
    host.start().unwrap();

    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    loop {
        let snapshot = host.snapshot(SnapshotSourceDto::CenterLive, true);
        if snapshot.tasks[0].status == TaskStatusDto::Stopped {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "继承 stdout/stderr 的后台后代阻塞了退出处理"
        );
        thread::sleep(Duration::from_millis(10));
    }

    host.stop().unwrap();
    fs::remove_dir_all(service).unwrap();
}

#[cfg(unix)]
#[test]
fn on_failure会在退避后创建新run() {
    let service = temporary_service();
    let counter = service.join("runs.txt");
    let configuration = format!(
        "{{\"version\":1,\"project\":\"runtime\",\"tasks\":{{\"app\":{{\"command\":\"sh\",\"args\":[\"-c\",\"printf run\\\\n >> \\\"$1\\\"; exit 1\",\"procora-test\",{}],\"restart\":\"on-failure\",\"restart_delay_ms\":10}}}}}}",
        serde_json::to_string(&counter).unwrap()
    );
    let compiled = load_str(&configuration, ConfigFormat::Json).unwrap();
    let mut host = ServiceHost::from_compiled_at(compiled, &service);
    host.start().unwrap();

    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    loop {
        host.refresh();
        let runs = fs::read_to_string(&counter).unwrap_or_default();
        if runs.matches("run").count() >= 2 {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "任务没有按策略重启");
        thread::sleep(Duration::from_millis(10));
    }

    host.stop().unwrap();
    fs::remove_dir_all(service).unwrap();
}
