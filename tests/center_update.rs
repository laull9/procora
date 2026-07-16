//! 中心服务器版本探测与自动替换测试。

use std::{
    collections::hash_map::DefaultHasher,
    fs,
    hash::{Hash, Hasher},
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Command, Output},
    thread,
    time::{Duration, Instant},
};

use interprocess::local_socket::{GenericNamespaced, ListenerOptions, prelude::*};
use procora::protocol::{CenterHello, CenterRequest, CenterResponse, PROTOCOL_VERSION};
use uuid::Uuid;

#[path = "support/command.rs"]
mod command_support;

use command_support::{remove_directory_when_released, run_background_cli};

/// 创建测试独占的中心数据目录。
fn temporary_home() -> PathBuf {
    let home = std::env::temp_dir().join(format!("procora-update-{}", Uuid::new_v4()));
    fs::create_dir_all(&home).unwrap();
    home
}

/// 按正式运行时代码计算指定中心目录的 IPC 端点。
fn endpoint_for(home: &Path) -> String {
    let mut hasher = DefaultHasher::new();
    home.hash(&mut hasher);
    format!("procora-center-{:016x}", hasher.finish())
}

/// 运行一个只够完成版本探测和正常关闭的旧版本中心替身。
fn spawn_outdated_center(endpoint: &str) -> thread::JoinHandle<()> {
    let name = endpoint
        .to_owned()
        .to_ns_name::<GenericNamespaced>()
        .unwrap();
    let listener = ListenerOptions::new()
        .name(name)
        .try_overwrite(true)
        .create_sync()
        .unwrap();
    thread::spawn(move || {
        for expected in ["ping", "hello", "shutdown"] {
            let (request, mut connection) = loop {
                let connection = listener.accept().unwrap();
                let mut connection = BufReader::new(connection);
                let mut line = String::new();
                if connection.read_line(&mut line).unwrap() == 0 {
                    continue;
                }
                let request: CenterRequest = serde_json::from_str(&line).unwrap();
                break (request, connection);
            };
            let response = match (expected, request) {
                ("ping", CenterRequest::Ping) => CenterResponse::Pong,
                ("hello", CenterRequest::Hello(_)) => CenterResponse::Hello(CenterHello {
                    protocol_version: PROTOCOL_VERSION,
                    procora_version: "0.0.0-outdated".to_owned(),
                    instance_id: Uuid::new_v4(),
                    service_count: 0,
                    event_sequence: 0,
                    control_allowed: true,
                }),
                ("shutdown", CenterRequest::Shutdown) => CenterResponse::ShuttingDown,
                (_, request) => panic!("旧中心收到意外请求: {request:?}"),
            };
            serde_json::to_writer(&mut *connection.get_mut(), &response).unwrap();
            connection.get_mut().write_all(b"\n").unwrap();
            connection.get_mut().flush().unwrap();
        }
    })
}

/// 在硬期限内执行 CLI，避免平台集成错误永久挂住测试进程。
fn run_cli(binary: &str, home: &Path, argument: &str) -> Output {
    run_background_cli(
        Command::new(binary).arg(argument).env("PROCORA_HOME", home),
        home,
        argument,
    )
}

/// 在硬期限内等待旧中心替身结束，并返回其线程结果。
fn join_outdated_center(handle: thread::JoinHandle<()>) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !handle.is_finished() {
        assert!(Instant::now() < deadline, "旧中心替身未在 5 秒内退出");
        thread::sleep(Duration::from_millis(20));
    }
    handle.join().unwrap();
}

#[test]
// status发现旧版本后自动替换为当前版本。
fn status_replaces_legacy_center_with_current_version() {
    let home = temporary_home();
    let endpoint = endpoint_for(&home);
    let outdated = spawn_outdated_center(&endpoint);
    let binary = env!("CARGO_BIN_EXE_procora");

    let status = run_cli(binary, &home, "status");
    assert!(
        status.status.success(),
        "{}",
        String::from_utf8_lossy(&status.stderr)
    );
    let stdout = String::from_utf8_lossy(&status.stdout);
    assert!(stdout.contains("全局 Procora：运行中"));
    assert!(stdout.contains(&format!("版本：{}", env!("CARGO_PKG_VERSION"))));
    join_outdated_center(outdated);

    let down = run_cli(binary, &home, "down");
    assert!(down.status.success());
    remove_directory_when_released(&home);
}
