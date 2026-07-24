//! `ServiceHost` 真实调度、输出日志、资源与停止闭环测试。

use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use procora::config::{ConfigFormat, load_str};
use procora::daemon::ServiceHost;
use procora::protocol::{SnapshotSourceDto, TaskStatusDto};

/// 同一进程并行测试使用的临时目录序号。
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// 创建当前测试独占的服务目录。
fn temporary_service() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let directory = std::env::temp_dir().join(format!(
        "procora-host-{}-{nonce}-{sequence}",
        std::process::id()
    ));
    fs::create_dir_all(&directory).unwrap();
    directory
}

#[cfg(unix)]
#[test]
// 未声明cwd的Task始终以包含非ASCII字符的service根目录运行。
fn task_without_cwd_runs_from_unicode_service_root() {
    let parent = temporary_service();
    let service = parent.join("下载 目录");
    fs::create_dir_all(&service).unwrap();
    let compiled = load_str(
        "version: 1\nproject: runtime\ntasks:\n  pwd:\n    command: pwd\n",
        ConfigFormat::Yaml,
    )
    .unwrap();
    let mut host = ServiceHost::from_compiled_at(compiled, &service);

    host.start().unwrap();
    wait_until_stopped(&mut host);

    let output = fs::read_to_string(service.join(".procora/logs/tasks/pwd.log")).unwrap();
    assert_eq!(
        fs::canonicalize(output.trim()).unwrap(),
        fs::canonicalize(&service).unwrap()
    );
    fs::remove_dir_all(parent).unwrap();
}

#[cfg(unix)]
#[test]
// Task显式cwd优先于service根目录默认值。
fn explicit_task_cwd_overrides_service_root() {
    let service = temporary_service();
    let working_directory = service.join("明确工作目录");
    fs::create_dir_all(&working_directory).unwrap();
    let configuration = serde_json::json!({
        "version": 1,
        "project": "runtime",
        "tasks": {
            "pwd": {
                "command": "pwd",
                "cwd": working_directory,
            }
        }
    })
    .to_string();
    let compiled = load_str(&configuration, ConfigFormat::Json).unwrap();
    let mut host = ServiceHost::from_compiled_at(compiled, &service);

    host.start().unwrap();
    wait_until_stopped(&mut host);

    let output = fs::read_to_string(service.join(".procora/logs/tasks/pwd.log")).unwrap();
    assert_eq!(
        fs::canonicalize(output.trim()).unwrap(),
        fs::canonicalize(&working_directory).unwrap()
    );
    fs::remove_dir_all(service).unwrap();
}

/// 等待一次性 Task 全部退出并刷新日志。
fn wait_until_stopped(host: &mut ServiceHost) {
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    loop {
        let snapshot = host.snapshot(SnapshotSourceDto::CenterLive, true);
        if snapshot
            .tasks
            .iter()
            .all(|task| task.status == TaskStatusDto::Stopped)
        {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "Task 没有在期限内退出"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
// completed依赖会运行真实进程并写入service本地日志。
fn completed_dependency_runs_real_process_and_writes_logs() {
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
    assert!(
        snapshot
            .tasks
            .iter()
            .all(|task| { task.message.as_deref() == Some("Task 已退出，退出码 0") })
    );
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
// 可重试的创建失败不会击穿service宿主。
fn retryable_spawn_failure_does_not_break_service_host() {
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
// stop会终止长时间任务并排空最后输出。
fn stop_terminates_long_task_and_drains_output() {
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
// 顶层进程退出时继承管道的后台后代不会阻塞宿主。
fn inherited_pipe_descendant_does_not_block_host_exit() {
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
// on_failure会在退避后创建新run。
fn on_failure_creates_new_run_after_backoff() {
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

#[cfg(unix)]
#[test]
// max_restarts会停止真实失败进程的持续创建风暴。
fn max_restarts_stops_real_spawn_storm() {
    let service = temporary_service();
    let counter = service.join("bounded-runs.txt");
    let configuration = format!(
        "{{\"version\":1,\"project\":\"runtime\",\"tasks\":{{\"app\":{{\"command\":\"sh\",\"args\":[\"-c\",\"printf run\\\\n >> \\\"$1\\\"; exit 1\",\"procora-test\",{}],\"restart\":\"on-failure\",\"restart_delay_ms\":10,\"max_restarts\":2,\"restart_reset_after_ms\":0}}}}}}",
        serde_json::to_string(&counter).unwrap()
    );
    let compiled = load_str(&configuration, ConfigFormat::Json).unwrap();
    let mut host = ServiceHost::from_compiled_at(compiled, &service);
    host.start().unwrap();

    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    loop {
        let snapshot = host.snapshot(SnapshotSourceDto::CenterLive, true);
        if snapshot.tasks[0].status == TaskStatusDto::Failed {
            assert!(
                snapshot.tasks[0]
                    .message
                    .as_deref()
                    .is_some_and(|message| message.contains("2 次自动重启上限"))
            );
            break;
        }
        assert!(std::time::Instant::now() < deadline, "任务没有停止重启");
        thread::sleep(Duration::from_millis(10));
    }
    let runs = fs::read_to_string(&counter).unwrap();
    assert_eq!(runs.matches("run").count(), 3);
    thread::sleep(Duration::from_millis(80));
    host.refresh();
    assert_eq!(
        fs::read_to_string(&counter).unwrap().matches("run").count(),
        3
    );

    host.stop().unwrap();
    fs::remove_dir_all(service).unwrap();
}
