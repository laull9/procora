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
// 握手增量事件和状态历史形成一致会话。
fn handshake_events_and_history_form_consistent_session() {
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
// 过期事件游标要求客户端重新获取快照。
fn expired_event_cursor_requires_snapshot_refresh() {
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
// 可以按名称和路径管理同一个服务。
fn service_can_be_managed_by_name_and_path() {
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
// 服务根目录内的任意当前目录都能解析到已注册服务。
fn nested_current_directory_resolves_registered_service() {
    let directory = temporary_directory();
    let service_root = directory.join("demo");
    let nested = service_root.join("workspace/deep");
    write_service(&service_root, "demo");
    fs::create_dir_all(&nested).unwrap();
    let mut center = Center::empty(SqliteCenterRepository::new(
        directory.join("procora.sqlite3"),
    ));
    center.handle(CenterRequest::Open { path: service_root });
    let response = center.handle(CenterRequest::Snapshot {
        selector: ServiceSelectorDto::Path(nested),
    });
    assert!(matches!(
        response,
        CenterResponse::Snapshot(snapshot) if snapshot.project == "demo"
    ));
    center.handle(CenterRequest::Shutdown);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
// 嵌套服务同时注册时优先选择路径最接近的服务根目录。
fn nested_services_use_longest_matching_root() {
    let directory = temporary_directory();
    let parent = directory.join("parent");
    let child = parent.join("child");
    let child_work = child.join("workspace");
    let parent_work = parent.join("other");
    write_service(&parent, "parent");
    write_service(&child, "child");
    fs::create_dir_all(&child_work).unwrap();
    fs::create_dir_all(&parent_work).unwrap();
    let mut center = Center::empty(SqliteCenterRepository::new(
        directory.join("procora.sqlite3"),
    ));
    center.handle(CenterRequest::Open { path: parent });
    center.handle(CenterRequest::Open { path: child });
    let child_response = center.handle(CenterRequest::Snapshot {
        selector: ServiceSelectorDto::Path(child_work),
    });
    let parent_response = center.handle(CenterRequest::Snapshot {
        selector: ServiceSelectorDto::Path(parent_work),
    });
    assert!(matches!(
        child_response,
        CenterResponse::Snapshot(snapshot) if snapshot.project == "child"
    ));
    assert!(matches!(
        parent_response,
        CenterResponse::Snapshot(snapshot) if snapshot.project == "parent"
    ));
    center.handle(CenterRequest::Shutdown);
    fs::remove_dir_all(directory).unwrap();
}

#[cfg(unix)]
#[test]
// macOS等Unix平台通过符号链接进入子目录时仍使用规范化服务路径。
fn symlinked_nested_path_resolves_registered_service() {
    use std::os::unix::fs::symlink;

    let directory = temporary_directory();
    let service_root = directory.join("demo");
    let nested = service_root.join("workspace");
    let alias = directory.join("demo-alias");
    write_service(&service_root, "demo");
    fs::create_dir_all(&nested).unwrap();
    symlink(&service_root, &alias).unwrap();
    let mut center = Center::empty(SqliteCenterRepository::new(
        directory.join("procora.sqlite3"),
    ));
    center.handle(CenterRequest::Open { path: service_root });

    let response = center.handle(CenterRequest::Snapshot {
        selector: ServiceSelectorDto::Path(alias.join("workspace")),
    });

    assert!(matches!(
        response,
        CenterResponse::Snapshot(snapshot) if snapshot.project == "demo"
    ));
    center.handle(CenterRequest::Shutdown);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
// remove停止宿主并彻底删除注册记录。
fn remove_stops_host_and_deletes_registry() {
    let directory = temporary_directory();
    let service_root = directory.join("demo");
    write_service(&service_root, "demo");
    let repository = SqliteCenterRepository::new(directory.join("procora.sqlite3"));
    let mut center = Center::empty(repository.clone());
    center.handle(CenterRequest::Open {
        path: service_root.clone(),
    });

    let removed = center.handle(CenterRequest::Remove {
        selector: ServiceSelectorDto::Path(service_root.clone()),
    });
    assert!(matches!(
        removed,
        CenterResponse::Removed(service) if service.name == "demo"
    ));
    assert!(matches!(
        center.handle(CenterRequest::List),
        CenterResponse::Services(services) if services.is_empty()
    ));
    assert!(repository.load_services().unwrap().is_empty());
    assert!(repository.status_history("demo").unwrap().is_empty());
    assert!(service_root.join("procora.yaml").is_file());
    fs::remove_dir_all(directory).unwrap();
}

#[test]
// 中心服务器重启后恢复注册表和期望状态。
fn center_restart_restores_registry_and_desired_state() {
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
// 旧配置入口消失后，同名服务可从新目录接管陈旧注册记录。
fn missing_config_registration_can_relocate() {
    let directory = temporary_directory();
    let old_root = directory.join("downloads");
    let new_root = old_root.join("Test");
    let state_path = directory.join("procora.sqlite3");
    write_service(&old_root, "downloads");
    let mut center = Center::empty(SqliteCenterRepository::new(&state_path));
    center.handle(CenterRequest::Open {
        path: old_root.clone(),
    });
    center.handle(CenterRequest::Manage {
        action: ServiceActionDto::Stop,
        selector: ServiceSelectorDto::Name("downloads".to_owned()),
    });
    drop(center);
    fs::remove_file(old_root.join("procora.yaml")).unwrap();
    write_service(&new_root, "downloads");

    let mut restored = Center::load(SqliteCenterRepository::new(&state_path)).unwrap();
    assert!(matches!(
        restored.handle(CenterRequest::Snapshot {
            selector: ServiceSelectorDto::Name("downloads".to_owned()),
        }),
        CenterResponse::Error { .. }
    ));
    let reopened = restored.handle(CenterRequest::Open {
        path: new_root.clone(),
    });

    assert!(matches!(
        reopened,
        CenterResponse::Service(service)
            if service.name == "downloads"
                && service.root == procora::platform::canonicalize(&new_root).unwrap()
    ));
    let stored = SqliteCenterRepository::new(state_path)
        .load_services()
        .unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(
        stored[0].root,
        procora::platform::canonicalize(&new_root).unwrap()
    );
    restored.handle(CenterRequest::Shutdown);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
// 中心仍在运行时移动配置，也可在显式打开新目录时停止旧宿主并迁移。
fn live_service_with_missing_config_can_relocate() {
    let directory = temporary_directory();
    let old_root = directory.join("downloads");
    let new_root = old_root.join("Test");
    write_service(&old_root, "downloads");
    let mut center = Center::empty(SqliteCenterRepository::new(
        directory.join("procora.sqlite3"),
    ));
    center.handle(CenterRequest::Open {
        path: old_root.clone(),
    });
    fs::remove_file(old_root.join("procora.yaml")).unwrap();
    write_service(&new_root, "downloads");

    let reopened = center.handle(CenterRequest::Open {
        path: new_root.clone(),
    });

    assert!(matches!(
        reopened,
        CenterResponse::Service(service)
            if service.root == procora::platform::canonicalize(&new_root).unwrap()
    ));
    center.handle(CenterRequest::Shutdown);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
// 同名服务不能指向两个目录。
fn same_service_name_cannot_point_to_two_directories() {
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
// 恢复时配置改名会进入失败状态。
fn renamed_config_enters_failed_state_during_restore() {
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
// task日志通过游标从service目录续读。
fn task_logs_resume_from_service_directory_cursor() {
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
