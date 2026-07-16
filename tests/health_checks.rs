//! 健康检查配置、阈值和真实依赖门控测试。

use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use procora::{
    config::{ConfigFormat, load_str},
    daemon::ServiceHost,
    protocol::{SnapshotSourceDto, TaskHealthDto},
};
use serde_json::json;

/// 同进程并行测试的临时目录序号。
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// 创建健康检查测试独占目录。
fn temporary_directory() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("系统时间应晚于 Unix 纪元")
        .as_nanos();
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let directory = std::env::temp_dir().join(format!(
        "procora-health-{}-{nonce}-{sequence}",
        std::process::id()
    ));
    fs::create_dir_all(&directory).expect("应能创建测试目录");
    directory
}

/// 构造由当前测试二进制承担 Task 与检查程序的跨平台配置。
fn runtime_configuration(directory: &Path) -> String {
    let executable = std::env::current_exe().expect("应能读取当前测试二进制路径");
    let ready = directory.join("ready");
    let dependent = directory.join("dependent");
    json!({
        "version": 1,
        "project": "health-runtime",
        "tasks": {
            "server": {
                "command": executable,
                "args": ["--exact", "long_running_task_helper", "--nocapture"],
                "env": {
                    "PROCORA_HEALTH_TEST": "1",
                    "PROCORA_READY_FILE": ready,
                },
                "shutdown_timeout_ms": 500,
                "healthcheck": {
                    "command": executable,
                    "args": ["--exact", "health_check_helper", "--nocapture"],
                    "period_ms": 20,
                    "timeout_ms": 500,
                    "success_threshold": 2,
                    "failure_threshold": 2,
                }
            },
            "dependent": {
                "command": executable,
                "args": ["--exact", "dependent_task_helper", "--nocapture"],
                "env": {
                    "PROCORA_HEALTH_TEST": "1",
                    "PROCORA_DEPENDENT_FILE": dependent,
                },
                "depends_on": {
                    "server": { "condition": "healthy" }
                }
            }
        }
    })
    .to_string()
}

#[test]
// 健康检查配置应用默认值并保留参数数组。
fn health_check_defaults_preserve_argument_array() {
    let compiled = load_str(
        "version: 1\nproject: demo\ntasks:\n  api:\n    command: api\n    healthcheck:\n      command: checker\n      args: ['--ready']\n",
        ConfigFormat::Yaml,
    )
    .expect("有效检查应通过配置编译");
    let healthcheck = compiled
        .spec
        .tasks
        .values()
        .next()
        .unwrap()
        .healthcheck
        .as_ref()
        .unwrap();

    assert_eq!(healthcheck.command, "checker");
    assert_eq!(healthcheck.args, ["--ready"]);
    assert_eq!(healthcheck.period_ms, 10_000);
    assert_eq!(healthcheck.timeout_ms, 1_000);
    assert_eq!(healthcheck.success_threshold, 1);
    assert_eq!(healthcheck.failure_threshold, 3);
}

#[test]
// 健康检查拒绝无界时间和阈值。
fn health_checks_reject_unbounded_limits() {
    let error = load_str(
        "version: 1\nproject: demo\ntasks:\n  api:\n    command: api\n    healthcheck:\n      command: checker\n      period_ms: 0\n      timeout_ms: 300001\n      success_threshold: 0\n      failure_threshold: 101\n",
        ConfigFormat::Yaml,
    )
    .expect_err("无界检查配置必须被拒绝")
    .to_string();

    for field in [
        "healthcheck.period_ms",
        "healthcheck.timeout_ms",
        "healthcheck.success_threshold",
        "healthcheck.failure_threshold",
    ] {
        assert!(error.contains(field), "缺少字段诊断：{field}: {error}");
    }
}

#[test]
// 连续健康后才启动依赖任务。
fn dependent_task_starts_after_consecutive_health_successes() {
    let directory = temporary_directory();
    let compiled =
        load_str(&runtime_configuration(&directory), ConfigFormat::Json).expect("运行配置应有效");
    let mut host = ServiceHost::from_compiled_at(compiled, &directory);
    host.start().expect("服务应能启动");

    let dependent = directory.join("dependent");
    let deadline = Instant::now() + Duration::from_secs(5);
    let snapshot = loop {
        let snapshot = host.snapshot(SnapshotSourceDto::CenterLive, true);
        if dependent.exists() {
            break snapshot;
        }
        assert!(Instant::now() < deadline, "健康依赖没有在期限内放行");
        thread::sleep(Duration::from_millis(10));
    };
    let server = snapshot
        .tasks
        .iter()
        .find(|task| task.task_id.as_str() == "server")
        .expect("应包含 server Task");
    assert_eq!(server.health, TaskHealthDto::Healthy);

    host.stop().expect("服务应能停止并取消检查");
    fs::remove_dir_all(directory).expect("应能清理测试目录");
}

/// 被真实 Task 子进程调用：延迟创建就绪文件并保持运行。
#[test]
// 长期任务辅助进程。
fn long_running_task_helper() {
    if std::env::var_os("PROCORA_HEALTH_TEST").is_none() {
        return;
    }
    thread::sleep(Duration::from_millis(120));
    fs::write(
        std::env::var_os("PROCORA_READY_FILE").expect("应传入就绪文件"),
        b"ready",
    )
    .expect("应能写入就绪文件");
    thread::sleep(Duration::from_secs(2));
}

/// 被健康检查子进程调用：以文件是否出现决定退出状态。
#[test]
// 健康检查辅助进程。
fn health_check_helper() {
    if std::env::var_os("PROCORA_HEALTH_TEST").is_none() {
        return;
    }
    let ready = std::env::var_os("PROCORA_READY_FILE").expect("应传入就绪文件");
    assert!(Path::new(&ready).exists(), "服务尚未就绪");
}

/// 被下游 Task 子进程调用：记录依赖已经放行。
#[test]
// 依赖任务辅助进程。
fn dependent_task_helper() {
    if std::env::var_os("PROCORA_HEALTH_TEST").is_none() {
        return;
    }
    fs::write(
        std::env::var_os("PROCORA_DEPENDENT_FILE").expect("应传入下游标记文件"),
        b"started",
    )
    .expect("应能写入下游标记文件");
}
