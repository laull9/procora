use std::{collections::BTreeMap, path::Path};

use crate::config::{TaskConfigOrigins, ValueOrigin};
use crate::core::RestartPolicy;

use super::config_form::{FormDependency, FormTask};

impl FormTask {
    /// 返回新建 Task 使用的默认值。
    pub(super) fn default_value() -> Self {
        Self {
            extends: None,
            command: String::new(),
            args: Vec::new(),
            cwd: None,
            env_file: None,
            env: BTreeMap::new(),
            healthcheck: None,
            success_exit_codes: vec![0],
            depends_on: BTreeMap::new(),
            restart: "never".to_owned(),
            restart_delay_ms: 500,
            max_restarts: 0,
            restart_reset_after_ms: 60_000,
            shutdown_timeout_ms: 5_000,
            origins: TaskConfigOrigins {
                fields: BTreeMap::from([("command".to_owned(), ValueOrigin::Task)]),
                env: BTreeMap::new(),
                ..TaskConfigOrigins::default()
            },
        }
    }

    /// 判断字段是否应作为 Task 显式声明写回。
    pub(crate) fn explicit(&self, field: &str) -> bool {
        self.origins.field(field) == ValueOrigin::Task
    }

    /// 返回适合 TUI 展示的字段来源。
    pub(crate) fn origin_label(&self, field: &str) -> String {
        match self.origins.field(field) {
            ValueOrigin::BuiltIn => "内建默认".to_owned(),
            ValueOrigin::ProjectEnv => "项目 env".to_owned(),
            ValueOrigin::TaskDefaults => "项目 Task 默认".to_owned(),
            ValueOrigin::Profile => "活动 profile".to_owned(),
            ValueOrigin::TaskTemplate => self
                .origins
                .template(field)
                .map_or_else(|| "Task 模板".to_owned(), |name| format!("模板 {name}")),
            ValueOrigin::EnvFile => "env_file".to_owned(),
            ValueOrigin::Task => "Task 显式".to_owned(),
        }
    }
}

impl FormDependency {
    /// 返回新建管理依赖使用的默认值。
    pub(super) fn default_value() -> Self {
        Self {
            source: String::new(),
            version: String::new(),
            checksum: None,
            unpack: "auto".to_owned(),
            path: None,
            kind: "auto".to_owned(),
            verify: None,
        }
    }
}

/// 尽量把已规范化路径还原为相对当前入口目录的可移植写法。
pub(super) fn form_path(path: &Path, base_directory: Option<&Path>) -> String {
    let relative = base_directory
        .and_then(|base| std::fs::canonicalize(base).ok())
        .and_then(|base| path.strip_prefix(base).ok())
        .unwrap_or(path);
    let text = relative.to_string_lossy().into_owned();
    #[cfg(windows)]
    let text = text.replace('\\', "/");
    text
}

/// 将重启策略转为配置中的拼写。
pub(super) const fn restart_text(value: RestartPolicy) -> &'static str {
    match value {
        RestartPolicy::Never => "never",
        RestartPolicy::OnFailure => "on-failure",
        RestartPolicy::Always => "always",
    }
}
