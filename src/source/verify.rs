use std::{path::Path, process::Command};

#[cfg(target_os = "linux")]
use std::{io, thread, time::Duration};

#[cfg(unix)]
use std::fs;

use crate::config::{DependencyKind, DependencyVerifySpec};

use super::manager::SourceError;

#[cfg(target_os = "linux")]
const EXECUTABLE_BUSY_RETRIES: u8 = 3;

#[cfg(target_os = "linux")]
const EXECUTABLE_BUSY_RETRY_DELAY: Duration = Duration::from_millis(10);

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
    let output = run_command(&command, &verify.args).map_err(|error| SourceError::Verify {
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

/// 执行版本命令，并在 Linux 临时占用刚写入的可执行文件时进行有限重试。
fn run_command(command: &Path, args: &[String]) -> Result<std::process::Output, std::io::Error> {
    #[cfg(target_os = "linux")]
    for attempt in 0..EXECUTABLE_BUSY_RETRIES {
        match Command::new(command).args(args).output() {
            Err(error) if error.kind() == io::ErrorKind::ExecutableFileBusy => {
                thread::sleep(EXECUTABLE_BUSY_RETRY_DELAY * u32::from(attempt + 1));
            }
            result => return result,
        }
    }
    Command::new(command).args(args).output()
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use std::{
        fs::{self, OpenOptions},
        os::unix::fs::PermissionsExt,
        thread,
        time::{Duration, SystemTime, UNIX_EPOCH},
    };

    use crate::config::DependencyVerifySpec;

    use super::run_version_check;

    #[test]
    // Linux 上刚写入的可执行文件被短暂占用时会等待后再验证。
    fn version_check_retries_temporarily_busy_executable() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "procora-verify-busy-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        let command = root.join("tool");
        fs::write(&command, "#!/bin/sh\necho tool 1.0\n").unwrap();
        fs::set_permissions(&command, fs::Permissions::from_mode(0o700)).unwrap();

        let writer = OpenOptions::new().write(true).open(&command).unwrap();
        let release = thread::spawn(move || {
            thread::sleep(Duration::from_millis(25));
            drop(writer);
        });
        let verify = DependencyVerifySpec {
            command: None,
            args: Vec::new(),
            contains: Some("1.0".to_owned()),
        };

        run_version_check(&root, &command, "1.0", Some(&verify)).unwrap();
        release.join().unwrap();
        fs::remove_dir_all(root).unwrap();
    }
}
