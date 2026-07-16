use std::{path::Path, process::Command};

#[cfg(unix)]
use std::fs;

use crate::config::{DependencyKind, DependencyVerifySpec};

use super::manager::SourceError;

/// 为二进制依赖补充当前用户可执行权限。
pub(crate) fn ensure_kind(path: &Path, kind: DependencyKind) -> Result<(), SourceError> {
    match kind {
        DependencyKind::Directory if !path.is_dir() => {
            return Err(SourceError::ManagedPath(format!(
                "`{}` 不是目录",
                path.display()
            )));
        }
        DependencyKind::Binary | DependencyKind::File if !path.is_file() => {
            return Err(SourceError::ManagedPath(format!(
                "`{}` 不是文件",
                path.display()
            )));
        }
        _ => {}
    }
    #[cfg(unix)]
    if kind == DependencyKind::Binary {
        use std::os::unix::fs::PermissionsExt;

        let mut permissions = fs::metadata(path)?.permissions();
        permissions.set_mode(permissions.mode() | 0o700);
        fs::set_permissions(path, permissions)?;
    }
    Ok(())
}

/// 执行声明的版本命令并校验其标准输出或标准错误。
pub(crate) fn run_version_check(
    install_root: &Path,
    managed_path: &Path,
    version: &str,
    verify: Option<&DependencyVerifySpec>,
) -> Result<(), SourceError> {
    let Some(verify) = verify else {
        return Ok(());
    };
    let command = verify.command.as_ref().map_or_else(
        || managed_path.to_path_buf(),
        |path| install_root.join(path),
    );
    let output = Command::new(&command)
        .args(&verify.args)
        .output()
        .map_err(|error| SourceError::Verify {
            command: command.clone(),
            message: error.to_string(),
        })?;
    if !output.status.success() {
        return Err(SourceError::Verify {
            command,
            message: format!("退出状态 {}", output.status),
        });
    }
    let expected = verify.contains.as_deref().unwrap_or(version);
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    if !text.contains(expected) {
        return Err(SourceError::Verify {
            command,
            message: format!("输出不包含 `{expected}`"),
        });
    }
    Ok(())
}
