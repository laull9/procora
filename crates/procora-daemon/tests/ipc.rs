//! 中心服务器本地 IPC 连续请求测试。

use std::{
    fs,
    path::{Path, PathBuf},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use procora_daemon::{CenterClient, IpcError, run_center_server};
use procora_protocol::{
    CenterRequest, CenterResponse, ServiceActionDto, ServiceSelectorDto, ServiceStatusDto,
};

/// 创建独占的端点名称和临时状态目录。
fn isolated_runtime() -> (String, PathBuf) {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let endpoint = format!("procora-ipc-test-{}-{nonce}", std::process::id());
    let directory = std::env::temp_dir().join(&endpoint);
    fs::create_dir_all(&directory).unwrap();
    (endpoint, directory)
}

/// 写入可供中心服务器发现的最小配置。
fn write_service(directory: &Path) {
    fs::write(
        directory.join("procora.yaml"),
        "version: 1\nproject: ipc-demo\ntasks:\n  task:\n    command: echo\n",
    )
    .unwrap();
}

#[test]
fn 同一中心服务器可以连续处理管理请求() {
    let (endpoint, directory) = isolated_runtime();
    write_service(&directory);
    let state = directory.join("procora.sqlite3");
    let server_endpoint = endpoint.clone();
    let server_state = state.clone();
    thread::spawn(move || {
        run_center_server(&server_endpoint, &server_state).unwrap();
    });

    let client = CenterClient::new(endpoint);
    for _ in 0..100 {
        if client.ping() {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    assert!(client.ping());
    let hello = client.hello("ipc-test").unwrap();
    assert_eq!(hello.service_count, 0);

    let duplicate = run_center_server("unused-endpoint", &state);
    assert!(matches!(duplicate, Err(IpcError::AlreadyRunning)));

    let opened = client
        .request(&CenterRequest::Open {
            path: directory.clone(),
        })
        .unwrap();
    assert!(matches!(
        opened,
        CenterResponse::Service(service) if service.status == ServiceStatusDto::Running
    ));

    let stopped = client
        .request(&CenterRequest::Manage {
            action: ServiceActionDto::Stop,
            selector: ServiceSelectorDto::Name("ipc-demo".to_owned()),
        })
        .unwrap();
    assert!(matches!(
        stopped,
        CenterResponse::Service(service) if service.status == ServiceStatusDto::Stopped
    ));

    let listed = client.request(&CenterRequest::List).unwrap();
    assert!(matches!(
        listed,
        CenterResponse::Services(services)
            if services.len() == 1 && services[0].status == ServiceStatusDto::Stopped
    ));
    assert!(matches!(
        client.request(&CenterRequest::Shutdown).unwrap(),
        CenterResponse::ShuttingDown
    ));
    for _ in 0..100 {
        if !client.ping() {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    assert!(!client.ping());
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn 后台中心在没有观察客户端时仍推进完成依赖() {
    let (endpoint, directory) = isolated_runtime();
    fs::write(
        directory.join("procora.yaml"),
        "version: 1\nproject: autonomous\ntasks:\n  prepare:\n    command: rustc\n    args: ['--version']\n  app:\n    command: rustc\n    args: ['--version']\n    depends_on:\n      prepare:\n        condition: completed_successfully\n",
    )
    .unwrap();
    let state = directory.join("procora.sqlite3");
    let server_endpoint = endpoint.clone();
    let server_state = state.clone();
    let server = thread::spawn(move || {
        run_center_server(&server_endpoint, &server_state).unwrap();
    });
    let client = CenterClient::new(endpoint);
    for _ in 0..100 {
        if client.ping() {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    assert!(matches!(
        client
            .request(&CenterRequest::Open {
                path: directory.clone(),
            })
            .unwrap(),
        CenterResponse::Service(_)
    ));

    let app_log = directory.join(".procora/logs/tasks/app.log");
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    while fs::metadata(&app_log).map_or(true, |metadata| metadata.len() == 0) {
        assert!(
            std::time::Instant::now() < deadline,
            "没有客户端轮询时 completed_successfully 下游未启动"
        );
        thread::sleep(Duration::from_millis(10));
    }

    assert!(matches!(
        client.request(&CenterRequest::Shutdown).unwrap(),
        CenterResponse::ShuttingDown
    ));
    server.join().unwrap();
    fs::remove_dir_all(directory).unwrap();
}
