//! 中心服务器注册、定位、生命周期与恢复测试。

use std::{
    fs,
    path::{Path, PathBuf},
    str::FromStr,
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::Duration,
    time::{SystemTime, UNIX_EPOCH},
};

use procora::core::TaskId;
use procora::daemon::Center;
use procora::protocol::{
    CenterRequest, CenterResponse, ClientHello, PROTOCOL_VERSION, ServiceActionDto,
    ServiceSelectorDto, ServiceStatusDto, TaskStatusDto,
};
use procora::storage::SqliteCenterRepository;

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
        "procora-center-{}-{nonce}-{sequence}",
        std::process::id()
    ));
    fs::create_dir_all(&directory).unwrap();
    directory
}

#[test]
fn 握手增量事件和状态历史形成一致会话() {
    let directory = temporary_directory();
    let service_root = directory.join("demo");
    write_service(&service_root, "demo");
    let mut center = Center::empty(SqliteCenterRepository::new(
        directory.join("procora.sqlite3"),
    ));
    let CenterResponse::Hello(hello) = center.handle(CenterRequest::Hello(ClientHello {
        protocol_version: PROTOCOL_VERSION,
        client_name: "test".to_owned(),
    })) else {
        panic!("应返回握手信息");
    };
    assert!(hello.control_allowed);
    assert_eq!(hello.event_sequence, 0);

    center.handle(CenterRequest::Open { path: service_root });
    let CenterResponse::Events(events) = center.handle(CenterRequest::Events {
        after_sequence: hello.event_sequence,
    }) else {
        panic!("应返回增量事件");
    };
    assert!(!events.events.is_empty());
    assert_eq!(events.next_sequence, events.events.last().unwrap().sequence);
    assert!(!events.resync_required);

    let CenterResponse::History(history) = center.handle(CenterRequest::History {
        selector: ServiceSelectorDto::Name("demo".to_owned()),
    }) else {
        panic!("应返回状态历史");
    };
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].status, ServiceStatusDto::Running);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn 过期事件游标要求客户端重新获取快照() {
    let directory = temporary_directory();
    let service_root = directory.join("demo");
    write_service(&service_root, "demo");
    let mut center = Center::empty(SqliteCenterRepository::new(
        directory.join("procora.sqlite3"),
    ));
    center.handle(CenterRequest::Open { path: service_root });
    for index in 0..260 {
        let action = if index % 2 == 0 {
            ServiceActionDto::Stop
        } else {
            ServiceActionDto::Start
        };
        center.handle(CenterRequest::Manage {
            action,
            selector: ServiceSelectorDto::Name("demo".to_owned()),
        });
    }

    let CenterResponse::Events(batch) = center.handle(CenterRequest::Events { after_sequence: 0 })
    else {
        panic!("应返回事件批次");
    };
    assert!(batch.resync_required);
    assert!(batch.events.is_empty());
    fs::remove_dir_all(directory).unwrap();
}

/// 在目录中写入两任务服务配置。
fn write_service(root: &Path, name: &str) {
    fs::create_dir_all(root).unwrap();
    fs::write(
        root.join("procora.yaml"),
        format!(
            "version: 1\nproject: {name}\ntasks:\n  first:\n    {}\n  second:\n    {}\n    depends_on:\n      first: {{}}\n",
            long_running_task(),
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

#[test]
fn 可以按名称和路径管理同一个服务() {
    let directory = temporary_directory();
    let service_root = directory.join("demo");
    write_service(&service_root, "demo");
    let repository = SqliteCenterRepository::new(directory.join("procora.sqlite3"));
    let mut center = Center::empty(repository.clone());

    let opened = center.handle(CenterRequest::Open {
        path: service_root.clone(),
    });
    let CenterResponse::Service(opened) = opened else {
        panic!("应返回服务摘要");
    };
    assert_eq!(opened.status, ServiceStatusDto::Running);
    assert_eq!(opened.task_count, 2);

    let stopped = center.handle(CenterRequest::Manage {
        action: ServiceActionDto::Stop,
        selector: ServiceSelectorDto::Name("demo".to_owned()),
    });
    let CenterResponse::Service(stopped) = stopped else {
        panic!("应返回停止后的服务摘要");
    };
    assert_eq!(stopped.status, ServiceStatusDto::Stopped);

    let snapshot = center.handle(CenterRequest::Snapshot {
        selector: ServiceSelectorDto::Path(service_root),
    });
    let CenterResponse::Snapshot(snapshot) = snapshot else {
        panic!("应按路径返回任务快照");
    };
    assert!(
        snapshot
            .tasks
            .iter()
            .all(|task| task.status == TaskStatusDto::Stopped)
    );
    let history = repository.status_history("demo").unwrap();
    assert_eq!(history.len(), 2);
    let service_log = fs::read_to_string(directory.join("demo/.procora/logs/service.log")).unwrap();
    assert!(service_log.contains("service_status=running"));
    assert!(service_log.contains("service_status=stopped"));
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn 中心服务器重启后恢复注册表和期望状态() {
    let directory = temporary_directory();
    let service_root = directory.join("demo");
    write_service(&service_root, "demo");
    let state_path = directory.join("procora.sqlite3");
    let mut center = Center::empty(SqliteCenterRepository::new(&state_path));
    center.handle(CenterRequest::Open { path: service_root });
    drop(center);

    let mut restored = Center::load(SqliteCenterRepository::new(state_path)).unwrap();
    let CenterResponse::Services(services) = restored.handle(CenterRequest::List) else {
        panic!("应返回恢复后的服务列表");
    };
    assert_eq!(services.len(), 1);
    assert_eq!(services[0].name, "demo");
    assert_eq!(services[0].status, ServiceStatusDto::Running);
    drop(restored);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn 同名服务不能指向两个目录() {
    let directory = temporary_directory();
    write_service(&directory.join("one"), "demo");
    write_service(&directory.join("two"), "demo");
    let mut center = Center::empty(SqliteCenterRepository::new(
        directory.join("procora.sqlite3"),
    ));
    center.handle(CenterRequest::Open {
        path: directory.join("one"),
    });

    let response = center.handle(CenterRequest::Open {
        path: directory.join("two"),
    });
    assert!(matches!(response, CenterResponse::Error { .. }));
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn 恢复时配置改名会进入失败状态() {
    let directory = temporary_directory();
    let service_root = directory.join("demo");
    write_service(&service_root, "before");
    let state_path = directory.join("procora.sqlite3");
    let mut center = Center::empty(SqliteCenterRepository::new(&state_path));
    center.handle(CenterRequest::Open {
        path: service_root.clone(),
    });
    drop(center);
    write_service(&service_root, "after");

    let mut restored = Center::load(SqliteCenterRepository::new(state_path)).unwrap();
    let CenterResponse::Services(services) = restored.handle(CenterRequest::List) else {
        panic!("应返回恢复后的服务列表");
    };
    assert_eq!(services[0].name, "before");
    assert_eq!(services[0].status, ServiceStatusDto::Failed);
    assert!(services[0].message.as_deref().unwrap().contains("显式迁移"));
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn task日志通过游标从service目录续读() {
    let directory = temporary_directory();
    let service_root = directory.join("demo");
    fs::create_dir_all(&service_root).unwrap();
    fs::write(
        service_root.join("procora.yaml"),
        "version: 1\nproject: demo\ntasks:\n  task:\n    command: rustc\n    args: ['--version']\n",
    )
    .unwrap();
    let mut center = Center::empty(SqliteCenterRepository::new(
        directory.join("procora.sqlite3"),
    ));
    center.handle(CenterRequest::Open { path: service_root });
    let task_id = TaskId::from_str("task").unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(3);

    let batch = loop {
        let response = center.handle(CenterRequest::TaskLogs {
            selector: ServiceSelectorDto::Name("demo".to_owned()),
            task_id: task_id.clone(),
            cursor: None,
            max_bytes: 1024,
        });
        let CenterResponse::TaskLogs(batch) = response else {
            panic!("应返回 Task 日志");
        };
        if !batch.bytes.is_empty() {
            break batch;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "Task 日志没有及时写入"
        );
        thread::sleep(Duration::from_millis(10));
    };
    assert!(
        String::from_utf8(batch.bytes)
            .unwrap()
            .starts_with("rustc ")
    );
    let CenterResponse::TaskLogs(next) = center.handle(CenterRequest::TaskLogs {
        selector: ServiceSelectorDto::Name("demo".to_owned()),
        task_id,
        cursor: Some(batch.next_cursor),
        max_bytes: u32::MAX,
    }) else {
        panic!("应返回续读结果");
    };
    assert!(next.bytes.is_empty());
    drop(center);
    fs::remove_dir_all(directory).unwrap();
}
