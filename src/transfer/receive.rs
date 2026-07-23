use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    path::Path,
};

use anyhow::{Context, bail};
use fs2::FileExt;
use sha2::{Digest, Sha256};

use crate::config::UploadKind;

use super::{
    archive,
    protocol::{
        TRANSFER_PROTOCOL_VERSION, TransferInit, TransferResponse, TransferResult,
        TransferSelection, TransferTarget,
    },
    target,
};

/// 接收一条 SSH stdin 上传流并在校验后替换声明目标。
pub(crate) fn run() -> anyhow::Result<()> {
    let stdin = std::io::stdin();
    let mut input = BufReader::new(stdin.lock());
    let init: TransferInit = read_json_line(&mut input, "上传请求")?;
    if init.protocol != TRANSFER_PROTOCOL_VERSION {
        bail!(
            "不支持上传协议版本 {}，当前为 {}",
            init.protocol,
            TRANSFER_PROTOCOL_VERSION
        );
    }
    let resolved = negotiate_target(&mut input, &init)?;
    let runtime = resolved.root.join(".procora");
    if fs::symlink_metadata(&runtime).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        bail!(
            "Service 运行目录 `{}` 是符号链接，拒绝接收上传",
            runtime.display()
        );
    }
    fs::create_dir_all(runtime.join("transfers/incoming"))?;
    fs::create_dir_all(runtime.join("transfers/locks"))?;
    let _lock = acquire_lock(&runtime, &resolved.selector)?;
    let archive_path = runtime
        .join("transfers/incoming")
        .join(format!("{}.tar.gz", uuid::Uuid::new_v4()));
    send_response(&TransferResponse::Ready {
        target: resolved.selector.clone(),
    })?;
    let result = receive_and_commit(&mut input, &init, &resolved, &archive_path);
    let _ = fs::remove_file(&archive_path);
    let result = result?;
    send_response(&TransferResponse::Complete { result })?;
    Ok(())
}

/// 协商显式或自动发现的目标，并完成类型与大小预检。
fn negotiate_target(
    input: &mut impl BufRead,
    init: &TransferInit,
) -> anyhow::Result<target::ResolvedTarget> {
    let selector = if let Some(selector) = &init.target {
        selector.clone()
    } else {
        let compatible = target::list()?
            .into_iter()
            .filter(|candidate| {
                candidate.kind == init.source_kind && candidate.max_bytes >= init.content_bytes
            })
            .map(|candidate| TransferTarget {
                selector: candidate.selector,
                kind: candidate.kind,
                max_bytes: candidate.max_bytes,
            })
            .collect::<Vec<_>>();
        match compatible.as_slice() {
            [] => bail!(
                "远端没有可接收 {:?} 且上限不少于 {} 字节的活动上传目标",
                init.source_kind,
                init.content_bytes
            ),
            [only] => only.selector.clone(),
            _ => {
                send_response(&TransferResponse::Choose {
                    targets: compatible.clone(),
                })?;
                let selection: TransferSelection = read_json_line(input, "上传目标选择")?;
                if !compatible
                    .iter()
                    .any(|candidate| candidate.selector == selection.target)
                {
                    bail!("选择的上传目标不在本次远端候选中");
                }
                selection.target
            }
        }
    };
    let resolved = target::resolve(&selector)?;
    validate_target(init, &resolved)?;
    Ok(resolved)
}

/// 校验来源类型以及压缩前后的传输上限。
fn validate_target(init: &TransferInit, resolved: &target::ResolvedTarget) -> anyhow::Result<()> {
    if resolved.kind != init.source_kind {
        bail!("上传来源类型与目标声明不一致：目标要求 {:?}", resolved.kind);
    }
    if init.content_bytes > resolved.max_bytes {
        bail!(
            "上传内容 {} 字节，超过目标上限 {} 字节",
            init.content_bytes,
            resolved.max_bytes
        );
    }
    let max_archive_bytes = resolved.max_bytes.saturating_add(64 * 1024 * 1024);
    if init.archive_bytes > max_archive_bytes {
        bail!("上传归档超过目标允许的传输上限 {max_archive_bytes} 字节");
    }
    Ok(())
}

/// 限制协商消息长度并解析一行 JSON。
fn read_json_line<T: serde::de::DeserializeOwned>(
    input: &mut impl BufRead,
    label: &str,
) -> anyhow::Result<T> {
    let mut bytes = Vec::new();
    input.take(64 * 1024).read_until(b'\n', &mut bytes)?;
    if bytes.is_empty() || !bytes.ends_with(b"\n") {
        bail!("{label}缺少完整协议行");
    }
    serde_json::from_slice(&bytes).with_context(|| format!("{label}不是有效 JSON"))
}

/// 发送并立即刷新一条远端协议消息。
fn send_response(response: &TransferResponse) -> anyhow::Result<()> {
    let stdout = std::io::stdout();
    let mut output = stdout.lock();
    serde_json::to_writer(&mut output, response)?;
    output.write_all(b"\n")?;
    output.flush()?;
    Ok(())
}

/// 完整落盘、校验压缩归档并提交目标。
fn receive_and_commit(
    input: &mut impl Read,
    init: &TransferInit,
    resolved: &target::ResolvedTarget,
    archive_path: &Path,
) -> anyhow::Result<TransferResult> {
    let actual = write_archive(input, archive_path, init.archive_bytes)?;
    if !actual.eq_ignore_ascii_case(&init.sha256) {
        bail!(
            "上传归档 SHA-256 不匹配：期望 {}，实际 {actual}",
            init.sha256
        );
    }
    let parent = resolved.path.parent().context("上传目标没有父目录")?;
    let stage = parent.join(format!(".procora-upload-{}", uuid::Uuid::new_v4()));
    let unpacked = archive::unpack(archive_path, &stage, resolved.kind, resolved.max_bytes);
    if unpacked.as_ref().is_err_and(|_| stage.exists()) {
        let _ = fs::remove_dir_all(&stage);
    }
    let unpacked = unpacked?;
    if unpacked != init.content_bytes {
        let _ = fs::remove_dir_all(&stage);
        bail!(
            "上传内容大小不匹配：期望 {}，实际 {unpacked}",
            init.content_bytes
        );
    }
    let staged_target = if resolved.kind == UploadKind::File {
        stage.join("payload")
    } else {
        stage.clone()
    };
    if let Err(error) = activate(&staged_target, &resolved.path) {
        let _ = fs::remove_dir_all(&stage);
        return Err(error);
    }
    if resolved.kind == UploadKind::File {
        let _ = fs::remove_dir_all(&stage);
    }
    Ok(TransferResult {
        target: resolved.selector.clone(),
        path: resolved.path.display().to_string(),
        content_bytes: unpacked,
        sha256: init.sha256.clone(),
    })
}

/// 有界接收精确归档字节并同步计算摘要。
fn write_archive(input: &mut impl Read, path: &Path, archive_bytes: u64) -> anyhow::Result<String> {
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    let mut limited = input.take(archive_bytes);
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
    if written != archive_bytes {
        bail!("上传流提前结束：期望 {archive_bytes} 字节，实际 {written} 字节");
    }
    output.flush()?;
    output.sync_all()?;
    Ok(format!("{:x}", digest.finalize()))
}

/// 用同目录备份和重命名替换目标，失败时恢复旧内容。
fn activate(staging: &Path, destination: &Path) -> anyhow::Result<()> {
    let backup = destination.with_file_name(format!(".procora-backup-{}", uuid::Uuid::new_v4()));
    let had_previous = destination.exists();
    if had_previous {
        fs::rename(destination, &backup)?;
    }
    if let Err(error) = fs::rename(staging, destination) {
        if had_previous {
            let _ = fs::rename(&backup, destination);
        }
        return Err(error.into());
    }
    if had_previous {
        if backup.is_dir() {
            let _ = fs::remove_dir_all(backup);
        } else {
            let _ = fs::remove_file(backup);
        }
    }
    Ok(())
}

/// 按目标选择器获取跨进程排他锁。
fn acquire_lock(runtime: &Path, selector: &str) -> anyhow::Result<fs::File> {
    let name = format!("{:x}", Sha256::digest(selector.as_bytes()));
    let file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(runtime.join("transfers/locks").join(name))?;
    file.try_lock_exclusive()
        .with_context(|| format!("上传目标 `{selector}` 正由另一个客户端更新"))?;
    Ok(file)
}
