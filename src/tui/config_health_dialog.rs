use std::collections::BTreeMap;

use super::{
    config_form::{FormConfig, FormHealthCheck, FormHttpHealthCheck, FormTask},
    config_form_dialog::{
        DialogField, field, map_field, optional, parse_args, parse_duration, parse_map, parse_u32,
        required_value,
    },
};
use crate::{
    config::ValueOrigin,
    core::{HealthCheckProbe, HealthCheckSpec},
};

impl FormTask {
    /// 返回详情区使用的健康检查类型摘要。
    pub(crate) fn health_label(&self) -> &'static str {
        self.healthcheck.as_ref().map_or("未配置", |health| {
            if health.http_get.is_some() {
                "HTTP GET"
            } else {
                "Exec"
            }
        })
    }
}

/// 把规范化健康检查转换为仍可编辑的表单值。
pub(super) fn from_spec(healthcheck: HealthCheckSpec) -> FormHealthCheck {
    let (command, args, cwd, http_get) = match healthcheck.probe {
        HealthCheckProbe::Exec { command, args, cwd } => (
            Some(command),
            args,
            cwd.map(|path| path.display().to_string()),
            None,
        ),
        HealthCheckProbe::HttpGet { http_get } => (
            None,
            Vec::new(),
            None,
            Some(FormHttpHealthCheck {
                scheme: http_get.scheme.as_str().to_owned(),
                host: http_get.host,
                port: http_get.port,
                path: http_get.path,
                headers: http_get.headers,
                status_code: http_get.status_code,
            }),
        ),
    };
    FormHealthCheck {
        command,
        args,
        cwd,
        http_get,
        initial_delay_ms: healthcheck.initial_delay_ms,
        period_ms: healthcheck.period_ms,
        timeout_ms: healthcheck.timeout_ms,
        success_threshold: healthcheck.success_threshold,
        failure_threshold: healthcheck.failure_threshold,
    }
}

/// 构造健康检查弹窗的全部字段，未配置时显示可直接采用的默认值。
pub(super) fn fields(task: &FormTask) -> Vec<DialogField> {
    let health = task.healthcheck.as_ref();
    let kind = health.map_or("none", |health| {
        if health.http_get.is_some() {
            "http"
        } else {
            "exec"
        }
    });
    let http = health.and_then(|health| health.http_get.as_ref());
    vec![
        field("类型", kind, &["none", "exec", "http"]),
        field(
            "Exec 命令",
            health
                .and_then(|health| health.command.as_deref())
                .unwrap_or(""),
            &[],
        ),
        field(
            "Exec 参数（JSON 数组或空格分隔）",
            &serde_json::to_string(&health.map_or_else(Vec::new, |health| health.args.clone()))
                .expect("字符串数组序列化不会失败"),
            &[],
        ),
        field(
            "Exec 工作目录（可空）",
            health
                .and_then(|health| health.cwd.as_deref())
                .unwrap_or(""),
            &[],
        ),
        field(
            "HTTP 协议",
            http.map_or("http", |http| http.scheme.as_str()),
            &["http", "https"],
        ),
        field(
            "HTTP 主机",
            http.map_or("127.0.0.1", |http| &http.host),
            &[],
        ),
        field(
            "HTTP 端口（可空）",
            &http
                .and_then(|http| http.port)
                .map_or_else(String::new, |port| port.to_string()),
            &[],
        ),
        field("HTTP 路径", http.map_or("/", |http| &http.path), &[]),
        map_field(
            "HTTP 请求头（按 F4 编辑键值表）",
            &http.map_or_else(BTreeMap::new, |http| http.headers.clone()),
        ),
        field(
            "HTTP 预期状态码",
            &http.map_or(200, |http| http.status_code).to_string(),
            &[],
        ),
        field(
            "首次检查等待（如 0ms/2s）",
            &crate::config::format_duration(health.map_or(0, |health| health.initial_delay_ms)),
            &[],
        ),
        field(
            "检查周期（如 10s）",
            &crate::config::format_duration(health.map_or(10_000, |health| health.period_ms)),
            &[],
        ),
        field(
            "单次检查超时（如 1s）",
            &crate::config::format_duration(health.map_or(1_000, |health| health.timeout_ms)),
            &[],
        ),
        field(
            "连续成功阈值",
            &health
                .map_or(1, |health| health.success_threshold)
                .to_string(),
            &[],
        ),
        field(
            "连续失败阈值",
            &health
                .map_or(3, |health| health.failure_threshold)
                .to_string(),
            &[],
        ),
    ]
}

/// 校验健康检查字段并更新指定 Task。
pub(super) fn commit(
    task_name: &str,
    fields: &[DialogField],
    config: &mut FormConfig,
) -> Result<(), String> {
    let task = config
        .tasks
        .get_mut(task_name)
        .ok_or_else(|| format!("Task `{task_name}` 已不存在"))?;
    task.healthcheck = match fields[0].value.as_str() {
        "none" => None,
        "exec" => Some(FormHealthCheck {
            command: Some(required_value(&fields[1].value, "Exec 命令")?),
            args: parse_args(&fields[2].value, "Exec 参数")?,
            cwd: optional(&fields[3].value),
            http_get: None,
            initial_delay_ms: parse_duration(&fields[10].value, "首次检查等待")?,
            period_ms: parse_duration(&fields[11].value, "检查周期")?,
            timeout_ms: parse_duration(&fields[12].value, "单次检查超时")?,
            success_threshold: parse_u32(&fields[13].value, "连续成功阈值")?,
            failure_threshold: parse_u32(&fields[14].value, "连续失败阈值")?,
        }),
        "http" => Some(FormHealthCheck {
            command: None,
            args: Vec::new(),
            cwd: None,
            http_get: Some(FormHttpHealthCheck {
                scheme: fields[4].value.clone(),
                host: required_value(&fields[5].value, "HTTP 主机")?,
                port: parse_optional_u16(&fields[6].value, "HTTP 端口")?,
                path: required_value(&fields[7].value, "HTTP 路径")?,
                headers: parse_map(&fields[8].value, "HTTP 请求头")?,
                status_code: parse_u16(&fields[9].value, "HTTP 预期状态码")?,
            }),
            initial_delay_ms: parse_duration(&fields[10].value, "首次检查等待")?,
            period_ms: parse_duration(&fields[11].value, "检查周期")?,
            timeout_ms: parse_duration(&fields[12].value, "单次检查超时")?,
            success_threshold: parse_u32(&fields[13].value, "连续成功阈值")?,
            failure_threshold: parse_u32(&fields[14].value, "连续失败阈值")?,
        }),
        _ => return Err("健康检查类型只能是 none、exec 或 http".to_owned()),
    };
    task.origins.fields.insert(
        "healthcheck".to_owned(),
        if task.healthcheck.is_some() {
            ValueOrigin::Task
        } else {
            ValueOrigin::BuiltIn
        },
    );
    Ok(())
}

/// 解析可空端口字段。
fn parse_optional_u16(value: &str, label: &str) -> Result<Option<u16>, String> {
    optional(value)
        .map(|value| parse_u16(&value, label))
        .transpose()
}

/// 解析 16 位无符号整数。
fn parse_u16(value: &str, label: &str) -> Result<u16, String> {
    value
        .trim()
        .parse()
        .map_err(|_| format!("{label} 必须是 0–65535 的整数"))
}
