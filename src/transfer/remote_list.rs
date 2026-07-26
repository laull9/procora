use std::process::Stdio;

use anyhow::{Context, anyhow};

use crate::protocol::UploadTargetViewDto;

use super::remote::{
    SessionFailure, base_ssh, process_error, prompt_target, remote_command_missing,
    validate_remote_bin, validate_ssh_target,
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
    match list_session(configured_target, &remote_bin, false) {
        Ok(targets) => Ok(targets),
        Err(failure) if failure.retryable_login && !batch => {
            eprintln!("SSH 自动登录失败：{:#}", failure.error);
            let target = prompt_target(Some(configured_target))?;
            validate_ssh_target(&target)?;
            eprintln!("将由 OpenSSH 请求主机确认或密码；Procora 不读取或保存密码。");
            finish_list(
                list_session(&target, &remote_bin, true),
                &target,
                configured_remote_bin,
                &mut remote_bin,
                batch,
                true,
            )
        }
        Err(failure) if failure.retryable_login => Err(failure
            .error
            .context("SSH 自动登录失败（batch 模式不会询问密码）")),
        Err(failure) => finish_list(
            Err(failure),
            configured_target,
            configured_remote_bin,
            &mut remote_bin,
            batch,
            false,
        ),
    }
}

/// 在命令缺失时完成远端路径回退，并再次读取上传目标。
fn finish_list(
    attempt: Result<Vec<UploadTargetViewDto>, SessionFailure>,
    target: &str,
    configured_remote_bin: Option<&str>,
    remote_bin: &mut String,
    batch: bool,
    interactive_login: bool,
) -> anyhow::Result<Vec<UploadTargetViewDto>> {
    match attempt {
        Ok(targets) => Ok(targets),
        Err(failure) if failure.remote_missing => {
            *remote_bin = super::remote_binary::resolve_after_missing(
                target,
                configured_remote_bin,
                batch,
                interactive_login,
                failure.error,
            )?;
            list_session(target, remote_bin, interactive_login).map_err(|failure| failure.error)
        }
        Err(failure) => Err(failure.error),
    }
}

/// 运行一次有界的远端上传目标查询。
fn list_session(
    ssh_target: &str,
    remote_bin: &str,
    interactive_login: bool,
) -> Result<Vec<UploadTargetViewDto>, SessionFailure> {
    let mut command = base_ssh(interactive_login);
    command
        .arg(ssh_target)
        .args([remote_bin, "__upload-targets"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let output = command.output().map_err(|error| SessionFailure {
        error: anyhow!(error).context("无法启动本机 ssh；请先安装 OpenSSH 客户端"),
        retryable_login: false,
        remote_missing: false,
    })?;
    if !output.status.success() {
        return Err(SessionFailure {
            error: process_error(output.status, &output.stderr, remote_bin),
            retryable_login: output.status.code() == Some(255),
            remote_missing: remote_command_missing(
                output.status.code(),
                &String::from_utf8_lossy(&output.stderr),
            ),
        });
    }
    if output.stdout.len() > 1024 * 1024 {
        return Err(SessionFailure {
            error: anyhow!("远端上传目标清单超过 1 MiB，拒绝解析"),
            retryable_login: false,
            remote_missing: false,
        });
    }
    serde_json::from_slice(&output.stdout)
        .context("远端返回了无效上传目标清单")
        .map_err(|error| SessionFailure {
            error,
            retryable_login: false,
            remote_missing: false,
        })
}
