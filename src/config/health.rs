use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::core::{HealthCheckProbe, HealthCheckSpec, HttpHealthCheckSpec, HttpScheme};

use super::ConfigDiagnostic;

/// 配置前端反序列化使用的原始健康检查 DTO。
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawHealthCheck {
    #[serde(skip_serializing_if = "Option::is_none")]
    command: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    args: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cwd: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    http_get: Option<RawHttpHealthCheck>,
    #[serde(default)]
    initial_delay_ms: u64,
    #[serde(default = "default_period_ms")]
    period_ms: u64,
    #[serde(default = "default_timeout_ms")]
    timeout_ms: u64,
    #[serde(default = "default_success_threshold")]
    success_threshold: u32,
    #[serde(default = "default_failure_threshold")]
    failure_threshold: u32,
}

/// 配置前端反序列化使用的原始 HTTP GET 探针。
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RawHttpHealthCheck {
    #[serde(default)]
    scheme: HttpScheme,
    #[serde(default = "default_http_host")]
    host: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    port: Option<u16>,
    #[serde(default = "default_http_path")]
    path: String,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    headers: std::collections::BTreeMap<String, String>,
    #[serde(default = "default_http_status_code")]
    status_code: u16,
}

impl RawHealthCheck {
    /// 把 include 片段内的相对工作目录改写为声明文件目录路径。
    pub(super) fn rebase(&mut self, base: &Path) {
        self.cwd = self
            .cwd
            .take()
            .map(|path| normalize_path(&path, Some(base)));
    }
}

/// 校验并规范化可选健康检查。
pub(super) fn normalize_healthcheck(
    raw: Option<RawHealthCheck>,
    task_path: &str,
    base_directory: Option<&Path>,
    diagnostics: &mut Vec<ConfigDiagnostic>,
) -> Option<HealthCheckSpec> {
    let raw = raw?;
    let path = format!("{task_path}.healthcheck");
    let probe = normalize_probe(
        raw.command,
        raw.args,
        raw.cwd,
        raw.http_get,
        &path,
        base_directory,
        diagnostics,
    );
    bounded_duration(raw.period_ms, &format!("{path}.period_ms"), diagnostics);
    bounded_duration(raw.timeout_ms, &format!("{path}.timeout_ms"), diagnostics);
    if raw.initial_delay_ms > MAX_DURATION_MS {
        diagnostics.push(diagnostic(
            format!("{path}.initial_delay_ms"),
            format!("不能超过 {MAX_DURATION_MS} 毫秒"),
        ));
    }
    bounded_threshold(
        raw.success_threshold,
        &format!("{path}.success_threshold"),
        diagnostics,
    );
    bounded_threshold(
        raw.failure_threshold,
        &format!("{path}.failure_threshold"),
        diagnostics,
    );
    Some(HealthCheckSpec {
        probe,
        initial_delay_ms: raw.initial_delay_ms,
        period_ms: raw.period_ms,
        timeout_ms: raw.timeout_ms,
        success_threshold: raw.success_threshold,
        failure_threshold: raw.failure_threshold,
    })
}

/// 校验互斥的 exec 与 HTTP GET 探针并生成领域值。
fn normalize_probe(
    command: Option<String>,
    args: Vec<String>,
    cwd: Option<PathBuf>,
    http_get: Option<RawHttpHealthCheck>,
    path: &str,
    base_directory: Option<&Path>,
    diagnostics: &mut Vec<ConfigDiagnostic>,
) -> HealthCheckProbe {
    if command.is_some() && http_get.is_some() {
        diagnostics.push(diagnostic(path, "command 与 http_get 只能配置其中一种"));
    }
    if http_get.is_some() && (!args.is_empty() || cwd.is_some()) {
        diagnostics.push(diagnostic(path, "args 与 cwd 只适用于 command 探针"));
    }
    let http_get = http_get.map(|http_get| normalize_http_probe(http_get, path, diagnostics));
    if let Some(command) = command {
        if command.trim().is_empty() {
            diagnostics.push(diagnostic(format!("{path}.command"), "命令不能为空"));
        }
        return HealthCheckProbe::Exec {
            command,
            args,
            cwd: cwd.map(|path| normalize_path(&path, base_directory)),
        };
    }
    if let Some(http_get) = http_get {
        return HealthCheckProbe::HttpGet { http_get };
    }
    diagnostics.push(diagnostic(path, "必须配置 command 或 http_get"));
    HealthCheckProbe::Exec {
        command: String::new(),
        args,
        cwd: cwd.map(|path| normalize_path(&path, base_directory)),
    }
}

/// 校验 HTTP 请求目标、头部和预期状态码。
fn normalize_http_probe(
    raw: RawHttpHealthCheck,
    health_path: &str,
    diagnostics: &mut Vec<ConfigDiagnostic>,
) -> HttpHealthCheckSpec {
    let path = format!("{health_path}.http_get");
    let host = raw.host.trim().to_owned();
    if !valid_http_host(&host) {
        diagnostics.push(diagnostic(
            format!("{path}.host"),
            "必须是主机名、IP 地址或带方括号的 IPv6 地址",
        ));
    }
    if raw.port == Some(0) {
        diagnostics.push(diagnostic(format!("{path}.port"), "必须大于零"));
    }
    if !raw.path.starts_with('/') {
        diagnostics.push(diagnostic(format!("{path}.path"), "必须以 / 开头"));
    } else if raw.path.len() > MAX_HTTP_PATH_BYTES
        || raw
            .path
            .bytes()
            .any(|byte| byte.is_ascii_control() || byte == b' ')
    {
        diagnostics.push(diagnostic(
            format!("{path}.path"),
            format!("不能超过 {MAX_HTTP_PATH_BYTES} 字节且不能包含空格或控制字符"),
        ));
    }
    if !(100..400).contains(&raw.status_code) {
        diagnostics.push(diagnostic(
            format!("{path}.status_code"),
            "必须在 100–399 之间",
        ));
    }
    validate_http_headers(&raw.headers, &path, diagnostics);
    HttpHealthCheckSpec {
        scheme: raw.scheme,
        host,
        port: raw.port,
        path: raw.path,
        headers: raw.headers,
        status_code: raw.status_code,
    }
}

/// 校验有界且不会注入换行的 HTTP 请求头。
fn validate_http_headers(
    headers: &std::collections::BTreeMap<String, String>,
    path: &str,
    diagnostics: &mut Vec<ConfigDiagnostic>,
) {
    if headers.len() > MAX_HTTP_HEADERS {
        diagnostics.push(diagnostic(
            format!("{path}.headers"),
            format!("不能超过 {MAX_HTTP_HEADERS} 个请求头"),
        ));
    }
    let mut total = 0_usize;
    for (name, value) in headers {
        total = total.saturating_add(name.len()).saturating_add(value.len());
        if name.is_empty()
            || name.len() > MAX_HTTP_HEADER_NAME_BYTES
            || !name.bytes().all(is_http_token_byte)
        {
            diagnostics.push(diagnostic(
                format!("{path}.headers.{name}"),
                "请求头名称格式无效",
            ));
        }
        if value.len() > MAX_HTTP_HEADER_VALUE_BYTES
            || value
                .bytes()
                .any(|byte| byte != b'\t' && (byte < b' ' || byte == 0x7f))
        {
            diagnostics.push(diagnostic(
                format!("{path}.headers.{name}"),
                "请求头值过长或包含控制字符",
            ));
        }
    }
    if total > MAX_HTTP_HEADERS_TOTAL_BYTES {
        diagnostics.push(diagnostic(
            format!("{path}.headers"),
            format!("请求头总计不能超过 {MAX_HTTP_HEADERS_TOTAL_BYTES} 字节"),
        ));
    }
}

/// 判断主机字段是否能安全拼入不含用户信息的 URL authority。
fn valid_http_host(host: &str) -> bool {
    if host.is_empty() || host.len() > MAX_HTTP_HOST_BYTES {
        return false;
    }
    if host.parse::<std::net::IpAddr>().is_ok() {
        return true;
    }
    if host
        .strip_prefix('[')
        .and_then(|host| host.strip_suffix(']'))
        .is_some_and(|host| host.parse::<std::net::Ipv6Addr>().is_ok())
    {
        return true;
    }
    host.split('.').all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    })
}

/// 判断字节是否属于 RFC HTTP token 可用字符。
fn is_http_token_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
        || matches!(
            byte,
            b'!' | b'#'
                | b'$'
                | b'%'
                | b'&'
                | b'\''
                | b'*'
                | b'+'
                | b'-'
                | b'.'
                | b'^'
                | b'_'
                | b'`'
                | b'|'
                | b'~'
        )
}

/// 校验非零且有上限的探针时长。
fn bounded_duration(value: u64, path: &str, diagnostics: &mut Vec<ConfigDiagnostic>) {
    if value == 0 {
        diagnostics.push(diagnostic(path, "必须大于零"));
    } else if value > MAX_DURATION_MS {
        diagnostics.push(diagnostic(path, format!("不能超过 {MAX_DURATION_MS} 毫秒")));
    }
}

/// 校验连续结果阈值，防止无界等待。
fn bounded_threshold(value: u32, path: &str, diagnostics: &mut Vec<ConfigDiagnostic>) {
    if value == 0 {
        diagnostics.push(diagnostic(path, "必须大于零"));
    } else if value > MAX_THRESHOLD {
        diagnostics.push(diagnostic(path, format!("不能超过 {MAX_THRESHOLD}")));
    }
}

/// 按配置目录规范化健康检查工作目录。
fn normalize_path(path: &Path, base_directory: Option<&Path>) -> PathBuf {
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        base_directory.map_or_else(|| path.to_path_buf(), |base| base.join(path))
    };
    let mut normalized = PathBuf::new();
    for component in joined.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

/// 创建健康检查字段诊断。
fn diagnostic(path: impl Into<String>, message: impl Into<String>) -> ConfigDiagnostic {
    ConfigDiagnostic {
        path: path.into(),
        message: message.into(),
    }
}

/// 默认检查周期。
const fn default_period_ms() -> u64 {
    10_000
}

/// 默认检查超时。
const fn default_timeout_ms() -> u64 {
    1_000
}

/// 默认连续成功阈值。
const fn default_success_threshold() -> u32 {
    1
}

/// 默认连续失败阈值。
const fn default_failure_threshold() -> u32 {
    3
}

/// 单次时长配置上限。
const MAX_DURATION_MS: u64 = 300_000;

/// 连续结果阈值上限。
const MAX_THRESHOLD: u32 = 100;

/// 默认 HTTP 检查主机。
fn default_http_host() -> String {
    "127.0.0.1".to_owned()
}

/// 默认 HTTP 检查路径。
fn default_http_path() -> String {
    "/".to_owned()
}

/// 默认 HTTP 检查成功状态码。
const fn default_http_status_code() -> u16 {
    200
}

/// HTTP 主机名最大字节数。
const MAX_HTTP_HOST_BYTES: usize = 253;
/// HTTP 请求路径最大字节数。
const MAX_HTTP_PATH_BYTES: usize = 2048;
/// 单次 HTTP 检查请求头数量上限。
const MAX_HTTP_HEADERS: usize = 32;
/// HTTP 请求头名称最大字节数。
const MAX_HTTP_HEADER_NAME_BYTES: usize = 128;
/// HTTP 请求头值最大字节数。
const MAX_HTTP_HEADER_VALUE_BYTES: usize = 1024;
/// HTTP 请求头名称和值总字节数上限。
const MAX_HTTP_HEADERS_TOTAL_BYTES: usize = 8192;
