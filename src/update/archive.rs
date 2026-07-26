use std::{
    fs,
    io::{self},
    path::Path,
};

use anyhow::{Context, bail};

/// 从发布归档中提取唯一的当前平台 Procora 可执行文件。
pub(super) fn extract(archive: &Path, destination: &Path) -> anyhow::Result<()> {
    #[cfg(target_os = "windows")]
    {
        extract_zip(archive, destination)
    }
    #[cfg(not(target_os = "windows"))]
    {
        extract_tar_gz(archive, destination)
    }
}

/// 安全提取只含 `procora` 普通文件的 tar.gz 发布归档。
#[cfg(not(target_os = "windows"))]
fn extract_tar_gz(archive: &Path, destination: &Path) -> anyhow::Result<()> {
    let file = fs::File::open(archive)?;
    let decoder = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);
    let mut entries = archive.entries().context("无法读取更新归档")?;
    let mut entry = entries.next().transpose()?.context("更新归档为空")?;
    if entry.path()?.as_ref() != Path::new("procora") || !entry.header().entry_type().is_file() {
        bail!("更新归档必须只包含普通文件 `procora`");
    }
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)?;
    io::copy(&mut entry, &mut output)?;
    output.sync_all()?;
    drop(entry);
    if entries.next().transpose()?.is_some() {
        bail!("更新归档包含意外的额外条目");
    }
    Ok(())
}

/// 安全提取只含 `procora.exe` 普通文件的 zip 发布归档。
#[cfg(target_os = "windows")]
fn extract_zip(archive: &Path, destination: &Path) -> anyhow::Result<()> {
    let file = fs::File::open(archive)?;
    let mut archive = zip::ZipArchive::new(file).context("无法读取更新归档")?;
    if archive.len() != 1 {
        bail!("更新归档必须只包含 `procora.exe`");
    }
    let mut entry = archive.by_index(0)?;
    if !entry.is_file() || entry.name() != "procora.exe" {
        bail!("更新归档必须只包含普通文件 `procora.exe`");
    }
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)?;
    io::copy(&mut entry, &mut output)?;
    output.sync_all()?;
    Ok(())
}

/// 把已验证的可执行文件复制到目标并保留可执行权限。
pub(super) fn prepare_executable(source: &Path, destination: &Path) -> anyhow::Result<()> {
    let mut input = fs::File::open(source)?;
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o755);
    }
    let mut output = options.open(destination)?;
    io::copy(&mut input, &mut output)?;
    output.sync_all()?;
    Ok(())
}

#[cfg(all(test, not(target_os = "windows")))]
mod tests {
    use std::{fs, io::Write};

    use super::extract_tar_gz;

    /// 创建当前测试独占目录。
    fn temporary_directory() -> std::path::PathBuf {
        let path =
            std::env::temp_dir().join(format!("procora-update-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&path).unwrap();
        path
    }

    /// 创建含指定普通条目的gzip tar归档。
    fn archive(path: &std::path::Path, entries: &[(&str, &[u8])]) {
        let file = fs::File::create(path).unwrap();
        let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
        let mut builder = tar::Builder::new(encoder);
        for (name, bytes) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_size(bytes.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            builder.append_data(&mut header, name, *bytes).unwrap();
        }
        builder
            .into_inner()
            .unwrap()
            .finish()
            .unwrap()
            .flush()
            .unwrap();
    }

    #[test]
    // 仅含procora普通文件的发布归档可安全提取。
    fn release_archive_extracts_single_executable() {
        let directory = temporary_directory();
        let source = directory.join("release.tar.gz");
        let destination = directory.join("procora");
        archive(&source, &[("procora", b"binary")]);

        extract_tar_gz(&source, &destination).unwrap();

        assert_eq!(fs::read(&destination).unwrap(), b"binary");
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    // 发布归档含额外文件时拒绝安装。
    fn release_archive_rejects_extra_entries() {
        let directory = temporary_directory();
        let source = directory.join("release.tar.gz");
        let destination = directory.join("procora");
        archive(&source, &[("procora", b"binary"), ("extra", b"unexpected")]);

        assert!(extract_tar_gz(&source, &destination).is_err());

        fs::remove_dir_all(directory).unwrap();
    }
}
