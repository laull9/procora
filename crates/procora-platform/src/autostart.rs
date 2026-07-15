//! 三平台当前用户级开机自启动注册。

use std::{ffi::OsString, path::PathBuf, process::Command};

use thiserror::Error;

mod render;

/// Linux 用户服务的固定单元名。
#[cfg(target_os = "linux")]
const SYSTEMD_UNIT_NAME: &str = "procora.service";
/// macOS `LaunchAgent` 的固定标签。
const LAUNCHD_LABEL: &str = "dev.procora.center";
/// Windows 任务计划程序中的固定任务名。
#[cfg(target_os = "windows")]
const WINDOWS_TASK_NAME: &str = "Procora Center";

/// 系统实际采用的当前用户级自启动后端。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AutostartBackend {
    /// Linux systemd 用户服务。
    SystemdUser,
    /// macOS `LaunchAgent`。
    LaunchAgent,
    /// Windows 当前用户登录任务。
    WindowsTask,
}

impl AutostartBackend {
    /// 返回适合 CLI 展示的后端名称。
    pub const fn label(self) -> &'static str {
        match self {
            Self::SystemdUser => "systemd 用户服务",
            Self::LaunchAgent => "macOS LaunchAgent",
            Self::WindowsTask => "Windows 任务计划程序",
        }
    }
}

/// 启动中心 daemon 所需的稳定参数。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DaemonAutostart {
    executable: PathBuf,
    endpoint: String,
    database: PathBuf,
}

impl DaemonAutostart {
    /// 创建一份指向当前可执行文件和数据目录的自启动定义。
    pub fn new(
        executable: impl Into<PathBuf>,
        endpoint: impl Into<String>,
        database: impl Into<PathBuf>,
    ) -> Self {
        Self {
            executable: executable.into(),
            endpoint: endpoint.into(),
            database: database.into(),
        }
    }

    /// 注册并立即启动当前平台的用户级中心服务。
    ///
    /// # Errors
    ///
    /// 当平台不受支持、托管文件无法写入或系统管理命令失败时返回错误。
    pub fn enable(&self) -> Result<AutostartBackend, AutostartError> {
        platform::enable(self)
    }
}

/// 移除并停止当前平台的用户级中心服务。
///
/// # Errors
///
/// 当平台不受支持、托管文件无法移除或系统管理命令失败时返回错误。
pub fn disable() -> Result<AutostartBackend, AutostartError> {
    platform::disable()
}

/// 自启动注册和移除可能产生的错误。
#[derive(Debug, Error)]
pub enum AutostartError {
    /// 当前用户的标准目录无法确定。
    #[error("当前平台无法确定用户主目录或配置目录")]
    MissingUserDirectory,
    /// 文件系统操作失败。
    #[error("{action}失败: {source}")]
    Io {
        /// 正在执行的文件系统动作。
        action: &'static str,
        /// 原始 I/O 错误。
        source: std::io::Error,
    },
    /// 原生系统管理命令返回失败状态。
    #[error("系统命令 `{program}` 执行失败: {details}")]
    CommandFailed {
        /// 失败的程序名称。
        program: String,
        /// stderr 或退出状态摘要。
        details: String,
    },
    /// 当前编译目标不支持自动托管。
    #[error("当前操作系统不支持 Procora 开机自启动托管")]
    Unsupported,
}

/// 执行系统管理命令并要求成功退出。
fn run_command(program: &str, arguments: &[OsString]) -> Result<(), AutostartError> {
    let output = Command::new(program)
        .args(arguments)
        .output()
        .map_err(|source| AutostartError::Io {
            action: "启动系统管理命令",
            source,
        })?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    Err(AutostartError::CommandFailed {
        program: program.to_owned(),
        details: if stderr.is_empty() {
            output.status.to_string()
        } else {
            stderr
        },
    })
}

/// 执行允许失败的清理命令。
#[cfg(any(target_os = "macos", target_os = "windows"))]
fn run_cleanup_command(program: &str, arguments: &[OsString]) {
    let _ = Command::new(program).args(arguments).output();
}

#[cfg(target_os = "linux")]
mod platform {
    use directories::BaseDirs;
    use std::fs;
    use std::path::PathBuf;

    use super::{
        AutostartBackend, AutostartError, DaemonAutostart, SYSTEMD_UNIT_NAME, run_command,
    };

    /// 返回 systemd 用户单元的标准路径。
    fn unit_path() -> Result<PathBuf, AutostartError> {
        BaseDirs::new()
            .map(|dirs| {
                dirs.config_dir()
                    .join("systemd/user")
                    .join(SYSTEMD_UNIT_NAME)
            })
            .ok_or(AutostartError::MissingUserDirectory)
    }

    /// 写入、加载并立即启用 systemd 用户服务。
    pub(super) fn enable(definition: &DaemonAutostart) -> Result<AutostartBackend, AutostartError> {
        let path = unit_path()?;
        fs::create_dir_all(path.parent().expect("systemd 单元路径必须有父目录")).map_err(
            |source| AutostartError::Io {
                action: "创建 systemd 用户单元目录",
                source,
            },
        )?;
        fs::write(&path, definition.systemd_unit()).map_err(|source| AutostartError::Io {
            action: "写入 systemd 用户单元",
            source,
        })?;
        run_command("systemctl", &["--user".into(), "daemon-reload".into()])?;
        run_command(
            "systemctl",
            &[
                "--user".into(),
                "enable".into(),
                "--now".into(),
                SYSTEMD_UNIT_NAME.into(),
            ],
        )?;
        Ok(AutostartBackend::SystemdUser)
    }

    /// 停止、禁用并删除 systemd 用户服务。
    pub(super) fn disable() -> Result<AutostartBackend, AutostartError> {
        let path = unit_path()?;
        if path.exists() {
            run_command(
                "systemctl",
                &[
                    "--user".into(),
                    "disable".into(),
                    "--now".into(),
                    SYSTEMD_UNIT_NAME.into(),
                ],
            )?;
            fs::remove_file(path).map_err(|source| AutostartError::Io {
                action: "删除 systemd 用户单元",
                source,
            })?;
            run_command("systemctl", &["--user".into(), "daemon-reload".into()])?;
        }
        Ok(AutostartBackend::SystemdUser)
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use directories::BaseDirs;
    use std::fs;
    use std::path::PathBuf;
    use std::process::Command;

    use super::{
        AutostartBackend, AutostartError, DaemonAutostart, LAUNCHD_LABEL, run_cleanup_command,
        run_command,
    };

    /// 返回当前用户 `LaunchAgent` plist 的标准路径。
    fn agent_path() -> Result<PathBuf, AutostartError> {
        BaseDirs::new()
            .map(|dirs| {
                dirs.home_dir()
                    .join("Library/LaunchAgents")
                    .join(format!("{LAUNCHD_LABEL}.plist"))
            })
            .ok_or(AutostartError::MissingUserDirectory)
    }

    /// 返回 launchctl 当前图形用户域。
    fn gui_domain() -> Result<String, AutostartError> {
        let output =
            Command::new("id")
                .arg("-u")
                .output()
                .map_err(|source| AutostartError::Io {
                    action: "读取当前用户 ID",
                    source,
                })?;
        if !output.status.success() {
            return Err(AutostartError::CommandFailed {
                program: "id".to_owned(),
                details: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            });
        }
        Ok(format!(
            "gui/{}",
            String::from_utf8_lossy(&output.stdout).trim()
        ))
    }

    /// 写入、加载并立即启用 `LaunchAgent`。
    pub(super) fn enable(definition: &DaemonAutostart) -> Result<AutostartBackend, AutostartError> {
        let path = agent_path()?;
        let domain = gui_domain()?;
        fs::create_dir_all(path.parent().expect("LaunchAgent 路径必须有父目录")).map_err(
            |source| AutostartError::Io {
                action: "创建 LaunchAgents 目录",
                source,
            },
        )?;
        if let Some(parent) = definition.database.parent() {
            fs::create_dir_all(parent).map_err(|source| AutostartError::Io {
                action: "创建中心数据目录",
                source,
            })?;
        }
        run_cleanup_command(
            "launchctl",
            &["bootout".into(), format!("{domain}/{LAUNCHD_LABEL}").into()],
        );
        fs::write(&path, definition.launch_agent_plist()).map_err(|source| AutostartError::Io {
            action: "写入 LaunchAgent plist",
            source,
        })?;
        run_command(
            "launchctl",
            &["enable".into(), format!("{domain}/{LAUNCHD_LABEL}").into()],
        )?;
        run_command(
            "launchctl",
            &[
                "bootstrap".into(),
                domain.clone().into(),
                path.into_os_string(),
            ],
        )?;
        run_command(
            "launchctl",
            &[
                "kickstart".into(),
                "-k".into(),
                format!("{domain}/{LAUNCHD_LABEL}").into(),
            ],
        )?;
        Ok(AutostartBackend::LaunchAgent)
    }

    /// 卸载、禁用并删除 `LaunchAgent`。
    pub(super) fn disable() -> Result<AutostartBackend, AutostartError> {
        let path = agent_path()?;
        let domain = gui_domain()?;
        run_cleanup_command(
            "launchctl",
            &["bootout".into(), format!("{domain}/{LAUNCHD_LABEL}").into()],
        );
        run_cleanup_command(
            "launchctl",
            &["disable".into(), format!("{domain}/{LAUNCHD_LABEL}").into()],
        );
        if path.exists() {
            fs::remove_file(path).map_err(|source| AutostartError::Io {
                action: "删除 LaunchAgent plist",
                source,
            })?;
        }
        Ok(AutostartBackend::LaunchAgent)
    }
}

#[cfg(target_os = "windows")]
mod platform {
    use std::process::Command;

    use super::{
        AutostartBackend, AutostartError, DaemonAutostart, WINDOWS_TASK_NAME, run_cleanup_command,
        run_command,
    };

    /// 创建登录触发任务并立即启动中心 daemon。
    pub(super) fn enable(definition: &DaemonAutostart) -> Result<AutostartBackend, AutostartError> {
        run_cleanup_command(
            "schtasks.exe",
            &["/End".into(), "/TN".into(), WINDOWS_TASK_NAME.into()],
        );
        run_command(
            "schtasks.exe",
            &[
                "/Create".into(),
                "/TN".into(),
                WINDOWS_TASK_NAME.into(),
                "/SC".into(),
                "ONLOGON".into(),
                "/TR".into(),
                definition.windows_task_action().into(),
                "/RL".into(),
                "LIMITED".into(),
                "/F".into(),
            ],
        )?;
        run_command(
            "schtasks.exe",
            &["/Run".into(), "/TN".into(), WINDOWS_TASK_NAME.into()],
        )?;
        Ok(AutostartBackend::WindowsTask)
    }

    /// 停止并删除当前用户的登录触发任务。
    pub(super) fn disable() -> Result<AutostartBackend, AutostartError> {
        let query = Command::new("schtasks.exe")
            .args(["/Query", "/TN", WINDOWS_TASK_NAME])
            .output()
            .map_err(|source| AutostartError::Io {
                action: "查询 Windows 自启动任务",
                source,
            })?;
        if query.status.success() {
            run_cleanup_command(
                "schtasks.exe",
                &["/End".into(), "/TN".into(), WINDOWS_TASK_NAME.into()],
            );
            run_command(
                "schtasks.exe",
                &[
                    "/Delete".into(),
                    "/TN".into(),
                    WINDOWS_TASK_NAME.into(),
                    "/F".into(),
                ],
            )?;
        }
        Ok(AutostartBackend::WindowsTask)
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod platform {
    use super::{AutostartBackend, AutostartError, DaemonAutostart};

    /// 在不支持的平台拒绝注册。
    pub(super) const fn enable(
        _definition: &DaemonAutostart,
    ) -> Result<AutostartBackend, AutostartError> {
        Err(AutostartError::Unsupported)
    }

    /// 在不支持的平台拒绝移除。
    pub(super) const fn disable() -> Result<AutostartBackend, AutostartError> {
        Err(AutostartError::Unsupported)
    }
}
