use std::{
    fs,
    io::{self, Read},
    path::{Component, Path, PathBuf},
};

use crate::config::UnpackMode;
use flate2::read::GzDecoder;

use super::manager::SourceError;

/// 自动识别归档并返回承载解包结果的目录。
pub(crate) fn materialize(
    downloaded: &Path,
    filename: &str,
    output: &Path,
    mode: UnpackMode,
) -> Result<PathBuf, SourceError> {
    fs::create_dir_all(output)?;
    if mode == UnpackMode::Never {
        let target = output.join(filename);
        fs::copy(downloaded, &target)?;
        return Ok(output.to_path_buf());
    }
    let mut header = [0_u8; 512];
    let read = fs::File::open(downloaded)?.read(&mut header)?;
    if header.starts_with(b"PK\x03\x04") {
        extract_zip(downloaded, output)?;
    } else if header.starts_with(&[0x1f, 0x8b]) {
        extract_gzip(downloaded, filename, output)?;
    } else if extension_is(filename, "tar") || (read > 262 && &header[257..262] == b"ustar") {
        extract_tar(fs::File::open(downloaded)?, output)?;
    } else {
        fs::copy(downloaded, output.join(filename))?;
    }
    Ok(output.to_path_buf())
}

/// 安全解压 ZIP，拒绝父目录和绝对路径条目。
fn extract_zip(archive: &Path, output: &Path) -> Result<(), SourceError> {
    let mut zip = zip::ZipArchive::new(fs::File::open(archive)?)?;
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index)?;
        let Some(relative) = entry.enclosed_name() else {
            return Err(SourceError::UnsafeArchive(entry.name().to_owned()));
        };
        let target = output.join(relative);
        if entry.is_dir() {
            fs::create_dir_all(&target)?;
            continue;
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        io::copy(&mut entry, &mut fs::File::create(&target)?)?;
    }
    Ok(())
}

/// 解压 gzip，并在内容为 tar 时继续安全展开。
fn extract_gzip(archive: &Path, filename: &str, output: &Path) -> Result<(), SourceError> {
    let decoded = GzDecoder::new(fs::File::open(archive)?);
    if extension_is(filename, "tgz") || is_tar_gzip(filename) {
        extract_tar(decoded, output)
    } else {
        let name = filename.strip_suffix(".gz").unwrap_or(filename);
        io::copy(
            &mut decoded.take(u64::MAX),
            &mut fs::File::create(output.join(name))?,
        )?;
        Ok(())
    }
}

/// 按 ASCII 大小写不敏感方式判断文件扩展名。
fn extension_is(path: &str, extension: &str) -> bool {
    Path::new(path)
        .extension()
        .is_some_and(|value| value.eq_ignore_ascii_case(extension))
}

/// 判断文件名是否具有 `.tar.gz` 双扩展名。
fn is_tar_gzip(path: &str) -> bool {
    extension_is(path, "gz")
        && Path::new(path)
            .file_stem()
            .and_then(|stem| Path::new(stem).extension())
            .is_some_and(|value| value.eq_ignore_ascii_case("tar"))
}

/// 只展开 tar 中的普通文件与目录并阻止路径逃逸。
fn extract_tar(reader: impl Read, output: &Path) -> Result<(), SourceError> {
    let mut archive = tar::Archive::new(reader);
    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        if !safe_relative(&path) {
            return Err(SourceError::UnsafeArchive(path.display().to_string()));
        }
        let kind = entry.header().entry_type();
        if kind.is_dir() {
            fs::create_dir_all(output.join(path))?;
        } else if kind.is_file() {
            let target = output.join(path);
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            entry.unpack(target)?;
        }
    }
    Ok(())
}

/// 判断归档内路径是否保持在安装目录内。
pub(crate) fn safe_relative(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}
