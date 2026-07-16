//! 三平台自启动定义渲染与转义测试。

use procora::platform::autostart::{AutostartBackend, DaemonAutostart};

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
