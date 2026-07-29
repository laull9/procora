//! 通过 OpenSSH 执行参数已经过领域校验的远端 Procora CLI 命令。

use std::process::{ExitStatus, Stdio};

use anyhow::anyhow;

use super::{
    remote::{resolve_ssh_target, validate_remote_bin},
    remote_auth::{SshAuth, base_ssh, confirm_host_key},
    remote_error::{LoginFailure, classify_login_failure, remote_command_missing},
};

/// 一次远端 CLI 命令的标准输出与非致命标准错误。
#[derive(Debug)]
pub(crate) struct RemoteCliOutput {
    pub(crate) stdout: Vec<u8>,
    pub(crate) stderr: Vec<u8>,
}

/// 一次远端 CLI 执行失败及其可恢复类别。
struct RemoteCliFailure {
    error: anyhow::Error,
    login_failure: LoginFailure,
    remote_missing: bool,
}

/// 解析 SSH 目标、处理认证回退并执行远端 Procora 命令。
pub(crate) fn run_remote_cli(
    configured_target: Option<&str>,
    configured_remote_bin: Option<&str>,
    batch: bool,
    arguments: &[String],
) -> anyhow::Result<RemoteCliOutput> {
    let ssh_target = resolve_ssh_target(configured_target, batch)?;
    let mut remote_bin = configured_remote_bin.unwrap_or("procora").to_owned();
    validate_remote_bin(&remote_bin)?;
    let mut auth = SshAuth::automatic();
    let mut attempt = execute(&ssh_target, &remote_bin, &auth, arguments);
    if attempt
        .as_ref()
        .is_err_and(|failure| failure.login_failure != LoginFailure::None)
    {
        if batch {
            let failure = attempt.expect_err("已确认 SSH 登录失败");
            return Err(failure
                .error
                .context("SSH 自动登录失败（batch 模式不会确认主机或询问密码）"));
        }
        if attempt
            .as_ref()
            .is_err_and(|failure| failure.login_failure == LoginFailure::HostKey)
        {
            confirm_host_key(&ssh_target)?;
            attempt = execute(&ssh_target, &remote_bin, &auth, arguments);
        }
        if attempt
            .as_ref()
            .is_err_and(|failure| failure.login_failure == LoginFailure::Authentication)
        {
            eprintln!("SSH 密钥自动登录不可用，改用一次性内存密码。");
            auth = SshAuth::prompt_password(&ssh_target)?;
            attempt = execute(&ssh_target, &remote_bin, &auth, arguments);
        }
    }
    match attempt {
        Ok(output) => Ok(output),
        Err(failure) if failure.remote_missing => {
            remote_bin = super::remote_binary::resolve_after_missing(
                &ssh_target,
                configured_remote_bin,
                batch,
                &auth,
                failure.error,
            )?;
            execute(&ssh_target, &remote_bin, &auth, arguments).map_err(|failure| failure.error)
        }
        Err(failure) => Err(failure.error),
    }
}

/// 执行一次不经过本机 shell 的固定远端命令。
fn execute(
    ssh_target: &str,
    remote_bin: &str,
    auth: &SshAuth,
    arguments: &[String],
) -> Result<RemoteCliOutput, RemoteCliFailure> {
    let mut command = base_ssh(auth).map_err(|error| RemoteCliFailure {
        error,
        login_failure: LoginFailure::None,
        remote_missing: false,
    })?;
    let output = command
        .arg(ssh_target)
        .arg(remote_bin)
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|error| RemoteCliFailure {
            error: anyhow!(error).context("无法启动本机 ssh；请先安装 OpenSSH 客户端"),
            login_failure: LoginFailure::None,
            remote_missing: false,
        })?;
    if output.status.success() {
        return Ok(RemoteCliOutput {
            stdout: output.stdout,
            stderr: output.stderr,
        });
    }
    let detail = crate::platform::decode_external_output(&output.stderr);
    Err(RemoteCliFailure {
        error: command_error(output.status, &detail, remote_bin),
        login_failure: classify_login_failure(output.status.code(), &output.stderr),
        remote_missing: remote_command_missing(output.status.code(), &detail),
    })
}

/// 把远端 CLI 退出状态转换为不带上传语义的诊断。
fn command_error(status: ExitStatus, detail: &str, remote_bin: &str) -> anyhow::Error {
    let detail = detail.trim();
    if remote_command_missing(status.code(), detail) {
        anyhow!(
            "远端无法启动 `{remote_bin}`：{detail}；可尝试 `--remote-bin ~/.local/bin/procora`，Windows 可使用 `--remote-bin C:/Tools/procora.exe`"
        )
    } else if detail.is_empty() {
        anyhow!("远端 Procora 命令失败：{status}")
    } else {
        anyhow!("远端 Procora 命令失败：{detail}")
    }
}
