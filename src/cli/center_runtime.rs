//! 中心服务器可执行文件安装、版本对账与后台进程管理。

use std::{
    collections::hash_map::DefaultHasher,
    env, fs,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    process::{Command as ProcessCommand, Stdio},
    thread,
    time::Duration,
};

use crate::{
    daemon::CenterClient,
    protocol::{CenterRequest, CenterResponse},
};
use anyhow::{Context, bail};
use directories::ProjectDirs;

/// 全新安装或版本替换后等待中心服务就绪的最长时间。
const CENTER_START_TIMEOUT: Duration = Duration::from_secs(5);

/// 当前用户中心服务器使用的稳定 IPC、数据库和可执行文件位置。
#[derive(Clone, Debug)]
pub(super) struct CenterPaths {
    /// 当前用户独立的本地 IPC 端点。
    pub(super) endpoint: String,
    /// 中心服务注册表数据库。
    pub(super) database: PathBuf,
    /// 由当前 CLI 自动维护的中心服务可执行文件副本。
    pub(super) executable: PathBuf,
}

/// 只连接正在运行的中心服务器，并在版本落后时自动替换。
pub(super) fn running_center() -> anyhow::Result<Option<CenterClient>> {
    let paths = center_paths()?;
    let client = CenterClient::new(paths.endpoint.clone());
    if !client.ping() {
        return Ok(None);
    }
    reconcile_running_center(client, &paths).map(Some)
}

/// 连接中心服务器，不存在时安装当前版本并启动后台进程。
pub(super) fn ensure_center() -> anyhow::Result<CenterClient> {
    let paths = center_paths()?;
    let client = CenterClient::new(paths.endpoint.clone());
    if client.ping() {
        return reconcile_running_center(client, &paths);
    }
    install_current_executable(&paths.executable)?;
    spawn_center_process(&paths.executable, &paths).context("无法启动全局 Procora 服务器")?;
    wait_until_ready(&client, CENTER_START_TIMEOUT)?;
    client.hello("procora-cli")?;
    Ok(client)
}

/// 对账已运行中心版本，落后时正常停机并换成当前 CLI 版本。
fn reconcile_running_center(
    client: CenterClient,
    paths: &CenterPaths,
) -> anyhow::Result<CenterClient> {
    if client
        .hello("procora-cli")
        .is_ok_and(|hello| hello.uses_current_version())
    {
        return Ok(client);
    }

    shutdown_center(&client).context("停止旧版本全局 Procora 服务器失败")?;
    install_current_executable(&paths.executable)?;
    spawn_center_process(&paths.executable, paths)
        .context("启动更新后的全局 Procora 服务器失败")?;
    let updated = CenterClient::new(paths.endpoint.clone());
    wait_until_ready(&updated, CENTER_START_TIMEOUT)?;
    let hello = updated.hello("procora-cli")?;
    if !hello.uses_current_version() {
        bail!("全局 Procora 服务器更新后仍未运行当前版本")
    }
    Ok(updated)
}

/// 请求中心服务器正常退出并等待端点关闭。
pub(super) fn shutdown_center(client: &CenterClient) -> anyhow::Result<()> {
    match client.request(&CenterRequest::Shutdown)? {
        CenterResponse::ShuttingDown => {}
        CenterResponse::Error { message } => bail!(message),
        response => bail!("全局 Procora 服务器返回了意外响应: {response:?}"),
    }
    for _ in 0..100 {
        if !client.ping() {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(20));
    }
    bail!("全局 Procora 服务器未在 2 秒内退出")
}

/// 计算当前用户独立的中心服务路径。
pub(super) fn center_paths() -> anyhow::Result<CenterPaths> {
    let home = if let Some(path) = env::var_os("PROCORA_HOME") {
        PathBuf::from(path)
    } else {
        ProjectDirs::from("dev", "procora", "Procora")
            .context("当前平台没有可用的用户数据目录")?
            .data_local_dir()
            .to_path_buf()
    };
    let mut hasher = DefaultHasher::new();
    home.hash(&mut hasher);
    let executable_name = if cfg!(windows) {
        "procora.exe"
    } else {
        "procora"
    };
    Ok(CenterPaths {
        endpoint: format!("procora-center-{:016x}", hasher.finish()),
        database: home.join("procora.sqlite3"),
        executable: home.join("bin").join(executable_name),
    })
}

/// 把当前 CLI 可执行文件通过同目录临时文件替换到中心稳定路径。
pub(super) fn install_current_executable(destination: &Path) -> anyhow::Result<()> {
    let source = env::current_exe().context("无法定位当前 procora 可执行文件")?;
    if source == destination {
        return Ok(());
    }
    let parent = destination.parent().context("中心可执行文件缺少父目录")?;
    fs::create_dir_all(parent).context("无法创建 Procora 中心可执行文件目录")?;
    let temporary = parent.join(format!(".procora-update-{}", std::process::id()));
    if temporary.exists() {
        fs::remove_file(&temporary).context("无法清理遗留的 Procora 更新临时文件")?;
    }
    fs::copy(&source, &temporary).context("无法复制当前 Procora 到中心目录")?;
    replace_file(&temporary, destination).context("无法替换中心 Procora 可执行文件")?;
    Ok(())
}

/// 在支持覆盖重命名的平台原子替换文件，Windows 使用停机后的删除再重命名。
fn replace_file(source: &Path, destination: &Path) -> std::io::Result<()> {
    #[cfg(windows)]
    if destination.exists() {
        fs::remove_file(destination)?;
    }
    fs::rename(source, destination)
}

/// 启动与当前终端会话分离的中心服务器子进程。
fn spawn_center_process(executable: &Path, paths: &CenterPaths) -> std::io::Result<()> {
    let mut command = ProcessCommand::new(executable);
    command
        .arg("__daemon")
        .arg("--endpoint")
        .arg(&paths.endpoint)
        .arg("--database")
        .arg(&paths.database)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    #[cfg(unix)]
    {
        use process_wrap::std::{CommandWrap, ProcessSession};

        let mut command = CommandWrap::from(command);
        command.wrap(ProcessSession);
        command.spawn()?;
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;

        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        command
            .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP)
            .spawn()?;
    }
    Ok(())
}

/// 等待中心服务器在给定期限内开始响应探测。
fn wait_until_ready(client: &CenterClient, timeout: Duration) -> anyhow::Result<()> {
    let attempts = timeout.as_millis().div_ceil(20);
    for _ in 0..attempts {
        if client.ping() {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(20));
    }
    bail!("全局 Procora 服务器未在 {} 秒内就绪", timeout.as_secs())
}
