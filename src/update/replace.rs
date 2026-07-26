use std::{fs, path::Path};

use anyhow::Context;

/// 在 Unix 上把新文件写入同目录后原子替换当前可执行文件。
#[cfg(not(target_os = "windows"))]
pub(super) fn install(
    source: &Path,
    destination: &Path,
    restart_center: bool,
) -> anyhow::Result<()> {
    use std::process::Command;

    let parent = destination.parent().context("当前可执行文件没有父目录")?;
    let staged = parent.join(format!(".procora-update-{}", uuid::Uuid::new_v4()));
    let result = (|| -> anyhow::Result<()> {
        super::archive::prepare_executable(source, &staged)?;
        fs::rename(&staged, destination).with_context(|| {
            format!(
                "无法覆盖 {}；请确认当前用户拥有安装目录写权限",
                destination.display()
            )
        })?;
        if restart_center {
            let status = Command::new(destination)
                .arg("__reconcile-update")
                .status()
                .context("Procora 已更新，但无法启动新版本对账全局 Center")?;
            if !status.success() {
                anyhow::bail!("Procora 已更新，但新版本无法对账全局 Center：{status}");
            }
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(staged);
    }
    result
}

/// 在 Windows 上启动新版本更新助手，等待当前进程退出后再覆盖。
#[cfg(target_os = "windows")]
pub(super) fn install(
    source: &Path,
    destination: &Path,
    restart_center: bool,
) -> anyhow::Result<()> {
    use std::{os::windows::process::CommandExt, process::Command};

    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    Command::new(source)
        .arg("__apply-update")
        .arg("--source")
        .arg(source)
        .arg("--destination")
        .arg(destination)
        .args(restart_center.then_some("--restart-center"))
        .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP)
        .spawn()
        .context("无法启动 Windows 更新助手")?;
    Ok(())
}

/// Windows 更新助手等待旧进程解锁并完成可恢复替换。
#[cfg(target_os = "windows")]
pub(crate) fn apply_windows(
    source: &Path,
    destination: &Path,
    restart_center: bool,
) -> anyhow::Result<()> {
    use std::{
        process::{Command, Stdio},
        thread,
        time::Duration,
    };

    let parent = destination.parent().context("当前可执行文件没有父目录")?;
    let staged = parent.join(format!(".procora-update-{}.exe", uuid::Uuid::new_v4()));
    let backup = parent.join(format!(".procora-old-{}.exe", uuid::Uuid::new_v4()));
    super::archive::prepare_executable(source, &staged)?;
    let mut last_error = None;
    for _ in 0..300 {
        match fs::rename(destination, &backup) {
            Ok(()) => {
                if let Err(error) = fs::rename(&staged, destination) {
                    let _ = fs::rename(&backup, destination);
                    return Err(error).context("Windows 更新文件替换失败，已尝试恢复旧版本");
                }
                let _ = fs::remove_file(&backup);
                if restart_center {
                    let status = Command::new(destination)
                        .arg("__reconcile-update")
                        .stdin(Stdio::null())
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .status()
                        .context("Procora 已更新，但无法启动新版本对账全局 Center")?;
                    if !status.success() {
                        anyhow::bail!("Procora 已更新，但新版本无法对账全局 Center：{status}");
                    }
                }
                Command::new(destination)
                    .arg("__cleanup-update")
                    .arg("--path")
                    .arg(source)
                    .spawn()
                    .context("无法启动 Windows 更新暂存清理器")?;
                return Ok(());
            }
            Err(error) => last_error = Some(error),
        }
        thread::sleep(Duration::from_millis(100));
    }
    let _ = fs::remove_file(&staged);
    Err(last_error
        .context("Windows 更新等待当前进程退出超时")?
        .into())
}

/// Windows 清理器等待更新助手退出后移除其暂存目录。
#[cfg(target_os = "windows")]
pub(crate) fn cleanup_windows(path: &Path) -> anyhow::Result<()> {
    use std::{thread, time::Duration};

    for _ in 0..300 {
        match fs::remove_file(path) {
            Ok(()) => {
                if let Some(parent) = path.parent() {
                    let _ = fs::remove_dir(parent);
                }
                return Ok(());
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(_) => thread::sleep(Duration::from_millis(100)),
        }
    }
    anyhow::bail!("Windows 更新助手暂存文件仍被占用：{}", path.display())
}
