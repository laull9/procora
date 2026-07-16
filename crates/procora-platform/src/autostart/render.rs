//! 各原生托管器的配置和命令行渲染。

use std::{ffi::OsStr, fmt::Write as _, path::PathBuf};

use super::{DaemonAutostart, LAUNCHD_LABEL};

impl DaemonAutostart {
    /// 生成 Linux systemd 用户单元内容。
    pub fn systemd_unit(&self) -> String {
        let home = self
            .database
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."));
        format!(
            "[Unit]\nDescription=Procora Global Server\nStartLimitIntervalSec=30s\nStartLimitBurst=5\n\n[Service]\nType=simple\nEnvironment=PROCORA_HOME={}\nExecStart={} __daemon --endpoint {} --database {}\nExecStop={} down\nRestart=on-failure\nRestartSec=2s\nTimeoutStopSec=10s\nKillMode=control-group\n\n[Install]\nWantedBy=default.target\n",
            systemd_argument(home),
            systemd_argument(&self.executable),
            systemd_argument(&self.endpoint),
            systemd_argument(&self.database),
            systemd_argument(&self.executable),
        )
    }

    /// 生成 macOS `LaunchAgent` plist 内容。
    pub fn launch_agent_plist(&self) -> String {
        let stdout = sibling_path(&self.database, "center.stdout.log");
        let stderr = sibling_path(&self.database, "center.stderr.log");
        let mut arguments = String::new();
        for argument in &daemon_arguments(self) {
            writeln!(arguments, "    <string>{}</string>", xml_escape(argument))
                .expect("写入 String 不会失败");
        }
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n<dict>\n  <key>Label</key>\n  <string>{LAUNCHD_LABEL}</string>\n  <key>ProgramArguments</key>\n  <array>\n{arguments}  </array>\n  <key>RunAtLoad</key>\n  <true/>\n  <key>KeepAlive</key>\n  <dict>\n    <key>SuccessfulExit</key>\n    <false/>\n  </dict>\n  <key>ProcessType</key>\n  <string>Background</string>\n  <key>StandardOutPath</key>\n  <string>{}</string>\n  <key>StandardErrorPath</key>\n  <string>{}</string>\n</dict>\n</plist>\n",
            xml_escape(&stdout.to_string_lossy()),
            xml_escape(&stderr.to_string_lossy()),
        )
    }

    /// 生成 Windows 任务计划程序使用的完整执行动作。
    pub fn windows_task_action(&self) -> String {
        daemon_arguments(self)
            .iter()
            .map(|argument| windows_argument(argument))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// 返回内部前台 daemon 的完整参数数组。
fn daemon_arguments(definition: &DaemonAutostart) -> [String; 6] {
    [
        definition.executable.to_string_lossy().into_owned(),
        "__daemon".to_owned(),
        "--endpoint".to_owned(),
        definition.endpoint.clone(),
        "--database".to_owned(),
        definition.database.to_string_lossy().into_owned(),
    ]
}

/// 返回数据库同目录下的日志文件路径。
fn sibling_path(database: &std::path::Path, name: &str) -> PathBuf {
    database
        .parent()
        .map_or_else(|| PathBuf::from(name), |parent| parent.join(name))
}

/// 把参数编码成 systemd.service 支持的双引号参数。
fn systemd_argument(value: impl AsRef<OsStr>) -> String {
    let value = value.as_ref().to_string_lossy();
    let mut escaped = String::with_capacity(value.len() + 2);
    escaped.push('"');
    for character in value.chars() {
        match character {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '%' => escaped.push_str("%%"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            _ => escaped.push(character),
        }
    }
    escaped.push('"');
    escaped
}

/// 转义 XML 文本节点中的保留字符。
fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// 按 Windows 命令行解析规则编码单个参数。
fn windows_argument(value: &str) -> String {
    if !value.is_empty()
        && !value
            .chars()
            .any(|character| character.is_whitespace() || character == '"')
    {
        return value.to_owned();
    }
    let mut encoded = String::from("\"");
    let mut backslashes = 0;
    for character in value.chars() {
        if character == '\\' {
            backslashes += 1;
        } else if character == '"' {
            encoded.push_str(&"\\".repeat(backslashes * 2 + 1));
            encoded.push('"');
            backslashes = 0;
        } else {
            encoded.push_str(&"\\".repeat(backslashes));
            backslashes = 0;
            encoded.push(character);
        }
    }
    encoded.push_str(&"\\".repeat(backslashes * 2));
    encoded.push('"');
    encoded
}
