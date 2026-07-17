//! 项目管理依赖的原始 DTO 与规范化规则。

use std::{
    collections::BTreeMap,
    path::{Component, Path, PathBuf},
};

use serde::Deserialize;

use crate::config::{
    ConfigDiagnostic, DependencyDownloadSpec, DependencySshSpec, DependencyVerifySpec,
    ManagedDependencies, ManagedDependencySpec,
};

use super::{diagnostic, required_text, valid_dependency_id};

/// 配置前端支持一行来源或完整下载对象。
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(super) enum RawManagedDependency {
    /// `name: https://example.com/file` 一行简写。
    Source(String),
    /// 带版本、校验或高级传输策略的完整对象。
    Detailed(Box<RawManagedDependencyFields>),
}

/// 完整项目依赖 DTO。
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawManagedDependencyFields {
    pub(super) source: Option<String>,
    #[serde(default)]
    pub(super) mirrors: Vec<String>,
    version: Option<String>,
    checksum: Option<String>,
    #[serde(default)]
    unpack: RawUnpackMode,
    path: Option<PathBuf>,
    #[serde(default)]
    kind: RawDependencyKind,
    verify: Option<RawDependencyVerify>,
    #[serde(default)]
    download: RawDependencyDownload,
    #[serde(default)]
    pub(super) ssh: RawDependencySsh,
}

impl RawManagedDependency {
    /// 把一行来源转换为使用全部默认值的完整 DTO。
    fn into_fields(self) -> RawManagedDependencyFields {
        match self {
            Self::Source(source) => RawManagedDependencyFields {
                source: Some(source),
                ..RawManagedDependencyFields::default()
            },
            Self::Detailed(fields) => *fields,
        }
    }
}

/// 配置前端反序列化使用的 SSH 认证 DTO。
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawDependencySsh {
    pub(super) identity_file: Option<PathBuf>,
    pub(super) known_hosts_file: Option<PathBuf>,
}

/// 配置前端反序列化使用的下载策略 DTO。
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawDependencyDownload {
    #[serde(default = "default_download_retries")]
    retries: u8,
    #[serde(
        default = "default_download_timeout_ms",
        rename = "timeout",
        alias = "timeout_ms",
        deserialize_with = "crate::config::deserialize_duration"
    )]
    timeout_ms: u64,
    #[serde(default = "default_download_max_bytes")]
    max_bytes: u64,
    #[serde(default)]
    headers: BTreeMap<String, String>,
}

impl Default for RawDependencyDownload {
    fn default() -> Self {
        Self {
            retries: default_download_retries(),
            timeout_ms: default_download_timeout_ms(),
            max_bytes: default_download_max_bytes(),
            headers: BTreeMap::new(),
        }
    }
}

/// 返回远端来源失败后的默认重试次数。
const fn default_download_retries() -> u8 {
    2
}

/// 返回单次远端传输的默认总超时。
const fn default_download_timeout_ms() -> u64 {
    120_000
}

/// 返回单个依赖默认允许的最大下载字节数。
const fn default_download_max_bytes() -> u64 {
    2 * 1024 * 1024 * 1024
}

/// 配置前端反序列化使用的版本验证 DTO。
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawDependencyVerify {
    command: Option<PathBuf>,
    #[serde(default)]
    args: Vec<String>,
    contains: Option<String>,
}

/// 原始依赖内容类型。
#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum RawDependencyKind {
    #[default]
    Auto,
    Binary,
    File,
    Directory,
}

/// 原始依赖解包模式。
#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum RawUnpackMode {
    #[default]
    Auto,
    Never,
}

/// 校验并规范化全部项目级管理依赖。
pub(super) fn normalize_dependencies(
    raw_dependencies: BTreeMap<String, RawManagedDependency>,
    diagnostics: &mut Vec<ConfigDiagnostic>,
) -> ManagedDependencies {
    let mut dependencies = BTreeMap::new();
    for (id, raw) in raw_dependencies {
        let raw = raw.into_fields();
        let field = format!("dependencies.{id}");
        if !valid_dependency_id(&id) {
            diagnostics.push(diagnostic(
                &field,
                "依赖名称只能包含 ASCII 字母、数字、点、短横线和下划线",
            ));
            continue;
        }
        let source = required_text(raw.source, &format!("{field}.source"), diagnostics);
        if !source.is_empty() && !valid_source(&source) {
            diagnostics.push(diagnostic(
                format!("{field}.source"),
                "只支持 http://、https://、ssh://、SCP 地址、file:// 或本地路径",
            ));
        }
        validate_mirrors(&field, &raw.mirrors, diagnostics);
        let version = raw.version.map_or_else(
            || "source".to_owned(),
            |version| required_text(Some(version), &format!("{field}.version"), diagnostics),
        );
        if matches!(version.as_str(), "." | "..")
            || version.contains(['/', '\\'])
            || version.chars().any(char::is_control)
        {
            diagnostics.push(diagnostic(
                format!("{field}.version"),
                "不能包含路径分隔符、控制字符或父目录",
            ));
        }
        if let Some(checksum) = raw.checksum.as_deref()
            && !valid_checksum(checksum)
        {
            diagnostics.push(diagnostic(
                format!("{field}.checksum"),
                "必须是 64 位十六进制 SHA-256，可带 sha256: 前缀",
            ));
        }
        if raw
            .path
            .as_ref()
            .is_some_and(|path| !valid_relative_path(path))
        {
            diagnostics.push(diagnostic(
                format!("{field}.path"),
                "必须是归档内不含父目录的相对路径",
            ));
        }
        let verify = normalize_verify(&field, raw.verify, diagnostics);
        validate_download(&field, &raw.download, diagnostics);
        validate_ssh(&field, &raw.ssh, diagnostics);
        dependencies.insert(
            id,
            ManagedDependencySpec {
                source,
                mirrors: raw.mirrors,
                version,
                checksum: raw.checksum,
                unpack: raw.unpack.into(),
                path: raw.path,
                kind: raw.kind.into(),
                verify,
                download: DependencyDownloadSpec {
                    retries: raw.download.retries,
                    timeout_ms: raw.download.timeout_ms,
                    max_bytes: raw.download.max_bytes,
                    headers: raw.download.headers,
                },
                ssh: DependencySshSpec {
                    identity_file: raw.ssh.identity_file,
                    known_hosts_file: raw.ssh.known_hosts_file,
                },
            },
        );
    }
    dependencies
}

/// 校验镜像数量和每个来源写法。
fn validate_mirrors(field: &str, mirrors: &[String], diagnostics: &mut Vec<ConfigDiagnostic>) {
    if mirrors.len() > 8 {
        diagnostics.push(diagnostic(
            format!("{field}.mirrors"),
            "镜像数量不能超过 8 个",
        ));
    }
    for (index, mirror) in mirrors.iter().enumerate() {
        if mirror.trim().is_empty() || !valid_source(mirror) {
            diagnostics.push(diagnostic(
                format!("{field}.mirrors.{index}"),
                "只支持非空的 http://、https://、ssh://、SCP 地址、file:// 或本地路径",
            ));
        }
    }
}

/// 规范化并校验版本命令。
fn normalize_verify(
    field: &str,
    verify: Option<RawDependencyVerify>,
    diagnostics: &mut Vec<ConfigDiagnostic>,
) -> Option<DependencyVerifySpec> {
    let verify = verify.map(|verify| DependencyVerifySpec {
        command: verify.command,
        args: verify.args,
        contains: verify.contains,
    });
    if verify
        .as_ref()
        .and_then(|verify| verify.command.as_ref())
        .is_some_and(|path| !valid_relative_path(path))
    {
        diagnostics.push(diagnostic(
            format!("{field}.verify.command"),
            "必须是安装根目录内不含父目录的相对路径",
        ));
    }
    verify
}

/// 校验下载重试、超时、大小和请求头边界。
fn validate_download(
    field: &str,
    download: &RawDependencyDownload,
    diagnostics: &mut Vec<ConfigDiagnostic>,
) {
    if download.retries > 10 {
        diagnostics.push(diagnostic(
            format!("{field}.download.retries"),
            "不能超过 10 次",
        ));
    }
    if !(1_000..=30 * 60 * 1_000).contains(&download.timeout_ms) {
        diagnostics.push(diagnostic(
            format!("{field}.download.timeout"),
            "必须在 1s 到 30m 之间",
        ));
    }
    if !(1..=64 * 1024 * 1024 * 1024).contains(&download.max_bytes) {
        diagnostics.push(diagnostic(
            format!("{field}.download.max_bytes"),
            "必须在 1 字节到 64 GiB 之间",
        ));
    }
    validate_download_headers(field, &download.headers, diagnostics);
}

/// 校验 SSH 辅助文件路径。
fn validate_ssh(field: &str, ssh: &RawDependencySsh, diagnostics: &mut Vec<ConfigDiagnostic>) {
    for (name, path) in [
        ("identity_file", ssh.identity_file.as_ref()),
        ("known_hosts_file", ssh.known_hosts_file.as_ref()),
    ] {
        if path.is_some_and(|path| !valid_ssh_path(path)) {
            diagnostics.push(diagnostic(
                format!("{field}.ssh.{name}"),
                "必须是绝对路径或不含父目录的服务相对路径",
            ));
        }
    }
}

/// 校验 HTTP 下载请求头的数量、名称、值和总大小。
fn validate_download_headers(
    field: &str,
    headers: &BTreeMap<String, String>,
    diagnostics: &mut Vec<ConfigDiagnostic>,
) {
    if headers.len() > 32 {
        diagnostics.push(diagnostic(
            format!("{field}.download.headers"),
            "请求头数量不能超过 32 个",
        ));
    }
    let mut total = 0_usize;
    for (name, value) in headers {
        total = total.saturating_add(name.len()).saturating_add(value.len());
        if name.is_empty() || name.len() > 128 || !name.bytes().all(is_http_token_byte) {
            diagnostics.push(diagnostic(
                format!("{field}.download.headers.{name}"),
                "请求头名称不是合法 HTTP token 或超过 128 字节",
            ));
        }
        if value.len() > 8 * 1024
            || value
                .bytes()
                .any(|byte| byte == b'\r' || byte == b'\n' || byte == 0)
        {
            diagnostics.push(diagnostic(
                format!("{field}.download.headers.{name}"),
                "请求头值不能包含换行或 NUL，且不能超过 8 KiB",
            ));
        }
    }
    if total > 16 * 1024 {
        diagnostics.push(diagnostic(
            format!("{field}.download.headers"),
            "请求头名称和值的总大小不能超过 16 KiB",
        ));
    }
}

/// 判断字节是否可用于 HTTP token。
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

/// 判断来源是否属于支持的网络、SSH 或本地形式。
fn valid_source(value: &str) -> bool {
    value.starts_with("http://")
        || value.starts_with("https://")
        || value.starts_with("ssh://")
        || value.starts_with("file://")
        || (!value.contains("://")
            && value
                .split_once(':')
                .is_some_and(|(host, path)| !host.contains('/') && path.starts_with('/')))
        || !value.contains("://")
}

/// 判断 SHA-256 字符串格式是否合法。
fn valid_checksum(value: &str) -> bool {
    let value = value.strip_prefix("sha256:").unwrap_or(value);
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// 判断配置路径是否为不含点或父目录分量的相对路径。
fn valid_relative_path(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

/// 判断 SSH 辅助文件是否为绝对路径或安全的服务相对路径。
fn valid_ssh_path(path: &Path) -> bool {
    path.is_absolute() || valid_relative_path(path)
}
