//! GitHub Releases 自更新下载、校验与安装流程。

mod archive;
mod replace;

use std::{
    env, fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, bail};
use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};

const DEFAULT_REPOSITORY: &str = "laull9/procora";
const API_MAX_BYTES: u64 = 2 * 1024 * 1024;
const CHECKSUM_MAX_BYTES: u64 = 16 * 1024;
const ARCHIVE_MAX_BYTES: u64 = 128 * 1024 * 1024;

/// GitHub Release API 中本次更新所需的字段。
#[derive(Deserialize)]
struct Release {
    tag_name: String,
    assets: Vec<ReleaseAsset>,
}

/// GitHub Release 中单个下载资产。
#[derive(Deserialize)]
struct ReleaseAsset {
    name: String,
    browser_download_url: String,
}

/// 自动清理本次更新的独占暂存目录。
struct TemporaryDirectory(PathBuf);

impl TemporaryDirectory {
    /// 创建权限受当前用户控制的更新暂存目录。
    fn create() -> anyhow::Result<Self> {
        let path = env::temp_dir().join(format!("procora-update-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&path)?;
        Ok(Self(path))
    }
}

impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// 查询最新发布，并在需要时下载、校验和安装。
pub(crate) fn run(check: bool) -> anyhow::Result<()> {
    let repository = env::var("PROCORA_REPO").unwrap_or_else(|_| DEFAULT_REPOSITORY.to_owned());
    validate_repository(&repository)?;
    let api_url = env::var("PROCORA_UPDATE_API_URL")
        .unwrap_or_else(|_| format!("https://api.github.com/repos/{repository}/releases/latest"));
    let agent = http_agent();
    let release: Release = serde_json::from_slice(&download_bytes(
        &agent,
        &api_url,
        API_MAX_BYTES,
        "发布信息",
    )?)
    .context("GitHub 返回了无效的发布信息")?;
    let latest = parse_tag(&release.tag_name)?;
    let current = Version::parse(env!("CARGO_PKG_VERSION")).context("当前包版本格式无效")?;
    if latest <= current {
        println!("Procora 已是最新版本：v{current}");
        return Ok(());
    }
    println!("发现新版本：v{current} → v{latest}");
    if check {
        return Ok(());
    }

    let (target, extension, executable_name) = current_asset()?;
    let asset_name = format!("procora-{target}.{extension}");
    let checksum_name = format!("{asset_name}.sha256");
    let archive_url = asset_url(&release, &asset_name)?;
    let checksum_url = asset_url(&release, &checksum_name)?;
    let temporary = TemporaryDirectory::create()?;
    let archive_path = temporary.0.join(&asset_name);
    let executable_path = temporary.0.join(executable_name);
    let expected = parse_checksum(&download_bytes(
        &agent,
        checksum_url,
        CHECKSUM_MAX_BYTES,
        "校验文件",
    )?)?;
    let actual = download_file(
        &agent,
        archive_url,
        &archive_path,
        ARCHIVE_MAX_BYTES,
        "发布归档",
    )?;
    if actual != expected {
        bail!("更新归档 SHA-256 校验失败：期望 {expected}，实际 {actual}");
    }
    archive::extract(&archive_path, &executable_path)?;
    let destination = env::current_exe()
        .context("无法定位当前 Procora 可执行文件")?
        .canonicalize()
        .context("无法解析当前 Procora 可执行文件路径")?;
    let restart_center = crate::cli::center_is_running_for_update()?;
    replace::install(&executable_path, &destination, restart_center)?;
    #[cfg(target_os = "windows")]
    println!("更新已下载并校验，将在当前进程退出后安装 v{latest}");
    #[cfg(not(target_os = "windows"))]
    println!("Procora 已更新到 v{latest}：{}", destination.display());
    Ok(())
}

/// Windows 内部更新助手入口。
#[cfg(target_os = "windows")]
pub(crate) fn apply_windows(
    source: &Path,
    destination: &Path,
    restart_center: bool,
) -> anyhow::Result<()> {
    replace::apply_windows(source, destination, restart_center)
}

/// Windows 内部暂存清理器入口。
#[cfg(target_os = "windows")]
pub(crate) fn cleanup_windows(path: &Path) -> anyhow::Result<()> {
    replace::cleanup_windows(path)
}

/// 创建带总超时、连接超时和有限重定向的更新 HTTP 客户端。
fn http_agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout(Duration::from_mins(2))
        .timeout_connect(Duration::from_secs(15))
        .redirects(5)
        .build()
}

/// 下载有界的小型响应到内存。
fn download_bytes(
    agent: &ureq::Agent,
    url: &str,
    max_bytes: u64,
    label: &str,
) -> anyhow::Result<Vec<u8>> {
    let response = request(agent, url, label)?;
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

/// 流式下载归档并同步计算小写 SHA-256。
fn download_file(
    agent: &ureq::Agent,
    url: &str,
    destination: &Path,
    max_bytes: u64,
    label: &str,
) -> anyhow::Result<String> {
    let response = request(agent, url, label)?;
    enforce_content_length(&response, max_bytes, label)?;
    let mut input = response.into_reader().take(max_bytes + 1);
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)?;
    let mut digest = Sha256::new();
    let mut buffer = vec![0_u8; 64 * 1024];
    let mut total = 0_u64;
    loop {
        let count = input.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        total = total.saturating_add(count as u64);
        if total > max_bytes {
            bail!("{label}超过允许的 {max_bytes} 字节");
        }
        output.write_all(&buffer[..count])?;
        digest.update(&buffer[..count]);
    }
    output.sync_all()?;
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

/// 从严格的 `vX.Y.Z` 标签读取语义版本。
fn parse_tag(tag: &str) -> anyhow::Result<Version> {
    let value = tag
        .strip_prefix('v')
        .context("最新 Release 标签必须以 `v` 开头")?;
    Version::parse(value).with_context(|| format!("最新 Release 标签版本无效：{tag}"))
}

/// 定位发布中名称完全匹配的资产。
fn asset_url<'a>(release: &'a Release, name: &str) -> anyhow::Result<&'a str> {
    release
        .assets
        .iter()
        .find(|asset| asset.name == name)
        .map(|asset| asset.browser_download_url.as_str())
        .with_context(|| format!("最新 Release 缺少当前平台资产 `{name}`"))
}

/// 解析 sha256sum 兼容格式的摘要首字段。
fn parse_checksum(bytes: &[u8]) -> anyhow::Result<String> {
    let content = std::str::from_utf8(bytes).context("SHA-256 校验文件不是 UTF-8")?;
    let value = content
        .split_ascii_whitespace()
        .next()
        .context("SHA-256 校验文件为空")?
        .to_ascii_lowercase();
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("SHA-256 校验文件格式无效");
    }
    Ok(value)
}

/// 校验与安装脚本一致的 GitHub `owner/repo` 名称。
fn validate_repository(repository: &str) -> anyhow::Result<()> {
    let mut parts = repository.split('/');
    let owner = parts.next().unwrap_or_default();
    let name = parts.next().unwrap_or_default();
    if owner.is_empty()
        || name.is_empty()
        || parts.next().is_some()
        || !owner
            .bytes()
            .chain(name.bytes())
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        bail!("PROCORA_REPO 必须使用有效的 owner/repo 格式");
    }
    Ok(())
}

/// 返回当前编译平台对应的发布 target、归档扩展名和可执行文件名。
fn current_asset() -> anyhow::Result<(&'static str, &'static str, &'static str)> {
    match (env::consts::OS, env::consts::ARCH) {
        ("linux", "x86_64") => Ok(("x86_64-unknown-linux-musl", "tar.gz", "procora")),
        ("linux", "aarch64") => Ok(("aarch64-unknown-linux-musl", "tar.gz", "procora")),
        ("macos", "x86_64") => Ok(("x86_64-apple-darwin", "tar.gz", "procora")),
        ("macos", "aarch64") => Ok(("aarch64-apple-darwin", "tar.gz", "procora")),
        ("windows", "x86_64") => Ok(("x86_64-pc-windows-msvc", "zip", "procora.exe")),
        ("windows", "aarch64") => Ok(("aarch64-pc-windows-msvc", "zip", "procora.exe")),
        (system, architecture) => {
            bail!("当前平台没有更新产物：{architecture}-{system}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_checksum, parse_tag, validate_repository};

    #[test]
    // Release标签必须使用v前缀并遵循语义版本。
    fn release_tag_requires_semantic_version() {
        assert_eq!(parse_tag("v0.6.0").unwrap().to_string(), "0.6.0");
        assert!(parse_tag("0.6.0").is_err());
        assert!(parse_tag("vnext").is_err());
    }

    #[test]
    // 校验文件只接受完整十六进制SHA-256首字段。
    fn checksum_requires_complete_sha256() {
        let hash = "a".repeat(64);
        assert_eq!(
            parse_checksum(format!("{hash}  procora.tar.gz\n").as_bytes()).unwrap(),
            hash
        );
        assert!(parse_checksum(b"abcd file").is_err());
    }

    #[test]
    // 仓库覆盖值不能注入额外URL路径或shell字符。
    fn repository_requires_owner_and_name() {
        assert!(validate_repository("laull9/procora").is_ok());
        assert!(validate_repository("owner/repo/extra").is_err());
        assert!(validate_repository("owner/repo?x").is_err());
    }
}
