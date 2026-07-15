//! 三平台自启动定义渲染测试。

use procora_platform::autostart::DaemonAutostart;

/// 返回包含空格与保留字符的测试定义。
fn definition() -> DaemonAutostart {
    DaemonAutostart::new(
        "/opt/Procora Bin/procora",
        "procora-center-%demo",
        "/tmp/Procora & Data/procora.sqlite3",
    )
}

#[test]
fn systemd单元以前台daemon作为主进程() {
    let unit = definition().systemd_unit();

    assert!(unit.contains("Type=simple"));
    assert!(unit.contains("ExecStart=\"/opt/Procora Bin/procora\" __daemon"));
    assert!(unit.contains("procora-center-%%demo"));
    assert!(unit.contains("WantedBy=default.target"));
}

#[test]
fn launch_agent逐参数编码并设置登录启动() {
    let plist = definition().launch_agent_plist();

    assert!(plist.contains("<string>dev.procora.center</string>"));
    assert!(plist.contains("<key>RunAtLoad</key>"));
    assert!(plist.contains("<string>procora-center-%demo</string>"));
    assert!(plist.contains("Procora &amp; Data"));
    assert!(plist.contains("<key>SuccessfulExit</key>"));
}

#[test]
fn windows任务动作正确引用含空格参数() {
    let action = definition().windows_task_action();

    assert!(action.starts_with("\"/opt/Procora Bin/procora\" __daemon"));
    assert!(action.contains("\"/tmp/Procora & Data/procora.sqlite3\""));
}
