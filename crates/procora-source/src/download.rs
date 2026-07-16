use std::{
    fs,
    io::{self, Read},
    path::Path,
    process::{Command, Stdio},
    time::Duration,
};

use crate::manager::SourceError;

/// 把支持的远端或本地来源保存到指定文件。
pub(crate) fn fetch(source: &str, destination: &Path) -> Result<(), SourceError> {
    if source.starts_with("http://") || source.starts_with("https://") {
        return fetch_http(source, destination);
    }
    if let Some(path) = source.strip_prefix("file://") {
        fs::copy(path, destination).map_err(|source_error| SourceError::Download {
            location: source.to_owned(),
            message: source_error.to_string(),
        })?;
        return Ok(());
    }
    if source.starts_with("ssh://") || is_scp_source(source) {
        return fetch_ssh(source, destination);
    }
    fs::copy(source, destination).map_err(|source_error| SourceError::Download {
        location: source.to_owned(),
        message: source_error.to_string(),
    })?;
    Ok(())
}

/// 返回尽可能保留归档扩展名的安全下载文件名。
pub(crate) fn source_filename(source: &str, fallback: &str) -> String {
    let clean = source.split(['?', '#']).next().unwrap_or(source);
    let candidate = clean.rsplit(['/', ':']).next().unwrap_or_default();
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

/// 使用阻塞 HTTP 客户端流式写入下载内容。
fn fetch_http(source: &str, destination: &Path) -> Result<(), SourceError> {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(15))
        .timeout_read(Duration::from_mins(1))
        .timeout_write(Duration::from_mins(1))
        .build();
    let response = agent
        .get(source)
        .call()
        .map_err(|error| SourceError::Download {
            location: source.to_owned(),
            message: error.to_string(),
        })?;
    let mut reader = response.into_reader();
    let mut output = fs::File::create(destination)?;
    io::copy(&mut reader, &mut output)?;
    Ok(())
}

/// 使用本机 OpenSSH 的 scp 客户端下载 SSH 来源。
fn fetch_ssh(source: &str, destination: &Path) -> Result<(), SourceError> {
    let scp_source = ssh_url_to_scp(source).unwrap_or_else(|| source.to_owned());
    let mut child = Command::new("scp")
        .arg("-q")
        .args(["-o", "BatchMode=yes", "-o", "ConnectTimeout=15"])
        .arg("--")
        .arg(&scp_source)
        .arg(destination)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| SourceError::Download {
            location: source.to_owned(),
            message: format!("无法启动 scp：{error}"),
        })?;
    let mut stderr = String::new();
    if let Some(mut stream) = child.stderr.take() {
        stream.read_to_string(&mut stderr)?;
    }
    let status = child.wait()?;
    if !status.success() {
        return Err(SourceError::Download {
            location: source.to_owned(),
            message: stderr.trim().to_owned(),
        });
    }
    Ok(())
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
        return Some(format!("scp://{authority}/{path}"));
    }
    Some(format!("{authority}:/{path}"))
}
