use super::config_form::{FormConfig, FormHealthCheck, FormTask};

impl FormConfig {
    /// 使用易读且所有字符串均安全转义的 YAML 输出配置。
    #[allow(clippy::format_push_string)]
    pub(super) fn yaml(&self) -> String {
        let mut text = format!(
            "version: {}\nproject: {}\n",
            self.version,
            quoted(&self.project)
        );
        if !self.vars.is_empty() {
            text.push_str("vars:\n");
            for (key, value) in &self.vars {
                text.push_str(&format!("  {}: {}\n", quoted(key), quoted(value)));
            }
        }
        optional_yaml(&mut text, 0, "profile", self.active_profile.as_deref());
        if !self.profiles.is_empty() {
            text.push_str("profiles: ");
            text.push_str(
                &serde_json::to_string(&self.profiles)
                    .expect("profile 声明可序列化为 YAML 兼容 JSON"),
            );
            text.push('\n');
        }
        if !self.env.is_empty() {
            text.push_str("env:\n");
            for (key, value) in &self.env {
                text.push_str(&format!("  {}: {}\n", quoted(key), quoted(value)));
            }
        }
        self.task_defaults.append_yaml(&mut text);
        if !self.task_templates.is_empty() {
            text.push_str("task_templates: ");
            text.push_str(
                &serde_json::to_string(&self.task_templates)
                    .expect("模板声明可序列化为 YAML 兼容 JSON"),
            );
            text.push('\n');
        }
        append_dependencies(&mut text, self);
        text.push_str("tasks:\n");
        for (id, task) in &self.tasks {
            text.push_str(&format!("  {}:\n", quoted(id)));
            optional_yaml(&mut text, 4, "extends", task.extends.as_deref());
            append_command(&mut text, task);
            if task.explicit("cwd") {
                optional_yaml(&mut text, 4, "cwd", task.cwd.as_deref());
            }
            optional_yaml(&mut text, 4, "env_file", task.env_file.as_deref());
            if !task.env.is_empty() {
                text.push_str("    env:\n");
                for (key, value) in &task.env {
                    text.push_str(&format!("      {}: {}\n", quoted(key), quoted(value)));
                }
            }
            if task.explicit("healthcheck")
                && let Some(healthcheck) = &task.healthcheck
            {
                append_healthcheck_yaml(&mut text, healthcheck);
            }
            if task.explicit("success_exit_codes") {
                yaml_i32_array(&mut text, 4, "success_exit_codes", &task.success_exit_codes);
            }
            append_task_dependencies(&mut text, task);
            yaml_origin_value(&mut text, task, "restart", &task.restart);
            yaml_origin_duration(
                &mut text,
                task,
                "restart_delay_ms",
                "restart_delay",
                task.restart_delay_ms,
            );
            yaml_origin_value(&mut text, task, "max_restarts", &task.max_restarts);
            yaml_origin_duration(
                &mut text,
                task,
                "restart_reset_after_ms",
                "restart_reset_after",
                task.restart_reset_after_ms,
            );
            yaml_origin_duration(
                &mut text,
                task,
                "shutdown_timeout_ms",
                "shutdown_timeout",
                task.shutdown_timeout_ms,
            );
        }
        for (id, task) in &self.inactive_tasks {
            text.push_str(&format!(
                "  {}: {}\n",
                quoted(id),
                serde_json::to_string(task).expect("未准入 Task 可序列化为 YAML 兼容 JSON")
            ));
        }
        text
    }
}

/// 以名称数组或条件标量 map 追加紧凑 Task 依赖。
#[allow(clippy::format_push_string)]
fn append_task_dependencies(text: &mut String, task: &FormTask) {
    if task.depends_on.is_empty() {
        return;
    }
    if task
        .depends_on
        .values()
        .all(|dependency| dependency.condition == "started")
    {
        let names = task
            .depends_on
            .keys()
            .map(|name| quoted(name))
            .collect::<Vec<_>>();
        text.push_str(&format!("    depends_on: [{}]\n", names.join(", ")));
    } else {
        text.push_str("    depends_on:\n");
        for (name, dependency) in &task.depends_on {
            text.push_str(&format!(
                "      {}: {}\n",
                quoted(name),
                dependency.condition
            ));
        }
    }
}

/// 向 YAML 追加项目级管理依赖。
#[allow(clippy::format_push_string)]
fn append_dependencies(text: &mut String, config: &FormConfig) {
    if config.dependencies.is_empty() {
        return;
    }
    text.push_str("dependencies:\n");
    for (id, dependency) in &config.dependencies {
        text.push_str(&format!("  {}:\n", quoted(id)));
        text.push_str(&format!("    source: {}\n", quoted(&dependency.source)));
        text.push_str(&format!("    version: {}\n", quoted(&dependency.version)));
        optional_yaml(text, 4, "checksum", dependency.checksum.as_deref());
        text.push_str(&format!("    unpack: {}\n", dependency.unpack));
        optional_yaml(text, 4, "path", dependency.path.as_deref());
        text.push_str(&format!("    kind: {}\n", dependency.kind));
        if let Some(verify) = &dependency.verify {
            text.push_str("    verify:\n");
            optional_yaml(text, 6, "command", verify.command.as_deref());
            yaml_array(text, 6, "args", &verify.args);
            optional_yaml(text, 6, "contains", verify.contains.as_deref());
        }
    }
}

/// 按 Task 本地声明来源输出命令和参数。
#[allow(clippy::format_push_string)]
fn append_command(text: &mut String, task: &FormTask) {
    if task.explicit("command") {
        if task.args.is_empty() {
            text.push_str(&format!("    command: {}\n", quoted(&task.command)));
            if task.explicit("args") {
                text.push_str("    args: []\n");
            }
        } else {
            let argv = std::iter::once(&task.command)
                .chain(&task.args)
                .collect::<Vec<_>>();
            text.push_str(&format!(
                "    command: {}\n",
                serde_json::to_string(&argv).expect("argv 序列化不会失败")
            ));
        }
    } else if task.explicit("args") {
        if task.args.is_empty() {
            text.push_str("    args: []\n");
        } else {
            yaml_array(text, 4, "args", &task.args);
        }
    }
}

/// 仅在字段由 Task 显式声明时输出 YAML 标量。
#[allow(clippy::format_push_string)]
fn yaml_origin_value(
    text: &mut String,
    task: &FormTask,
    key: &str,
    value: &impl std::fmt::Display,
) {
    if task.explicit(key) {
        text.push_str(&format!("    {key}: {value}\n"));
    }
}

/// 仅在字段由 Task 显式声明时输出 YAML 可读时长。
#[allow(clippy::format_push_string)]
fn yaml_origin_duration(
    text: &mut String,
    task: &FormTask,
    origin_key: &str,
    output_key: &str,
    value: u64,
) {
    if task.explicit(origin_key) {
        text.push_str(&format!(
            "    {output_key}: {}\n",
            quoted(&crate::config::format_duration(value))
        ));
    }
}

/// 向 YAML Task 条目追加互斥的 exec 或 HTTP 健康检查。
#[allow(clippy::format_push_string)]
fn append_healthcheck_yaml(text: &mut String, healthcheck: &FormHealthCheck) {
    text.push_str("    healthcheck:\n");
    if let Some(command) = &healthcheck.command {
        text.push_str(&format!("      command: {}\n", quoted(command)));
        yaml_array(text, 6, "args", &healthcheck.args);
        optional_yaml(text, 6, "cwd", healthcheck.cwd.as_deref());
    }
    if let Some(http_get) = &healthcheck.http_get {
        text.push_str("      http_get:\n");
        text.push_str(&format!(
            "        scheme: {}\n        host: {}\n",
            http_get.scheme,
            quoted(&http_get.host)
        ));
        if let Some(port) = http_get.port {
            text.push_str(&format!("        port: {port}\n"));
        }
        text.push_str(&format!("        path: {}\n", quoted(&http_get.path)));
        if !http_get.headers.is_empty() {
            text.push_str("        headers:\n");
            for (name, value) in &http_get.headers {
                text.push_str(&format!("          {}: {}\n", quoted(name), quoted(value)));
            }
        }
        text.push_str(&format!("        status_code: {}\n", http_get.status_code));
    }
    text.push_str(&format!(
        "      initial_delay: {}\n      period: {}\n      timeout: {}\n",
        quoted(&crate::config::format_duration(
            healthcheck.initial_delay_ms
        )),
        quoted(&crate::config::format_duration(healthcheck.period_ms)),
        quoted(&crate::config::format_duration(healthcheck.timeout_ms))
    ));
    text.push_str(&format!(
        "      success_threshold: {}\n      failure_threshold: {}\n",
        healthcheck.success_threshold, healthcheck.failure_threshold
    ));
}

/// 输出 JSON 风格安全双引号字符串，YAML 同样支持该转义形式。
fn quoted(value: &str) -> String {
    serde_json::to_string(value).expect("字符串序列化不会失败")
}

/// 输出可选 YAML 字符串字段。
#[allow(clippy::format_push_string)]
fn optional_yaml(text: &mut String, indent: usize, key: &str, value: Option<&str>) {
    if let Some(value) = value {
        text.push_str(&format!(
            "{}{}: {}\n",
            " ".repeat(indent),
            key,
            quoted(value)
        ));
    }
}

/// 输出 YAML 字符串数组。
#[allow(clippy::format_push_string)]
fn yaml_array(text: &mut String, indent: usize, key: &str, values: &[String]) {
    if values.is_empty() {
        return;
    }
    text.push_str(&format!("{}{}:\n", " ".repeat(indent), key));
    for value in values {
        text.push_str(&format!("{}- {}\n", " ".repeat(indent + 2), quoted(value)));
    }
}

/// 输出 YAML 整数数组。
#[allow(clippy::format_push_string)]
fn yaml_i32_array(text: &mut String, indent: usize, key: &str, values: &[i32]) {
    text.push_str(&format!("{}{}:\n", " ".repeat(indent), key));
    for value in values {
        text.push_str(&format!("{}- {value}\n", " ".repeat(indent + 2)));
    }
}
