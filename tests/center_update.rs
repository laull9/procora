//! 中心服务器版本探测与自动替换测试。

use std::{
    collections::hash_map::DefaultHasher,
    fs,
    hash::{Hash, Hasher},
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::Command,
    thread,
};

use interprocess::local_socket::{GenericNamespaced, ListenerOptions, prelude::*};
use procora::protocol::{CenterHello, CenterRequest, CenterResponse, PROTOCOL_VERSION};
use uuid::Uuid;

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
            let connection = listener.accept().unwrap();
            let mut connection = BufReader::new(connection);
            let mut line = String::new();
            connection.read_line(&mut line).unwrap();
            let request: CenterRequest = serde_json::from_str(&line).unwrap();
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

#[test]
fn status发现旧版本后自动替换为当前版本() {
    let home = temporary_home();
    let outdated = spawn_outdated_center(&endpoint_for(&home));
    let binary = env!("CARGO_BIN_EXE_procora");

    let status = Command::new(binary)
        .arg("status")
        .env("PROCORA_HOME", &home)
        .output()
        .unwrap();
    assert!(
        status.status.success(),
        "{}",
        String::from_utf8_lossy(&status.stderr)
    );
    let stdout = String::from_utf8_lossy(&status.stdout);
    assert!(stdout.contains("全局 Procora：运行中"));
    assert!(stdout.contains(&format!("版本：{}", env!("CARGO_PKG_VERSION"))));
    outdated.join().unwrap();

    let down = Command::new(binary)
        .arg("down")
        .env("PROCORA_HOME", &home)
        .output()
        .unwrap();
    assert!(down.status.success());
    fs::remove_dir_all(home).unwrap();
}
