//! 更新下载地址改写、下载器选择与进度输出。

use std::{
    fs,
    io::{IsTerminal, Read, Write},
    path::{Path, PathBuf},
    process::Command,
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, bail};
use sha2::{Digest, Sha256};

/// 终端进度刷新间隔。
const TERMINAL_REFRESH: Duration = Duration::from_millis(100);
/// 非交互日志刷新间隔。
const LOG_REFRESH: Duration = Duration::from_secs(1);

/// 更新下载器，统一处理镜像、外部程序和内置 HTTP。
pub(super) struct Downloader {
    agent: ureq::Agent,
    mirror: Option<String>,
    command: Option<PathBuf>,
}

impl Downloader {
    /// 创建并校验一次更新使用的下载策略。
    pub(super) fn new(
        agent: ureq::Agent,
        mirror: Option<&str>,
        command: Option<&Path>,
    ) -> anyhow::Result<Self> {
        let mirror = mirror
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(validate_mirror)
            .transpose()?;
        Ok(Self {
            agent,
            mirror,
            command: command.map(Path::to_path_buf),
        })
    }

    /// 下载有界的小型响应到内存。
    pub(super) fn bytes(&self, url: &str, max_bytes: u64, label: &str) -> anyhow::Result<Vec<u8>> {
        let url = self.resolve(url);
        let response = request(&self.agent, &url, label)?;
        enforce_content_length(&response, max_bytes, label)?;
        let mut bytes = Vec::new();
        response
            .into_reader()
            .take(max_bytes + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 > max_bytes {
            bail!("{label}超过允许的 {max_bytes} 字节");
        }
        Ok(bytes)
    }

    /// 下载文件并同步计算小写 SHA-256。
    pub(super) fn file(
        &self,
        url: &str,
        destination: &Path,
        max_bytes: u64,
        expected_bytes: Option<u64>,
        label: &str,
        show_progress: bool,
    ) -> anyhow::Result<String> {
        if expected_bytes.is_some_and(|size| size > max_bytes) {
            bail!("{label}声明超过允许的 {max_bytes} 字节");
        }
        let url = self.resolve(url);
        if let Some(command) = &self.command {
            Self::command_file(
                command,
                &url,
                destination,
                max_bytes,
                expected_bytes,
                label,
                show_progress,
            )
        } else {
            self.http_file(
                &url,
                destination,
                max_bytes,
                expected_bytes,
                label,
                show_progress,
            )
        }
    }

    /// 使用内置 HTTP 客户端流式下载一个文件。
    fn http_file(
        &self,
        url: &str,
        destination: &Path,
        max_bytes: u64,
        expected_bytes: Option<u64>,
        label: &str,
        show_progress: bool,
    ) -> anyhow::Result<String> {
        let response = request(&self.agent, url, label)?;
        enforce_content_length(&response, max_bytes, label)?;
        let content_length = response
            .header("Content-Length")
            .and_then(|value| value.parse::<u64>().ok());
        let total_bytes = expected_bytes.or(content_length);
        let mut input = response.into_reader().take(max_bytes + 1);
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(destination)?;
        let mut digest = Sha256::new();
        let mut buffer = vec![0_u8; 64 * 1024];
        let mut downloaded = 0_u64;
        let mut progress = show_progress.then(|| DownloadProgress::new(label, total_bytes));
        loop {
            let count = input.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            downloaded = downloaded.saturating_add(count as u64);
            if downloaded > max_bytes {
                bail!("{label}超过允许的 {max_bytes} 字节");
            }
            output.write_all(&buffer[..count])?;
            digest.update(&buffer[..count]);
            if let Some(progress) = progress.as_mut() {
                progress.update(downloaded, false);
            }
        }
        output.sync_all()?;
        if let Some(progress) = progress.as_mut() {
            progress.update(downloaded, true);
        }
        Ok(format!("{:x}", digest.finalize()))
    }

    /// 调用外部程序下载，并由 Procora 观察输出文件进度。
    #[allow(clippy::too_many_arguments)]
    fn command_file(
        command: &Path,
        url: &str,
        destination: &Path,
        max_bytes: u64,
        expected_bytes: Option<u64>,
        label: &str,
        show_progress: bool,
    ) -> anyhow::Result<String> {
        let mut child = Command::new(command)
            .arg(url)
            .arg(destination)
            .spawn()
            .with_context(|| format!("无法启动下载程序 `{}`", command.display()))?;
        let mut progress = show_progress.then(|| DownloadProgress::new(label, expected_bytes));
        let status = loop {
            if let Some(status) = child.try_wait()? {
                break status;
            }
            let downloaded = file_size(destination)?;
            if downloaded > max_bytes {
                let _ = child.kill();
                let _ = child.wait();
                bail!("{label}超过允许的 {max_bytes} 字节");
            }
            if let Some(progress) = progress.as_mut() {
                progress.update(downloaded, false);
            }
            thread::sleep(TERMINAL_REFRESH);
        };
        if !status.success() {
            bail!("下载程序 `{}` 失败，退出状态：{status}", command.display());
        }
        let downloaded = file_size(destination)?;
        if downloaded > max_bytes {
            bail!("{label}超过允许的 {max_bytes} 字节");
        }
        if let Some(progress) = progress.as_mut() {
            progress.update(downloaded, true);
        }
        hash_file(destination)
    }

    /// 只改写 GitHub 官方地址，保留显式本地测试或私有来源。
    fn resolve(&self, url: &str) -> String {
        if !is_github_url(url) {
            return url.to_owned();
        }
        match &self.mirror {
            Some(mirror) if mirror.contains("{url}") => mirror.replace("{url}", url),
            Some(mirror) => format!("{}/{url}", mirror.trim_end_matches('/')),
            None => url.to_owned(),
        }
    }
}

/// 单次下载的终端进度状态。
struct DownloadProgress {
    label: String,
    total_bytes: Option<u64>,
    started: Instant,
    last_rendered: Instant,
    terminal: bool,
    previous_width: usize,
    finished: bool,
}

impl DownloadProgress {
    /// 创建进度状态并立即给出开始反馈。
    fn new(label: &str, total_bytes: Option<u64>) -> Self {
        let now = Instant::now();
        let mut progress = Self {
            label: label.to_owned(),
            total_bytes,
            started: now,
            last_rendered: now.checked_sub(LOG_REFRESH).unwrap_or(now),
            terminal: std::io::stderr().is_terminal(),
            previous_width: 0,
            finished: false,
        };
        progress.update(0, false);
        progress
    }

    /// 按终端类型节流刷新；完成时一定输出最终速度。
    fn update(&mut self, downloaded: u64, finished: bool) {
        let now = Instant::now();
        let interval = if self.terminal {
            TERMINAL_REFRESH
        } else {
            LOG_REFRESH
        };
        if !finished && now.duration_since(self.last_rendered) < interval {
            return;
        }
        let line = format_progress(
            &self.label,
            downloaded,
            self.total_bytes,
            now.duration_since(self.started),
        );
        if self.terminal {
            let padding = self.previous_width.saturating_sub(line.chars().count());
            eprint!("\r{line}{}", " ".repeat(padding));
            let _ = std::io::stderr().flush();
            self.previous_width = line.chars().count();
            if finished {
                eprintln!();
            }
        } else {
            eprintln!("{line}");
        }
        self.last_rendered = now;
        self.finished = finished;
    }
}

impl Drop for DownloadProgress {
    fn drop(&mut self) {
        if self.terminal && !self.finished && self.previous_width > 0 {
            eprintln!();
        }
    }
}

/// 生成稳定、可读的下载进度行。
fn format_progress(
    label: &str,
    downloaded: u64,
    total_bytes: Option<u64>,
    elapsed: Duration,
) -> String {
    let elapsed_millis = elapsed.as_millis();
    let speed = u128::from(downloaded)
        .saturating_mul(1_000)
        .checked_div(elapsed_millis)
        .and_then(|value| u64::try_from(value).ok())
        .unwrap_or(0);
    match total_bytes.filter(|total| *total > 0) {
        Some(total) => {
            let percent_tenths = downloaded.saturating_mul(1_000) / total;
            format!(
                "下载{label}  {:>3}.{}%  {} / {}  {}/s",
                percent_tenths / 10,
                percent_tenths % 10,
                format_bytes(downloaded),
                format_bytes(total),
                format_bytes(speed)
            )
        }
        None => format!(
            "下载{label}  {}  {}/s",
            format_bytes(downloaded),
            format_bytes(speed)
        ),
    }
}

/// 使用二进制单位格式化字节量。
fn format_bytes(bytes: u64) -> String {
    const UNITS: [(&str, u64); 5] = [
        ("B", 1),
        ("KiB", 1024),
        ("MiB", 1024 * 1024),
        ("GiB", 1024 * 1024 * 1024),
        ("TiB", 1024 * 1024 * 1024 * 1024),
    ];
    let (unit, divisor) = UNITS
        .iter()
        .rev()
        .find(|(_, divisor)| bytes >= *divisor)
        .copied()
        .unwrap_or(UNITS[0]);
    if divisor == 1 {
        format!("{bytes} {unit}")
    } else {
        let tenths = bytes.saturating_mul(10) / divisor;
        format!("{}.{} {unit}", tenths / 10, tenths % 10)
    }
}

/// 读取尚未创建或正在写入文件的当前大小。
fn file_size(path: &Path) -> anyhow::Result<u64> {
    match fs::metadata(path) {
        Ok(metadata) => Ok(metadata.len()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(error) => Err(error.into()),
    }
}

/// 对外部下载器产生的普通文件计算 SHA-256。
fn hash_file(path: &Path) -> anyhow::Result<String> {
    let metadata = fs::symlink_metadata(path).context("下载程序未生成输出文件")?;
    if !metadata.file_type().is_file() {
        bail!("下载程序输出不是普通文件：{}", path.display());
    }
    let mut input = fs::File::open(path)?;
    let mut digest = Sha256::new();
    std::io::copy(&mut input, &mut digest)?;
    Ok(format!("{:x}", digest.finalize()))
}

/// 发送带稳定 User-Agent 的 GET，并生成可操作的 HTTP 错误。
fn request(agent: &ureq::Agent, url: &str, label: &str) -> anyhow::Result<ureq::Response> {
    agent
        .get(url)
        .set("User-Agent", concat!("procora/", env!("CARGO_PKG_VERSION")))
        .set("Accept", "application/vnd.github+json")
        .call()
        .map_err(|error| anyhow::anyhow!("{label}下载失败：{error}"))
}

/// 在读取正文前拒绝服务器声明的超大响应。
fn enforce_content_length(
    response: &ureq::Response,
    max_bytes: u64,
    label: &str,
) -> anyhow::Result<()> {
    if let Some(length) = response
        .header("Content-Length")
        .and_then(|value| value.parse::<u64>().ok())
        && length > max_bytes
    {
        bail!("{label}声明 {length} 字节，超过允许的 {max_bytes} 字节");
    }
    Ok(())
}

/// 校验镜像是无空白的 HTTPS 地址或 HTTPS 模板。
fn validate_mirror(mirror: &str) -> anyhow::Result<String> {
    if !mirror.starts_with("https://")
        || mirror == "https://"
        || mirror.chars().any(char::is_whitespace)
    {
        bail!("GitHub 镜像必须是有效的 HTTPS 前缀或包含 `{{url}}` 的 HTTPS 模板");
    }
    Ok(mirror.to_owned())
}

/// 判断地址是否属于允许镜像改写的 GitHub HTTP 端点。
fn is_github_url(url: &str) -> bool {
    [
        "https://github.com/",
        "https://api.github.com/",
        "https://raw.githubusercontent.com/",
    ]
    .iter()
    .any(|prefix| url.starts_with(prefix))
}

#[cfg(test)]
#[path = "download_tests.rs"]
mod tests;
