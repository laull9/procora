//! 开机自启动 CLI 命令及 Windows 提权子流程。

use std::{thread, time::Duration};

#[cfg(target_os = "windows")]
use std::path::Path;

use anyhow::{Context, bail};

use crate::{
    daemon::CenterClient,
    platform::autostart::{self, AutostartBackend, DaemonAutostart},
};

use super::center_runtime;

/// 注册自启动；Windows 会先显式请求 UAC 提权。
pub(super) fn enable() -> anyhow::Result<()> {
    #[cfg(target_os = "windows")]
    {
        let message = super::elevation::request("enable")?;
        println!("{message}");
        Ok(())
    }
    #[cfg(not(target_os = "windows"))]
    {
        let backend = enable_inner()?;
        println!("已启用开机自启动：{}", backend.label());
        Ok(())
    }
}

/// 移除自启动；Windows 会先显式请求 UAC 提权。
pub(super) fn disable() -> anyhow::Result<()> {
    #[cfg(target_os = "windows")]
    {
        let message = super::elevation::request("disable")?;
        println!("{message}");
        Ok(())
    }
    #[cfg(not(target_os = "windows"))]
    {
        let backend = disable_inner()?;
        println!("已禁用开机自启动：{}", backend.label());
        Ok(())
    }
}

/// 执行已经由 Windows 确认提权的操作并写回完整诊断。
#[cfg(target_os = "windows")]
pub(super) fn complete_elevated(action: &str, result_path: &Path) -> anyhow::Result<()> {
    let result = match action {
        "enable" => enable_inner().map(|_| "已启用开机自启动：Windows 任务计划程序"),
        "disable" => disable_inner().map(|_| "已禁用开机自启动：Windows 任务计划程序"),
        _ => Err(anyhow::anyhow!("未知的 Windows 提权操作 `{action}`")),
    };
    super::elevation::write_result(result_path, &result)
}

/// 安装自启动定义并等待中心服务器就绪。
fn enable_inner() -> anyhow::Result<AutostartBackend> {
    let paths = center_runtime::center_paths()?;
    let client = CenterClient::new(paths.endpoint.clone());
    if client.ping() {
        center_runtime::shutdown_center(&client).context("无法把现有全局服务器移交给系统托管")?;
    }
    center_runtime::install_current_executable(&paths.executable)
        .context("安装中心 Procora 可执行文件失败")?;
    let definition = DaemonAutostart::new(&paths.executable, &paths.endpoint, &paths.database);
    let backend = definition.enable().context("注册开机自启动失败")?;

    for _ in 0..250 {
        if client.ping() {
            client.hello("procora-cli")?;
            return Ok(backend);
        }
        thread::sleep(Duration::from_millis(20));
    }
    bail!(
        "{} 已注册，但全局 Procora 服务器未在 5 秒内就绪",
        backend.label()
    )
}

/// 停止中心服务器并移除自启动定义。
fn disable_inner() -> anyhow::Result<AutostartBackend> {
    let paths = center_runtime::center_paths()?;
    let client = CenterClient::new(paths.endpoint);
    if client.ping() {
        center_runtime::shutdown_center(&client).context("停止自启动全局服务器失败")?;
    }
    autostart::disable().context("移除开机自启动失败")
}
