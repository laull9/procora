use std::process::Stdio;

use anyhow::{Context, anyhow};

use crate::protocol::UploadTargetViewDto;

use super::{
    remote::{SessionFailure, validate_remote_bin, validate_ssh_target},
    remote_auth::{SshAuth, base_ssh, confirm_host_key},
    remote_error::{LoginFailure, classify_login_failure, process_error, remote_command_missing},
};

/// 通过 SSH 获取远端 Center 当前活动上传目标。
pub(crate) fn list_remote(
    configured_target: &str,
    remote_bin: Option<&str>,
    batch: bool,
) -> anyhow::Result<Vec<UploadTargetViewDto>> {
    let configured_remote_bin = remote_bin;
    let mut remote_bin = remote_bin.unwrap_or("procora").to_owned();
    validate_remote_bin(&remote_bin)?;
    validate_ssh_target(configured_target)?;
    let mut auth = SshAuth::automatic();
    let mut attempt = list_session(configured_target, &remote_bin, &auth);
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
            confirm_host_key(configured_target)?;
            attempt = list_session(configured_target, &remote_bin, &auth);
        }
        if attempt
            .as_ref()
            .is_err_and(|failure| failure.login_failure == LoginFailure::Authentication)
        {
            eprintln!("SSH 密钥自动登录不可用，改用一次性内存密码。");
            auth = SshAuth::prompt_password(configured_target)?;
            attempt = list_session(configured_target, &remote_bin, &auth);
        }
    }
    finish_list(
        attempt,
        configured_target,
        configured_remote_bin,
        &mut remote_bin,
        batch,
        &auth,
    )
}

/// 在命令缺失时完成远端路径回退，并再次读取上传目标。
fn finish_list(
    attempt: Result<Vec<UploadTargetViewDto>, SessionFailure>,
    target: &str,
    configured_remote_bin: Option<&str>,
    remote_bin: &mut String,
    batch: bool,
    auth: &SshAuth,
) -> anyhow::Result<Vec<UploadTargetViewDto>> {
    match attempt {
        Ok(targets) => Ok(targets),
        Err(failure) if failure.remote_missing => {
            *remote_bin = super::remote_binary::resolve_after_missing(
                target,
                configured_remote_bin,
                batch,
                auth,
                failure.error,
            )?;
            list_session(target, remote_bin, auth).map_err(|failure| failure.error)
        }
        Err(failure) => Err(failure.error),
    }
}

/// 运行一次有界的远端上传目标查询。
fn list_session(
    ssh_target: &str,
    remote_bin: &str,
    auth: &SshAuth,
) -> Result<Vec<UploadTargetViewDto>, SessionFailure> {
    let mut command = base_ssh(auth).map_err(|error| SessionFailure {
        error,
        login_failure: LoginFailure::None,
        remote_missing: false,
        target_missing: false,
    })?;
    command
        .arg(ssh_target)
        .args([remote_bin, "__upload-targets"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let output = command.output().map_err(|error| SessionFailure {
        error: anyhow!(error).context("无法启动本机 ssh；请先安装 OpenSSH 客户端"),
        login_failure: LoginFailure::None,
        remote_missing: false,
        target_missing: false,
    })?;
    if !output.status.success() {
        return Err(SessionFailure {
            error: process_error(output.status, &output.stderr, remote_bin),
            login_failure: classify_login_failure(output.status.code(), &output.stderr),
            remote_missing: remote_command_missing(
                output.status.code(),
                &crate::platform::decode_external_output(&output.stderr),
            ),
            target_missing: false,
        });
    }
    if output.stdout.len() > 1024 * 1024 {
        return Err(SessionFailure {
            error: anyhow!("远端上传目标清单超过 1 MiB，拒绝解析"),
            login_failure: LoginFailure::None,
            remote_missing: false,
            target_missing: false,
        });
    }
    serde_json::from_slice(&output.stdout)
        .context("远端返回了无效上传目标清单")
        .map_err(|error| SessionFailure {
            error,
            login_failure: LoginFailure::None,
            remote_missing: false,
            target_missing: false,
        })
}
