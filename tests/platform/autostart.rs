//! 三平台自启动定义渲染、转义与注册服务恢复测试。

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use procora::daemon::{Center, CenterClient};
use procora::platform::autostart::{AutostartBackend, DaemonAutostart};
use procora::protocol::{
    CenterRequest, CenterResponse, ServiceActionDto, ServiceSelectorDto, ServiceStatusDto,
};
use procora::storage::SqliteCenterRepository;

/// 当前测试进程内的自启动运行时目录序号。
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// 创建自启动测试独占目录和本地 IPC 端点。
fn isolated_runtime(label: &str) -> (PathBuf, String) {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let endpoint = format!(
        "procora-autostart-{label}-{}-{nonce}-{sequence}",
        std::process::id()
    );
    let directory = std::env::temp_dir().join(&endpoint);
    fs::create_dir_all(&directory).unwrap();
    (directory, endpoint)
}

/// 写入会在每次启动时输出版本号的跨平台服务配置。
fn write_boot_service(root: &Path, project: &str) {
    fs::create_dir_all(root).unwrap();
    let config = serde_json::json!({
        "version": 1,
        "project": project,
        "tasks": {
            "boot": {
                "command": env!("CARGO_BIN_EXE_procora"),
                "args": ["--version"]
            }
        }
    });
    fs::write(
        root.join("procora.json"),
        serde_json::to_vec_pretty(&config).unwrap(),
    )
    .unwrap();
}

/// 预先注册服务并按测试需要保存运行或停止期望。
fn register_service(database: &Path, root: &Path, project: &str, stopped: bool) {
    let mut center = Center::empty(SqliteCenterRepository::new(database));
    assert!(matches!(
        center.handle(CenterRequest::Open {
            path: root.to_path_buf()
        }),
        CenterResponse::Service(service) if service.status == ServiceStatusDto::Running
    ));
    if stopped {
        assert!(matches!(
            center.handle(CenterRequest::Manage {
                action: ServiceActionDto::Stop,
                selector: ServiceSelectorDto::Name(project.to_owned()),
            }),
            CenterResponse::Service(service) if service.status == ServiceStatusDto::Stopped
        ));
    }
    center.handle(CenterRequest::Shutdown);
}

/// 以前台 daemon 入口模拟系统用户登录后的原生托管启动。
fn spawn_daemon(endpoint: &str, database: &Path) -> Child {
    Command::new(env!("CARGO_BIN_EXE_procora"))
        .arg("__daemon")
        .arg("--endpoint")
        .arg(endpoint)
        .arg("--database")
        .arg(database)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap()
}

/// 在限定时间内等待中心服务器或 Task 启动证据出现。
fn wait_until(mut ready: impl FnMut() -> bool) -> bool {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        if ready() {
            return true;
        }
        thread::sleep(Duration::from_millis(20));
    }
    false
}

/// 正常关闭测试 daemon，并在通信失败时兜底回收子进程。
fn stop_daemon(mut daemon: Child, client: &CenterClient, ready: bool) -> std::process::ExitStatus {
    if ready {
        let _ = client.request(&CenterRequest::Shutdown);
    } else {
        let _ = daemon.kill();
    }
    daemon.wait().unwrap()
}

/// 返回包含空格与保留字符的测试定义。
fn definition() -> DaemonAutostart {
    DaemonAutostart::new(
        "/opt/Procora Bin/procora",
        "procora-center-%demo",
        "/tmp/Procora & Data/procora.sqlite3",
    )
}

#[test]
// systemd单元以前台daemon作为主进程。
fn systemd_unit_uses_foreground_daemon() {
    let unit = definition().systemd_unit();

    assert!(unit.contains("Type=simple"));
    assert!(unit.contains("Description=Procora Global Server"));
    assert!(unit.contains("Environment=PROCORA_HOME=\"/tmp/Procora & Data\""));
    assert!(unit.contains("ExecStart=\"/opt/Procora Bin/procora\" __daemon"));
    assert!(unit.contains("ExecStop=\"/opt/Procora Bin/procora\" down"));
    assert!(unit.contains("procora-center-%%demo"));
    assert!(unit.contains("StartLimitBurst=5"));
    assert!(unit.contains("TimeoutStopSec=10s"));
    assert!(unit.contains("KillMode=control-group"));
    assert!(unit.contains("WantedBy=default.target"));
}

#[test]
// launch_agent逐参数编码并设置登录启动。
fn launch_agent_encodes_arguments_and_runs_at_login() {
    let plist = definition().launch_agent_plist();

    assert!(plist.contains("<string>dev.procora.center</string>"));
    assert!(plist.contains("<key>RunAtLoad</key>"));
    assert!(plist.contains("<string>procora-center-%demo</string>"));
    assert!(plist.contains("Procora &amp; Data"));
    assert!(plist.contains("<key>SuccessfulExit</key>"));
}

#[test]
// windows任务动作正确引用含空格参数。
fn windows_task_quotes_arguments_with_spaces() {
    let action = definition().windows_task_action();

    assert!(action.starts_with("\"/opt/Procora Bin/procora\" __daemon"));
    assert!(action.contains("\"/tmp/Procora & Data/procora.sqlite3\""));
}

#[test]
// windows任务绑定当前交互登录会话。
fn windows_task_targets_interactive_login() {
    let arguments = definition().windows_task_create_arguments();
    let arguments = arguments
        .iter()
        .map(|argument| argument.to_string_lossy())
        .collect::<Vec<_>>();

    assert!(arguments.windows(2).any(|pair| pair == ["/SC", "ONLOGON"]));
    assert!(arguments.iter().any(|argument| argument == "/IT"));
    assert!(arguments.windows(2).any(|pair| pair == ["/RL", "LIMITED"]));
}

/// 三个原生后端都应提供稳定、面向用户的名称。
#[test]
// 原生后端名称保持稳定。
fn native_backend_labels_remain_stable() {
    assert_eq!(AutostartBackend::SystemdUser.label(), "systemd 用户服务");
    assert_eq!(AutostartBackend::LaunchAgent.label(), "macOS LaunchAgent");
    assert_eq!(
        AutostartBackend::WindowsTask.label(),
        "Windows 任务计划程序"
    );
}

/// systemd 语法中的特殊字符必须在参数和百分号说明符中安全转义。
#[test]
// systemd单元转义特殊参数。
fn systemd_unit_escapes_special_arguments() {
    let definition = DaemonAutostart::new(
        "/opt/Procora \"bin\"/procora",
        "endpoint%name\nnext",
        "/tmp/Procora\\data/procora.sqlite3",
    );
    let unit = definition.systemd_unit();

    assert!(unit.contains("\"/opt/Procora \\\"bin\\\"/procora\""));
    assert!(unit.contains("\"endpoint%%name\\nnext\""));
    assert!(unit.contains("\"/tmp/Procora\\\\data/procora.sqlite3\""));
}

/// `LaunchAgent` XML 文本节点必须完整转义五个保留字符。
#[test]
// launch_agent转义xml保留字符。
fn launch_agent_escapes_xml_reserved_characters() {
    let definition = DaemonAutostart::new(
        "/opt/procora<&>\"'",
        "endpoint<&>\"'",
        "/tmp/data<&>\"'/procora.sqlite3",
    );
    let plist = definition.launch_agent_plist();

    assert!(plist.contains("&lt;&amp;&gt;&quot;&apos;"));
    assert!(plist.contains("endpoint&lt;&amp;&gt;&quot;&apos;"));
}

/// Windows 参数中的引号和末尾反斜杠必须按 `CommandLineToArgvW` 规则编码。
#[test]
// windows任务动作转义引号和末尾反斜杠。
fn windows_task_escapes_quotes_and_trailing_backslashes() {
    let definition = DaemonAutostart::new(
        "C:\\Program Files\\Procora\\",
        "endpoint \"quoted\"",
        "C:\\Data Path\\state\\",
    );
    let action = definition.windows_task_action();

    assert!(action.starts_with("\"C:\\Program Files\\Procora\\\\\" __daemon"));
    assert!(action.contains("\"endpoint \\\"quoted\\\"\""));
    assert!(action.ends_with("\"C:\\Data Path\\state\\\\\""));
}

#[test]
// 登录启动daemon后无需open请求即可拉起保存了运行期望的task。
fn login_daemon_starts_registered_running_service_without_client_open() {
    let (directory, endpoint) = isolated_runtime("running");
    let service = directory.join("service");
    let database = directory.join("procora.sqlite3");
    write_boot_service(&service, "boot-running");
    register_service(&database, &service, "boot-running", false);
    let task_log = service.join(".procora/logs/tasks/boot.log");
    let _ = fs::remove_file(&task_log);

    let daemon = spawn_daemon(&endpoint, &database);
    let client = CenterClient::new(endpoint);
    let daemon_ready = wait_until(|| client.ping());
    let task_started = daemon_ready
        && wait_until(|| {
            fs::read_to_string(&task_log)
                .is_ok_and(|content| content.contains(env!("CARGO_PKG_VERSION")))
        });
    let services = daemon_ready
        .then(|| client.request(&CenterRequest::List).ok())
        .flatten();
    let status = stop_daemon(daemon, &client, daemon_ready);

    assert!(daemon_ready, "自启动 daemon 未在限定时间内就绪");
    assert!(task_started, "已注册服务的 Task 没有在登录启动后自动执行");
    assert!(matches!(
        services,
        Some(CenterResponse::Services(services))
            if services.len() == 1 && services[0].status == ServiceStatusDto::Running
    ));
    assert!(status.success());
    fs::remove_dir_all(directory).unwrap();
}

#[test]
// 登录启动daemon必须保留用户显式停止的服务状态。
fn login_daemon_does_not_start_registered_stopped_service() {
    let (directory, endpoint) = isolated_runtime("stopped");
    let service = directory.join("service");
    let database = directory.join("procora.sqlite3");
    write_boot_service(&service, "boot-stopped");
    register_service(&database, &service, "boot-stopped", true);
    let task_log = service.join(".procora/logs/tasks/boot.log");
    let _ = fs::remove_file(&task_log);

    let daemon = spawn_daemon(&endpoint, &database);
    let client = CenterClient::new(endpoint);
    let daemon_ready = wait_until(|| client.ping());
    thread::sleep(Duration::from_millis(300));
    let services = daemon_ready
        .then(|| client.request(&CenterRequest::List).ok())
        .flatten();
    let task_started = task_log.exists();
    let status = stop_daemon(daemon, &client, daemon_ready);

    assert!(daemon_ready, "自启动 daemon 未在限定时间内就绪");
    assert!(!task_started, "已显式停止的服务不应在登录时自动执行 Task");
    assert!(matches!(
        services,
        Some(CenterResponse::Services(services))
            if services.len() == 1 && services[0].status == ServiceStatusDto::Stopped
    ));
    assert!(status.success());
    fs::remove_dir_all(directory).unwrap();
}
