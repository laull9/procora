//! 自定义成功退出码的配置与真实调度测试。

use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use procora::{
    config::{ConfigFormat, load_str},
    daemon::ServiceHost,
};
use serde_json::json;

/// 同进程并行测试的临时目录序号。
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// 创建成功退出码测试独占目录。
fn temporary_directory() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("系统时间应晚于 Unix 纪元")
        .as_nanos();
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let directory = std::env::temp_dir().join(format!(
        "procora-exit-code-{}-{nonce}-{sequence}",
        std::process::id()
    ));
    fs::create_dir_all(&directory).expect("应能创建测试目录");
    directory
}

#[test]
fn 自定义成功退出码始终包含零且拒绝负数() {
    let compiled = load_str(
        "version: 1\nproject: demo\ntasks:\n  task:\n    command: task\n    success_exit_codes: [130, 130]\n",
        ConfigFormat::Yaml,
    )
    .expect("非负退出码应有效");
    let codes = &compiled
        .spec
        .tasks
        .values()
        .next()
        .unwrap()
        .success_exit_codes;
    assert_eq!(codes.iter().copied().collect::<Vec<_>>(), [0, 130]);

    let error = load_str(
        "version: 1\nproject: demo\ntasks:\n  task:\n    command: task\n    success_exit_codes: [-1]\n",
        ConfigFormat::Yaml,
    )
    .expect_err("负退出码必须被拒绝")
    .to_string();
    assert!(error.contains("success_exit_codes"));
}

#[test]
fn 自定义成功退出码会放行完成依赖() {
    let directory = temporary_directory();
    let executable = std::env::current_exe().expect("应能读取测试二进制路径");
    let marker = directory.join("dependent");
    let configuration = json!({
        "version": 1,
        "project": "exit-code-runtime",
        "tasks": {
            "prepare": {
                "command": executable,
                "args": ["--exact", "非零退出辅助进程", "--nocapture"],
                "env": { "PROCORA_EXIT_CODE_HELPER": "1" },
                "success_exit_codes": [42]
            },
            "dependent": {
                "command": executable,
                "args": ["--exact", "完成依赖辅助进程", "--nocapture"],
                "env": {
                    "PROCORA_DEPENDENT_HELPER": "1",
                    "PROCORA_DEPENDENT_FILE": marker,
                },
                "depends_on": {
                    "prepare": { "condition": "completed_successfully" }
                }
            }
        }
    })
    .to_string();
    let compiled = load_str(&configuration, ConfigFormat::Json).expect("运行配置应有效");
    let mut host = ServiceHost::from_compiled_at(compiled, &directory);
    host.start().expect("服务应能启动");

    let marker = directory.join("dependent");
    let deadline = Instant::now() + Duration::from_secs(5);
    while !marker.exists() {
        host.refresh();
        assert!(Instant::now() < deadline, "自定义成功退出码没有放行依赖");
        thread::sleep(Duration::from_millis(10));
    }

    host.stop().expect("服务应能停止");
    fs::remove_dir_all(directory).expect("应能清理测试目录");
}

/// 被 Task 子进程调用：以声明为成功的非零退出码结束。
#[test]
fn 非零退出辅助进程() {
    if std::env::var_os("PROCORA_EXIT_CODE_HELPER").is_some() {
        std::process::exit(42);
    }
}

/// 被下游 Task 子进程调用：记录完成依赖已经放行。
#[test]
fn 完成依赖辅助进程() {
    if std::env::var_os("PROCORA_DEPENDENT_HELPER").is_none() {
        return;
    }
    fs::write(
        std::env::var_os("PROCORA_DEPENDENT_FILE").expect("应传入下游标记文件"),
        b"started",
    )
    .expect("应能写入下游标记文件");
}
