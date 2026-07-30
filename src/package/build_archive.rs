//! Procora 包的确定性容器写入。

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

/// 写入清单优先、条目稳定排序的 zstd tar。
pub(super) fn write_package(
    output: &Path,
    manifest: &[u8],
    blobs: &BTreeMap<String, PathBuf>,
) -> anyhow::Result<()> {
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    let file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)?;
    let result = (|| {
        let encoder = zstd::Encoder::new(file, 3)?;
        let mut archive = tar::Builder::new(encoder);
        append_bytes(&mut archive, "manifest.json", manifest, 0o644)?;
        for (blob, source) in blobs {
            let digest = blob.strip_prefix("sha256:").expect("构建器只生成 SHA-256");
            let archive_path = format!("blobs/sha256/{}/{}", &digest[..2], &digest[2..]);
            append_file(&mut archive, &archive_path, source)?;
        }
        let encoder = archive.into_inner()?;
        let file = encoder.finish()?;
        file.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(output);
    }
    result
}

/// 写入一个规范化普通文件条目。
fn append_file(
    archive: &mut tar::Builder<zstd::Encoder<'static, fs::File>>,
    path: &str,
    source: &Path,
) -> anyhow::Result<()> {
    let metadata = fs::metadata(source)?;
    let mut header = deterministic_header(metadata.len(), 0o644);
    let mut file = fs::File::open(source)?;
    archive.append_data(&mut header, path, &mut file)?;
    Ok(())
}

/// 写入一个内存中的规范化普通文件条目。
fn append_bytes(
    archive: &mut tar::Builder<zstd::Encoder<'static, fs::File>>,
    path: &str,
    bytes: &[u8],
    mode: u32,
) -> anyhow::Result<()> {
    let mut header = deterministic_header(bytes.len() as u64, mode);
    archive.append_data(&mut header, path, bytes)?;
    Ok(())
}

/// 创建不携带宿主身份和时间的 tar 头。
fn deterministic_header(size: u64, mode: u32) -> tar::Header {
    let mut header = tar::Header::new_gnu();
    header.set_size(size);
    header.set_mode(mode);
    header.set_uid(0);
    header.set_gid(0);
    header.set_mtime(0);
    header.set_cksum();
    header
}
