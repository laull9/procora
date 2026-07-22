use serde::{Serialize, Serializer, ser::SerializeMap};

use super::config_form::{FormConfig, FormDependency, FormDependencyDownload, FormTask};

impl Serialize for FormDependencyDownload {
    /// 下载策略只保存偏离开箱默认值的字段。
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(None)?;
        if self.retries != 2 {
            map.serialize_entry("retries", &self.retries)?;
        }
        if self.timeout_ms != 120_000 {
            map.serialize_entry("timeout", &crate::config::format_duration(self.timeout_ms))?;
        }
        if self.max_bytes != 2 * 1024 * 1024 * 1024 {
            map.serialize_entry("max_bytes", &self.max_bytes)?;
        }
        if !self.headers.is_empty() {
            map.serialize_entry("headers", &self.headers)?;
        }
        map.end()
    }
}

impl Serialize for FormDependency {
    /// 默认依赖只写来源字符串，高级字段按需写成对象。
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if self.is_compact() {
            return serializer.serialize_str(&self.source);
        }
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("source", &self.source)?;
        if !self.mirrors.is_empty() {
            map.serialize_entry("mirrors", &self.mirrors)?;
        }
        if self.version != "source" {
            map.serialize_entry("version", &self.version)?;
        }
        if let Some(checksum) = &self.checksum {
            map.serialize_entry("checksum", checksum)?;
        }
        if self.unpack != "auto" {
            map.serialize_entry("unpack", &self.unpack)?;
        }
        if let Some(path) = &self.path {
            map.serialize_entry("path", path)?;
        }
        if self.kind != "auto" {
            map.serialize_entry("kind", &self.kind)?;
        }
        if let Some(verify) = &self.verify {
            map.serialize_entry("verify", verify)?;
        }
        if !self.download.is_default() {
            map.serialize_entry("download", &self.download)?;
        }
        if !self.ssh.is_empty() {
            map.serialize_entry("ssh", &self.ssh)?;
        }
        map.end()
    }
}

impl Serialize for FormConfig {
    /// 保存基础声明、profile 定义和完整 Task 集合，不展开当前有效值。
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("version", &self.version)?;
        map.serialize_entry("project", &self.project)?;
        if !self.vars.is_empty() {
            map.serialize_entry("vars", &self.vars)?;
        }
        if let Some(profile) = &self.active_profile {
            map.serialize_entry("profile", profile)?;
        }
        if !self.profiles.is_empty() {
            map.serialize_entry("profiles", &self.profiles)?;
        }
        if !self.env.is_empty() {
            map.serialize_entry("env", &self.env)?;
        }
        if !self.task_defaults.is_empty() {
            map.serialize_entry("task_defaults", &self.task_defaults)?;
        }
        if !self.task_templates.is_empty() {
            map.serialize_entry("task_templates", &self.task_templates)?;
        }
        if !self.dependencies.is_empty() {
            map.serialize_entry("dependencies", &self.dependencies)?;
        }
        if !self.uploads.is_empty() {
            map.serialize_entry("uploads", &self.uploads)?;
        }
        let mut tasks = self.inactive_tasks.clone();
        for (name, task) in &self.tasks {
            let value = serde_json::to_value(task).map_err(serde::ser::Error::custom)?;
            tasks.insert(name.clone(), value);
        }
        map.serialize_entry("tasks", &tasks)?;
        map.end()
    }
}

impl Serialize for FormTask {
    /// 以紧凑 argv 命令输出 Task，同时省略没有声明的可选字段。
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(None)?;
        if let Some(template) = &self.extends {
            map.serialize_entry("extends", template)?;
        }
        if self.explicit("command") {
            if self.args.is_empty() {
                map.serialize_entry("command", &self.command)?;
                if self.explicit("args") {
                    map.serialize_entry("args", &self.args)?;
                }
            } else {
                let argv = std::iter::once(&self.command)
                    .chain(&self.args)
                    .collect::<Vec<_>>();
                map.serialize_entry("command", &argv)?;
            }
        } else if self.explicit("args") {
            map.serialize_entry("args", &self.args)?;
        }
        if self.explicit("cwd")
            && let Some(cwd) = &self.cwd
        {
            map.serialize_entry("cwd", cwd)?;
        }
        if let Some(env_file) = &self.env_file {
            map.serialize_entry("env_file", env_file)?;
        }
        if !self.env.is_empty() {
            map.serialize_entry("env", &self.env)?;
        }
        if self.explicit("healthcheck")
            && let Some(healthcheck) = &self.healthcheck
        {
            map.serialize_entry("healthcheck", healthcheck)?;
        }
        if self.explicit("success_exit_codes") {
            map.serialize_entry("success_exit_codes", &self.success_exit_codes)?;
        }
        if !self.depends_on.is_empty() {
            if self
                .depends_on
                .values()
                .all(|dependency| dependency.condition == "started")
            {
                map.serialize_entry("depends_on", &self.depends_on.keys().collect::<Vec<_>>())?;
            } else {
                let conditions = self
                    .depends_on
                    .iter()
                    .map(|(name, dependency)| (name, dependency.condition.as_str()))
                    .collect::<std::collections::BTreeMap<_, _>>();
                map.serialize_entry("depends_on", &conditions)?;
            }
        }
        if !self.uploads.is_empty() {
            map.serialize_entry("uploads", &self.uploads)?;
        }
        if self.explicit("restart") {
            map.serialize_entry("restart", &self.restart)?;
        }
        if self.explicit("restart_delay_ms") {
            map.serialize_entry(
                "restart_delay",
                &crate::config::format_duration(self.restart_delay_ms),
            )?;
        }
        if self.explicit("max_restarts") {
            map.serialize_entry("max_restarts", &self.max_restarts)?;
        }
        if self.explicit("restart_reset_after_ms") {
            map.serialize_entry(
                "restart_reset_after",
                &crate::config::format_duration(self.restart_reset_after_ms),
            )?;
        }
        if self.explicit("shutdown_timeout_ms") {
            map.serialize_entry(
                "shutdown_timeout",
                &crate::config::format_duration(self.shutdown_timeout_ms),
            )?;
        }
        map.end()
    }
}
