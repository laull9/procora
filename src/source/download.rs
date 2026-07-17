use std::{
    collections::BTreeMap,
    fs,
    io::{self, Read, Write},
    path::Path,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use sha2::{Digest, Sha256};

use crate::config::{DependencyDownloadSpec, DependencySshSpec};

use super::manager::SourceError;

/// 一次成功下载产生的内容身份。
pub(crate) struct DownloadOutcome {
    /// 下载内容的十六进制 SHA-256。
    pub(crate) sha256: String,
}

/// 按主来源和镜像顺序下载，并对瞬时远端故障执行有界重试。
pub(crate) fn fetch(
    sources: &[String],
    destination: &Path,
    policy: &DependencyDownloadSpec,
    ssh: &DependencySshSpec,
) -> Result<DownloadOutcome, SourceError> {
    let mut failures = Vec::new();
    for source in sources {
        let attempts = if is_remote(source) {
            usize::from(policy.retries) + 1
        } else {
            1
        };
        let mut last_failure = None;
        for attempt in 0..attempts {
            match fetch_once(source, destination, policy, ssh) {
                Ok(sha256) => return Ok(DownloadOutcome { sha256 }),
                Err(failure) => {
                    let retryable = failure.retryable;
                    last_failure = Some(failure.message);
                    if !retryable || attempt + 1 == attempts {
                        break;
                    }
                    thread::sleep(retry_delay(attempt));
                }
            }
        }
        failures.push(format!(
            "`{}`（最多尝试 {attempts} 次）：{}",
            display_source(source),
            last_failure.unwrap_or_else(|| "未知下载错误".to_owned())
        ));
    }
    Err(SourceError::Download {
        location: sources
            .iter()
            .map(|source| display_source(source))
            .collect::<Vec<_>>()
            .join("、"),
        message: failures.join("；"),
    })
}

/// 返回尽可能保留归档扩展名的安全下载文件名。
pub(crate) fn source_filename(source: &str, fallback: &str) -> String {
    let clean = if source.contains("://") {
        source.split(['?', '#']).next().unwrap_or(source)
    } else {
        source
    };
    let candidate = clean.rsplit(['/', '\\', ':']).next().unwrap_or_default();
    let safe = candidate
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || ".-_".contains(*character))
        .collect::<String>();
    if safe.is_empty() {
        fallback.to_owned()
    } else {
        safe
    }
}

/// 单次来源失败及其是否适合自动重试。
struct FetchFailure {
    message: String,
    retryable: bool,
}

/// 从一个来源执行单次有界下载。
fn fetch_once(
    source: &str,
    destination: &Path,
    policy: &DependencyDownloadSpec,
    ssh: &DependencySshSpec,
) -> Result<String, FetchFailure> {
    if source.starts_with("http://") || source.starts_with("https://") {
        return fetch_http(source, destination, policy);
    }
    if let Some(path) = source.strip_prefix("file://") {
        return fetch_local(Path::new(path), destination, policy.max_bytes);
    }
    if source.starts_with("ssh://") || is_scp_source(source) {
        return fetch_ssh(source, destination, policy, ssh);
    }
    fetch_local(Path::new(source), destination, policy.max_bytes)
}

/// 使用阻塞 HTTP 客户端流式写入、限量并同步计算摘要。
fn fetch_http(
    source: &str,
    destination: &Path,
    policy: &DependencyDownloadSpec,
) -> Result<String, FetchFailure> {
    let timeout = Duration::from_millis(policy.timeout_ms);
    let agent = ureq::AgentBuilder::new()
        .timeout(timeout)
        .timeout_connect(timeout.min(Duration::from_secs(15)))
        .redirects(if policy.headers.is_empty() { 5 } else { 0 })
        .build();
    let headers = resolve_headers(&policy.headers)?;
    let mut request = agent
        .get(source)
        .set("User-Agent", concat!("procora/", env!("CARGO_PKG_VERSION")));
    for (name, value) in &headers {
        request = request.set(name, value);
    }
    let response = request.call().map_err(http_failure)?;
    if let Some(length) = response
        .header("Content-Length")
        .and_then(|value| value.parse::<u64>().ok())
        && length > policy.max_bytes
    {
        return Err(FetchFailure {
            message: format!(
                "响应声明 {length} 字节，超过配置上限 {} 字节",
                policy.max_bytes
            ),
            retryable: false,
        });
    }
    write_bounded(response.into_reader(), destination, policy.max_bytes)
}

/// 把 HTTP 状态或传输错误转换为可诊断的重试分类。
fn http_failure(error: ureq::Error) -> FetchFailure {
    match error {
        ureq::Error::Status(status, _) => FetchFailure {
            message: format!("HTTP 状态码 {status}"),
            retryable: matches!(status, 408 | 425 | 429 | 500 | 502 | 503 | 504),
        },
        ureq::Error::Transport(error) => FetchFailure {
            message: format!("HTTP 传输错误：{error}"),
            retryable: true,
        },
    }
}

/// 复制本地文件时应用与远端一致的大小边界和流式摘要。
fn fetch_local(source: &Path, destination: &Path, max_bytes: u64) -> Result<String, FetchFailure> {
    let file = fs::File::open(source).map_err(|error| FetchFailure {
        message: error.to_string(),
        retryable: false,
    })?;
    if file
        .metadata()
        .is_ok_and(|metadata| metadata.len() > max_bytes)
    {
        return Err(FetchFailure {
            message: format!("文件超过配置上限 {max_bytes} 字节"),
            retryable: false,
        });
    }
    write_bounded(file, destination, max_bytes)
}

/// 使用本机 OpenSSH 的 scp 客户端执行非交互、有总超时的下载。
fn fetch_ssh(
    source: &str,
    destination: &Path,
    policy: &DependencyDownloadSpec,
    ssh: &DependencySshSpec,
) -> Result<String, FetchFailure> {
    let scp_source = ssh_url_to_scp(source).unwrap_or_else(|| source.to_owned());
    let mut command = Command::new("scp");
    command.arg("-q").arg("-B").args([
        "-o",
        "BatchMode=yes",
        "-o",
        "ConnectTimeout=15",
        "-o",
        "ConnectionAttempts=1",
        "-o",
        "ServerAliveInterval=15",
        "-o",
        "ServerAliveCountMax=3",
        "-o",
        "StrictHostKeyChecking=yes",
        "-o",
        "LogLevel=ERROR",
    ]);
    if let Some(identity) = &ssh.identity_file {
        command.arg("-i").arg(identity);
    }
    if let Some(known_hosts) = &ssh.known_hosts_file {
        command
            .arg("-o")
            .arg(format!("UserKnownHostsFile={}", known_hosts.display()));
    }
    let mut child = command
        .arg("--")
        .arg(&scp_source)
        .arg(destination)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| FetchFailure {
            message: format!("无法启动 scp：{error}"),
            retryable: false,
        })?;
    let deadline = Instant::now() + Duration::from_millis(policy.timeout_ms);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(50)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(FetchFailure {
                    message: format!("scp 超过总超时 {}ms", policy.timeout_ms),
                    retryable: true,
                });
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(FetchFailure {
                    message: format!("等待 scp 失败：{error}"),
                    retryable: true,
                });
            }
        }
    };
    let stderr = child.stderr.take().map_or_else(String::new, |stream| {
        let mut bytes = Vec::new();
        let _ = stream.take(8 * 1024).read_to_end(&mut bytes);
        String::from_utf8_lossy(&bytes).trim().to_owned()
    });
    if !status.success() {
        let retryable = ssh_failure_is_transient(&stderr);
        return Err(FetchFailure {
            message: if stderr.is_empty() {
                format!("scp 退出状态 {status}")
            } else {
                stderr
            },
            retryable,
        });
    }
    let metadata = fs::metadata(destination).map_err(|error| FetchFailure {
        message: format!("scp 未产生可读文件：{error}"),
        retryable: true,
    })?;
    if metadata.len() > policy.max_bytes {
        return Err(FetchFailure {
            message: format!(
                "scp 下载 {} 字节，超过配置上限 {} 字节",
                metadata.len(),
                policy.max_bytes
            ),
            retryable: false,
        });
    }
    hash_file(destination)
}

/// 有界复制输入，在一次磁盘写入中完成 SHA-256 计算并落盘同步。
fn write_bounded(
    mut input: impl Read,
    destination: &Path,
    max_bytes: u64,
) -> Result<String, FetchFailure> {
    let mut output = fs::File::create(destination).map_err(io_failure)?;
    let mut digest = Sha256::new();
    let mut total = 0_u64;
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let read = input.read(&mut buffer).map_err(io_failure)?;
        if read == 0 {
            break;
        }
        total = total.saturating_add(read as u64);
        if total > max_bytes {
            return Err(FetchFailure {
                message: format!("下载内容超过配置上限 {max_bytes} 字节"),
                retryable: false,
            });
        }
        output.write_all(&buffer[..read]).map_err(io_failure)?;
        digest.update(&buffer[..read]);
    }
    output.flush().map_err(io_failure)?;
    output.sync_all().map_err(io_failure)?;
    Ok(format!("{:x}", digest.finalize()))
}

/// 计算 scp 已落盘文件的 SHA-256。
fn hash_file(path: &Path) -> Result<String, FetchFailure> {
    let file = fs::File::open(path).map_err(io_failure)?;
    file.sync_all().map_err(io_failure)?;
    let max = file.metadata().map_err(io_failure)?.len().saturating_add(1);
    write_hash(file, max)
}

/// 只读取输入并计算 SHA-256。
fn write_hash(mut input: impl Read, max_bytes: u64) -> Result<String, FetchFailure> {
    let mut digest = Sha256::new();
    let mut total = 0_u64;
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let read = input.read(&mut buffer).map_err(io_failure)?;
        if read == 0 {
            break;
        }
        total = total.saturating_add(read as u64);
        if total > max_bytes {
            return Err(FetchFailure {
                message: "读取文件时大小发生变化".to_owned(),
                retryable: true,
            });
        }
        digest.update(&buffer[..read]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

/// 把 I/O 错误转换为可重试的传输失败。
fn io_failure(error: io::Error) -> FetchFailure {
    let message = error.to_string();
    drop(error);
    FetchFailure {
        message,
        retryable: true,
    }
}

/// 延迟读取请求头中的 `${env.NAME}`，避免凭据进入配置清单。
fn resolve_headers(
    headers: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, String>, FetchFailure> {
    headers
        .iter()
        .map(|(name, value)| {
            resolve_environment(value)
                .map(|value| (name.clone(), value))
                .map_err(|message| FetchFailure {
                    message: format!("请求头 `{name}`：{message}"),
                    retryable: false,
                })
        })
        .collect()
}

/// 展开字符串中的环境变量引用。
fn resolve_environment(value: &str) -> Result<String, String> {
    let mut output = String::with_capacity(value.len());
    let mut remaining = value;
    while let Some(start) = remaining.find("${env.") {
        output.push_str(&remaining[..start]);
        let expression = &remaining[start + 6..];
        let end = expression
            .find('}')
            .ok_or_else(|| "环境变量引用缺少 `}`".to_owned())?;
        let name = &expression[..end];
        if name.is_empty()
            || !name
                .bytes()
                .all(|byte| byte == b'_' || byte.is_ascii_alphanumeric())
        {
            return Err(format!("环境变量名称 `{name}` 非法"));
        }
        let resolved =
            std::env::var(name).map_err(|_| format!("环境变量 `{name}` 未设置或不是有效文本"))?;
        if resolved
            .bytes()
            .any(|byte| matches!(byte, b'\r' | b'\n' | 0))
        {
            return Err(format!("环境变量 `{name}` 包含请求头禁止字符"));
        }
        output.push_str(&resolved);
        remaining = &expression[end + 1..];
    }
    output.push_str(remaining);
    Ok(output)
}

/// 判断来源是否会使用远端传输。
fn is_remote(source: &str) -> bool {
    source.starts_with("http://")
        || source.starts_with("https://")
        || source.starts_with("ssh://")
        || is_scp_source(source)
}

/// 返回指数增长且有上限的瞬时故障重试间隔。
fn retry_delay(attempt: usize) -> Duration {
    Duration::from_millis(200_u64.saturating_mul(1_u64 << attempt.min(3)))
}

/// 判断 OpenSSH 错误是否更像网络瞬时故障而非认证或配置错误。
fn ssh_failure_is_transient(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    [
        "connection timed out",
        "connection refused",
        "connection reset",
        "connection closed",
        "no route to host",
        "network is unreachable",
        "operation timed out",
        "temporary failure",
    ]
    .iter()
    .any(|needle| message.contains(needle))
}

/// 隐去 URL 查询、片段和内嵌用户信息，避免诊断输出泄露凭据。
fn display_source(source: &str) -> String {
    let clean = source.split(['?', '#']).next().unwrap_or(source);
    let Some((scheme, rest)) = clean.split_once("://") else {
        return clean.to_owned();
    };
    let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
    let authority = authority.rsplit_once('@').map_or_else(
        || authority.to_owned(),
        |(_, host)| format!("<redacted>@{host}"),
    );
    if path.is_empty() {
        format!("{scheme}://{authority}")
    } else {
        format!("{scheme}://{authority}/{path}")
    }
}

/// 识别 `user@host:/path` 形式的 SCP 来源。
fn is_scp_source(source: &str) -> bool {
    !source.contains("://")
        && source
            .split_once(':')
            .is_some_and(|(host, path)| !host.is_empty() && path.starts_with('/'))
}

/// 把 ssh URL 转成 scp 可识别的地址形式。
fn ssh_url_to_scp(source: &str) -> Option<String> {
    let rest = source.strip_prefix("ssh://")?;
    let (authority, path) = rest.split_once('/')?;
    if authority.contains(':') {
        return Some(format!("scp://{authority}//{path}"));
    }
    Some(format!("{authority}:/{path}"))
}
