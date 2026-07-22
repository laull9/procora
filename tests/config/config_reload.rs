//! Center 配置候选、防抖监听、修订确认与旧宿主保留测试。

use std::{
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use procora::{
    daemon::Center,
    protocol::{
        CenterEventKindDto, CenterRequest, CenterResponse, ServiceActionDto, ServiceSelectorDto,
        ServiceStatusDto,
    },
    storage::SqliteCenterRepository,
};

/// 当前测试进程内的临时目录去重序列。
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// 创建当前测试独占的临时目录。
fn temporary_directory(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let directory = std::env::temp_dir().join(format!(
        "procora-reload-{label}-{}-{nonce}-{sequence}",
        std::process::id()
    ));
    fs::create_dir_all(&directory).unwrap();
    directory
}

/// 写入一个适合生命周期测试的长期运行服务。
fn write_service(root: &Path, task_ids: &[&str]) -> PathBuf {
    fs::create_dir_all(root).unwrap();
    let tasks = task_ids.iter().fold(String::new(), |mut output, task| {
        writeln!(output, "  {task}:\n    {}", long_running_task()).unwrap();
        output
    });
    let path = root.join("procora.yaml");
    fs::write(&path, format!("version: 1\nproject: demo\ntasks:\n{tasks}")).unwrap();
    path
}

/// 返回跨平台可由测试显式停止的 Task 配置片段。
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

/// 创建空 Center 并注册测试服务。
fn opened_center(directory: &Path, service_root: &Path) -> Center {
    let mut center = Center::empty(SqliteCenterRepository::new(
        directory.join("procora.sqlite3"),
    ));
    let response = center.handle(CenterRequest::Open {
        path: service_root.to_path_buf(),
    });
    assert!(matches!(response, CenterResponse::Service(_)));
    center
}

/// 返回 demo 服务选择器。
fn demo() -> ServiceSelectorDto {
    ServiceSelectorDto::Name("demo".to_owned())
}

/// 写入两个会以文件记录每次真实启动的独立 Task。
fn write_counted_service(root: &Path, changed_marker: &str, invalid_changed: bool) {
    let stable = counted_task("stable.starts");
    let changed = if invalid_changed {
        "command: procora-command-that-must-not-exist\n    cwd: .".to_owned()
    } else {
        counted_task(changed_marker)
    };
    fs::write(
        root.join("procora.yaml"),
        format!(
            "version: 1\nproject: demo\ntasks:\n  stable:\n    {stable}\n  changed:\n    {changed}\n"
        ),
    )
    .unwrap();
}

/// 返回会记录启动次数并持续运行的跨平台 Task 片段。
fn counted_task(marker: &str) -> String {
    #[cfg(unix)]
    {
        format!("command: sh\n    args: ['-c', 'echo start >> {marker}; sleep 30']\n    cwd: .")
    }
    #[cfg(windows)]
    {
        format!(
            "command: cmd.exe\n    args: ['/C', 'echo start>>{marker} & ping -n 30 127.0.0.1 > NUL']\n    cwd: ."
        )
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = marker;
        "command: rustc\n    args: ['--version']\n    cwd: .".to_owned()
    }
}

/// 等待启动标记达到指定行数，避免依赖固定进程调度延迟。
fn wait_for_lines(path: &Path, expected: usize) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        let lines = fs::read_to_string(path).map_or(0, |content| content.lines().count());
        if lines >= expected {
            return;
        }
        thread::sleep(Duration::from_millis(20));
    }
    panic!("{} 未达到 {expected} 行", path.display());
}

#[test]
// 无效候选与restart失败都保留旧运行宿主。
fn invalid_candidate_and_restart_failure_keep_old_host() {
    let directory = temporary_directory("invalid");
    let root = directory.join("service");
    let config = write_service(&root, &["first"]);
    let mut center = opened_center(&directory, &root);
    fs::write(&config, "version: 1\nproject: demo\ntasks: [").unwrap();

    let preview = center.handle(CenterRequest::PreviewConfig { selector: demo() });
    let CenterResponse::ConfigCandidate(candidate) = preview else {
        panic!("应返回候选预览");
    };
    assert!(!candidate.valid);
    assert!(candidate.message.unwrap().contains("解析失败"));
    assert!(matches!(
        center.handle(CenterRequest::Manage {
            action: ServiceActionDto::Restart,
            selector: demo(),
        }),
        CenterResponse::Error { .. }
    ));
    let CenterResponse::Services(services) = center.handle(CenterRequest::List) else {
        panic!("应返回服务列表");
    };
    assert_eq!(services[0].status, ServiceStatusDto::Running);
    assert_eq!(services[0].task_count, 1);

    center.handle(CenterRequest::Manage {
        action: ServiceActionDto::Stop,
        selector: demo(),
    });
    fs::remove_dir_all(directory).unwrap();
}

#[test]
// apply拒绝过期修订并只提交重新确认的候选。
fn apply_rejects_stale_revision_and_commits_reconfirmed_candidate() {
    let directory = temporary_directory("revision");
    let root = directory.join("service");
    let config = write_service(&root, &["first"]);
    let mut center = opened_center(&directory, &root);
    write_service(&root, &["first", "second"]);
    let CenterResponse::ConfigCandidate(first) =
        center.handle(CenterRequest::PreviewConfig { selector: demo() })
    else {
        panic!("应返回首次候选");
    };
    assert_eq!(first.diff.as_ref().unwrap().added[0].as_str(), "second");
    let stale_revision = first.revision.unwrap();

    write_service(&root, &["first", "third"]);
    assert!(matches!(
        center.handle(CenterRequest::ApplyConfig {
            selector: demo(),
            revision: stale_revision,
        }),
        CenterResponse::Error { .. }
    ));
    let CenterResponse::Services(before_apply) = center.handle(CenterRequest::List) else {
        panic!("应返回应用前列表");
    };
    assert_eq!(before_apply[0].task_count, 1);

    let CenterResponse::ConfigCandidate(latest) =
        center.handle(CenterRequest::PreviewConfig { selector: demo() })
    else {
        panic!("应返回最新候选");
    };
    let applied = center.handle(CenterRequest::ApplyConfig {
        selector: demo(),
        revision: latest.revision.unwrap(),
    });
    let CenterResponse::Service(service) = applied else {
        panic!("应成功应用最新候选: {applied:?}");
    };
    assert_eq!(service.task_count, 2);
    assert_eq!(service.status, ServiceStatusDto::Running);

    center.handle(CenterRequest::Manage {
        action: ServiceActionDto::Stop,
        selector: demo(),
    });
    drop(config);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
// 文件事件防抖后只生成候选而不自动替换宿主。
fn debounced_file_events_only_create_candidates() {
    let directory = temporary_directory("watch");
    let root = directory.join("service");
    write_service(&root, &["first"]);
    let mut center = opened_center(&directory, &root);
    let CenterResponse::Events(initial) =
        center.handle(CenterRequest::Events { after_sequence: 0 })
    else {
        panic!("应返回初始事件");
    };
    write_service(&root, &["first", "second"]);

    let deadline = Instant::now() + Duration::from_secs(4);
    let mut observed = false;
    while Instant::now() < deadline {
        thread::sleep(Duration::from_millis(50));
        let CenterResponse::Events(batch) = center.handle(CenterRequest::Events {
            after_sequence: initial.next_sequence,
        }) else {
            panic!("应返回增量事件");
        };
        observed |= batch
            .events
            .iter()
            .any(|event| event.kind == CenterEventKindDto::ConfigCandidateChanged);
        if observed {
            break;
        }
    }
    assert!(observed, "监听器应在静默窗口后生成候选事件");
    let CenterResponse::Services(services) = center.handle(CenterRequest::List) else {
        panic!("应返回服务列表");
    };
    assert_eq!(services[0].task_count, 1, "候选不得被隐式应用");

    center.handle(CenterRequest::Manage {
        action: ServiceActionDto::Stop,
        selector: demo(),
    });
    fs::remove_dir_all(directory).unwrap();
}

#[test]
// apply只重启语义差异影响的task。
fn apply_restarts_only_semantically_affected_tasks() {
    let directory = temporary_directory("scoped");
    let root = directory.join("service");
    fs::create_dir_all(&root).unwrap();
    write_counted_service(&root, "changed-v1.starts", false);
    let mut center = opened_center(&directory, &root);
    wait_for_lines(&root.join("stable.starts"), 1);
    wait_for_lines(&root.join("changed-v1.starts"), 1);

    write_counted_service(&root, "changed-v2.starts", false);
    let CenterResponse::ConfigCandidate(candidate) =
        center.handle(CenterRequest::PreviewConfig { selector: demo() })
    else {
        panic!("应返回候选预览");
    };
    let diff = candidate.diff.as_ref().unwrap();
    assert_eq!(diff.restart[0].as_str(), "changed");
    assert_eq!(diff.unchanged[0].as_str(), "stable");
    let response = center.handle(CenterRequest::ApplyConfig {
        selector: demo(),
        revision: candidate.revision.unwrap(),
    });
    assert!(matches!(response, CenterResponse::Service(_)));
    wait_for_lines(&root.join("changed-v2.starts"), 1);
    assert_eq!(
        fs::read_to_string(root.join("stable.starts"))
            .unwrap()
            .lines()
            .count(),
        1,
        "无影响 Task 必须保留原运行身份"
    );

    center.handle(CenterRequest::Manage {
        action: ServiceActionDto::Stop,
        selector: demo(),
    });
    fs::remove_dir_all(directory).unwrap();
}

#[test]
// 受影响task启动失败会恢复旧有效定义。
fn affected_task_start_failure_restores_previous_definition() {
    let directory = temporary_directory("rollback");
    let root = directory.join("service");
    fs::create_dir_all(&root).unwrap();
    write_counted_service(&root, "changed-v1.starts", false);
    let mut center = opened_center(&directory, &root);
    wait_for_lines(&root.join("stable.starts"), 1);
    wait_for_lines(&root.join("changed-v1.starts"), 1);

    write_counted_service(&root, "unused", true);
    let CenterResponse::ConfigCandidate(candidate) =
        center.handle(CenterRequest::PreviewConfig { selector: demo() })
    else {
        panic!("应返回候选预览");
    };
    let response = center.handle(CenterRequest::ApplyConfig {
        selector: demo(),
        revision: candidate.revision.unwrap(),
    });
    assert!(matches!(response, CenterResponse::Error { .. }));
    wait_for_lines(&root.join("changed-v1.starts"), 2);
    let CenterResponse::Services(services) = center.handle(CenterRequest::List) else {
        panic!("应返回服务列表");
    };
    assert_eq!(services[0].status, ServiceStatusDto::Running);
    assert_eq!(
        fs::read_to_string(root.join("stable.starts"))
            .unwrap()
            .lines()
            .count(),
        1,
        "回退也不得重启无影响 Task"
    );

    center.handle(CenterRequest::Manage {
        action: ServiceActionDto::Stop,
        selector: demo(),
    });
    fs::remove_dir_all(directory).unwrap();
}

#[test]
// 管理依赖占位符不会让未变化候选产生伪重启。
fn dependency_placeholders_do_not_cause_spurious_restarts() {
    let directory = temporary_directory("dependency-semantic");
    let root = directory.join("service");
    fs::create_dir_all(&root).unwrap();
    let executable = std::env::current_exe().unwrap();
    let config = serde_json::json!({
        "version": 1,
        "project": "demo",
        "dependencies": {
            "tool": {
                "source": executable,
                "version": "test",
                "kind": "binary"
            }
        },
        "tasks": {
            "once": {
                "command": "${dependency.tool}",
                "args": ["--help"]
            }
        }
    });
    fs::write(
        root.join("procora.json"),
        serde_json::to_vec_pretty(&config).unwrap(),
    )
    .unwrap();
    let mut center = opened_center(&directory, &root);

    let CenterResponse::ConfigCandidate(candidate) =
        center.handle(CenterRequest::PreviewConfig { selector: demo() })
    else {
        panic!("应返回候选预览");
    };
    assert!(candidate.valid);
    let diff = candidate.diff.unwrap();
    assert!(diff.is_empty());
    assert_eq!(diff.unchanged[0].as_str(), "once");

    center.handle(CenterRequest::Manage {
        action: ServiceActionDto::Stop,
        selector: demo(),
    });
    fs::remove_dir_all(directory).unwrap();
}

#[test]
// apply会核对整个include闭包并拒绝成员文件的过期修订。
fn apply_checks_include_closure_and_rejects_stale_member() {
    let directory = temporary_directory("include-revision");
    let root = directory.join("service");
    fs::create_dir_all(&root).unwrap();
    fs::write(
        root.join("procora.yaml"),
        "include: [tasks.yaml]\nversion: 1\nproject: demo\ntasks: {}\n",
    )
    .unwrap();
    write_included_tasks(&root, &["first"]);
    let mut center = opened_center(&directory, &root);

    write_included_tasks(&root, &["first", "second"]);
    let CenterResponse::ConfigCandidate(candidate) =
        center.handle(CenterRequest::PreviewConfig { selector: demo() })
    else {
        panic!("应返回 include 候选预览");
    };
    let stale_revision = candidate.revision.unwrap();
    write_included_tasks(&root, &["first", "third"]);
    let response = center.handle(CenterRequest::ApplyConfig {
        selector: demo(),
        revision: stale_revision,
    });
    assert!(matches!(response, CenterResponse::Error { .. }));
    let CenterResponse::Services(services) = center.handle(CenterRequest::List) else {
        panic!("应返回服务列表");
    };
    assert_eq!(services[0].status, ServiceStatusDto::Running);
    assert_eq!(services[0].task_count, 1);

    center.handle(CenterRequest::Manage {
        action: ServiceActionDto::Stop,
        selector: demo(),
    });
    fs::remove_dir_all(directory).unwrap();
}

/// 写入由入口 include 的长期运行 Task 片段。
fn write_included_tasks(root: &Path, task_ids: &[&str]) {
    use std::fmt::Write as _;
    let mut output = String::from("tasks:\n");
    for task_id in task_ids {
        writeln!(output, "  {task_id}:\n    {}", long_running_task()).unwrap();
    }
    fs::write(root.join("tasks.yaml"), output).unwrap();
}
