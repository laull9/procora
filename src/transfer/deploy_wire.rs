//! 全托管部署接收端的有界行协议、归档写入与请求边界。

use std::{
    fs,
    io::{BufRead, Read, Write},
    path::{Component, Path},
};

use anyhow::{Context, bail};
use fs2::FileExt;
use sha2::{Digest, Sha256};

use crate::core::ServiceName;

use super::deploy_protocol::{DEPLOY_PROTOCOL_VERSION, DeployInit, DeployPhase, DeployResponse};

/// 单次部署允许接收和展开的最大字节数。
pub(super) const MAX_DEPLOY_BYTES: u64 = 8 * 1024 * 1024 * 1024;
const MAX_TIMEOUT_MS: u64 = 10 * 60 * 1_000;

/// 校验部署请求的全部无副作用边界。
pub(super) fn validate_init(init: &DeployInit) -> anyhow::Result<()> {
    if init.protocol != DEPLOY_PROTOCOL_VERSION {
        bail!(
            "不支持部署协议版本 {}，远端支持 {}",
            init.protocol,
            DEPLOY_PROTOCOL_VERSION
        );
    }
    let _: ServiceName = init.project.parse()?;
    let config_path = Path::new(&init.config_path);
    if !portable_config_path(&init.config_path) || !safe_relative(config_path) {
        bail!("部署配置入口必须是 Service 内不含 `.procora` 的普通相对路径");
    }
    if init.archive_bytes == 0 || init.archive_bytes > MAX_DEPLOY_BYTES {
        bail!("部署归档大小必须在 1..={MAX_DEPLOY_BYTES} 字节内");
    }
    if init.content_bytes > MAX_DEPLOY_BYTES {
        bail!("部署内容超过 {MAX_DEPLOY_BYTES} 字节上限");
    }
    if init.sha256.len() != 64 || !init.sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("部署归档 SHA-256 格式无效");
    }
    if init.timeout_ms == 0 || init.timeout_ms > MAX_TIMEOUT_MS {
        bail!("部署验收超时必须在 1ms..=10m 内");
    }
    if init.stable_for_ms > init.timeout_ms {
        bail!("部署稳定窗口不能超过验收超时");
    }
    if !(1..=32).contains(&init.keep) {
        bail!("release 保留数量必须在 1..=32 内");
    }
    Ok(())
}

/// 接收精确归档字节并同步计算摘要。
pub(super) fn write_archive(
    input: &mut impl Read,
    path: &Path,
    expected: u64,
) -> anyhow::Result<String> {
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    let mut limited = input.take(expected);
    let mut digest = Sha256::new();
    let mut written = 0_u64;
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let read = limited.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        output.write_all(&buffer[..read])?;
        digest.update(&buffer[..read]);
        written = written.saturating_add(read as u64);
    }
    if written != expected {
        bail!("部署流提前结束：期望 {expected} 字节，实际 {written} 字节");
    }
    output.flush()?;
    output.sync_all()?;
    Ok(format!("{:x}", digest.finalize()))
}

/// 读取一条有界 JSON 行。
pub(super) fn read_json_line<T: serde::de::DeserializeOwned>(
    input: &mut impl BufRead,
    label: &str,
) -> anyhow::Result<T> {
    let mut bytes = Vec::new();
    input.take(64 * 1024).read_until(b'\n', &mut bytes)?;
    if bytes.is_empty() || !bytes.ends_with(b"\n") {
        bail!("{label}不是完整 JSON 行");
    }
    serde_json::from_slice(&bytes).with_context(|| format!("{label}不是有效 JSON"))
}

/// 写入并刷新一条部署响应。
pub(super) fn send_response(response: &DeployResponse) -> anyhow::Result<()> {
    let stdout = std::io::stdout();
    let mut output = stdout.lock();
    serde_json::to_writer(&mut output, response)?;
    output.write_all(b"\n")?;
    output.flush()?;
    Ok(())
}

/// 尽力发送阶段事件；客户端断开不应改变远端部署事务的结果。
pub(super) fn send_progress(phase: DeployPhase, message: impl Into<String>) {
    let _ = send_response(&DeployResponse::Progress {
        phase,
        message: message.into(),
    });
}

/// 为单个托管 Service 获取跨进程排他锁。
pub(super) fn acquire_lock(service_root: &Path) -> anyhow::Result<fs::File> {
    fs::create_dir_all(service_root)?;
    let path = service_root.join("deploy.lock");
    let file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&path)?;
    file.try_lock_exclusive()
        .with_context(|| format!("Service 正由另一个部署更新：{}", service_root.display()))?;
    Ok(file)
}

/// 校验协议路径在 Windows 与 Unix 上具有相同片段语义。
fn portable_config_path(value: &str) -> bool {
    !value.is_empty()
        && value.split('/').all(|segment| {
            !segment.is_empty()
                && !segment.ends_with(['.', ' '])
                && !segment
                    .chars()
                    .any(|character| character.is_control() || r#"\/<>:"|?*"#.contains(character))
        })
}

/// 只接受不越界且不进入内部目录的配置相对路径。
fn safe_relative(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && path.components().all(|component| match component {
            Component::Normal(value) => value != ".procora",
            _ => false,
        })
}
