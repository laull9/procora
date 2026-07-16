//! 中心服务器本地 IPC 连续请求测试。

use std::{
    fs,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[cfg(windows)]
use interprocess::local_socket::Stream;
use interprocess::local_socket::{GenericNamespaced, ListenerOptions, prelude::*};
use procora::daemon::{CenterClient, IpcError, run_center_server};
use procora::protocol::{
    CenterRequest, CenterResponse, ClientHello, PROTOCOL_VERSION, ServiceActionDto,
    ServiceSelectorDto, ServiceStatusDto,
};

/// 同一进程并行测试使用的临时端点去重序列。
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// 创建独占的端点名称和临时状态目录。
fn isolated_runtime() -> (String, PathBuf) {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let endpoint = format!("procora-ipc-test-{}-{nonce}-{sequence}", std::process::id());
    let directory = std::env::temp_dir().join(&endpoint);
    fs::create_dir_all(&directory).unwrap();
    (endpoint, directory)
}

/// 写入可供中心服务器发现的最小配置。
fn write_service(directory: &Path) {
    fs::write(
        directory.join("procora.yaml"),
        format!(
            "version: 1\nproject: ipc-demo\ntasks:\n  task:\n    {}\n",
            long_running_task(),
        ),
    )
    .unwrap();
}

/// 返回当前目标系统可长期运行且可由测试显式停止的任务配置片段。
fn long_running_task() -> &'static str {
    #[cfg(unix)]
    {
        "command: sh\n    args: ['-c', 'sleep 30']"
    }
    #[cfg(windows)]
    {
        "command: cmd.exe\n    args: ['/C', 'ping -n 30 127.0.0.1 > NUL']"
    }
    #[cfg(not(any(unix, windows)))]
    {
        "command: rustc\n    args: ['--version']"
    }
}

/// 每个平台的本地 IPC 都必须拒绝不兼容的协议主版本。
#[test]
fn 本地ipc拒绝不兼容协议版本() {
    let (endpoint, directory) = isolated_runtime();
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

    let response = client
        .request(&CenterRequest::Hello(ClientHello {
            protocol_version: PROTOCOL_VERSION + 1,
            client_name: "incompatible-test".to_owned(),
        }))
        .unwrap();
    assert!(matches!(response, CenterResponse::Error { message } if message.contains("不兼容")));
    assert!(matches!(
        client.request(&CenterRequest::Shutdown).unwrap(),
        CenterResponse::ShuttingDown
    ));
    server.join().unwrap();
    fs::remove_dir_all(directory).unwrap();
}

/// 无响应的本地中心不能永久阻塞客户端命令。
#[test]
fn 无响应中心请求会在期限内返回() {
    let (endpoint, directory) = isolated_runtime();
    let name = endpoint.clone().to_ns_name::<GenericNamespaced>().unwrap();
    let listener = ListenerOptions::new()
        .name(name)
        .try_overwrite(true)
        .create_sync()
        .unwrap();
    let server = thread::spawn(move || {
        let connection = listener.accept().unwrap();
        let mut connection = BufReader::new(connection);
        let mut request = String::new();
        connection.read_line(&mut request).unwrap();
        thread::sleep(Duration::from_secs(3));
    });

    let started = std::time::Instant::now();
    let result = CenterClient::new(endpoint).request(&CenterRequest::Ping);
    assert!(matches!(
        result,
        Err(IpcError::Io(error)) if error.kind() == std::io::ErrorKind::TimedOut
    ));
    assert!(started.elapsed() < Duration::from_secs(3));

    server.join().unwrap();
    fs::remove_dir_all(directory).unwrap();
}

/// Windows 管道没有空闲实例时，连接必须在期限内失败而不能无限等待。
#[cfg(windows)]
#[test]
fn windows繁忙中心连接会在期限内返回() {
    let (endpoint, directory) = isolated_runtime();
    let name = endpoint.clone().to_ns_name::<GenericNamespaced>().unwrap();
    let listener = ListenerOptions::new()
        .name(name)
        .try_overwrite(true)
        .create_sync()
        .unwrap();
    let occupied_name = endpoint.clone().to_ns_name::<GenericNamespaced>().unwrap();
    let occupied = Stream::connect(occupied_name).unwrap();

    let started = std::time::Instant::now();
    let result = CenterClient::new(endpoint).request(&CenterRequest::Ping);
    assert!(matches!(
        result,
        Err(IpcError::Io(error)) if error.kind() == std::io::ErrorKind::TimedOut
    ));
    assert!(started.elapsed() < Duration::from_secs(3));

    drop(occupied);
    drop(listener);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn 同一中心服务器可以连续处理管理请求() {
    let (endpoint, directory) = isolated_runtime();
    write_service(&directory);
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
    assert!(client.ping());
    let hello = client.hello("ipc-test").unwrap();
    assert_eq!(hello.service_count, 0);
    assert_eq!(hello.procora_version, env!("CARGO_PKG_VERSION"));
    assert!(hello.uses_current_version());

    let duplicate = run_center_server("unused-endpoint", &state);
    assert!(
        matches!(duplicate, Err(IpcError::AlreadyRunning)),
        "重复启动返回了意外结果: {duplicate:?}"
    );

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
    server.join().unwrap();
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
