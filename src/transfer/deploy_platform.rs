//! 探测裸机部署目标的平台与 ABI。

use std::process::Stdio;

use anyhow::anyhow;
use serde::Deserialize;

use crate::config::DeployPlatform;

use super::{
    deploy_protocol::DEPLOY_PROTOCOL_VERSION,
    remote::{self, SessionFailure},
    remote_auth::{SshAuth, base_ssh, confirm_host_key},
    remote_error::{LoginFailure, classify_login_failure, process_error, remote_command_missing},
};

/// 远端 SSH 能力探测的最小响应。
#[derive(Deserialize)]
struct PlatformProbe {
    name: String,
    platform: Option<DeployPlatform>,
    #[serde(default)]
    deploy_protocol: Option<ProtocolRange>,
}

/// 远端声明的全托管部署协议范围。
#[derive(Deserialize)]
struct ProtocolRange {
    min: u32,
    max: u32,
}

/// 完成平台探测所需的登录和远端命令路径回退。
pub(super) fn resolve_remote_platform(
    ssh_target: &str,
    configured_remote_bin: Option<&str>,
    remote_bin: &mut String,
    auth: &mut SshAuth,
    batch: bool,
) -> anyhow::Result<DeployPlatform> {
    let mut attempt = probe_platform(ssh_target, remote_bin, auth);
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
            confirm_host_key(ssh_target)?;
            attempt = probe_platform(ssh_target, remote_bin, auth);
        }
        if attempt
            .as_ref()
            .is_err_and(|failure| failure.login_failure == LoginFailure::Authentication)
        {
            eprintln!("SSH 密钥自动登录不可用，改用一次性内存密码。");
            *auth = SshAuth::prompt_password(ssh_target)?;
            attempt = probe_platform(ssh_target, remote_bin, auth);
        }
    }
    match attempt {
        Ok(platform) => Ok(platform),
        Err(failure) if failure.remote_missing => {
            *remote_bin = super::remote_binary::resolve_after_missing(
                ssh_target,
                configured_remote_bin,
                batch,
                auth,
                failure.error,
            )?;
            probe_platform(ssh_target, remote_bin, auth).map_err(|failure| failure.error)
        }
        Err(failure) => Err(failure.error),
    }
}

/// 通过无副作用能力握手读取远端 Procora 运行平台。
fn probe_platform(
    ssh_target: &str,
    remote_bin: &str,
    auth: &SshAuth,
) -> Result<DeployPlatform, SessionFailure> {
    let mut command = base_ssh(auth).map_err(|error| SessionFailure {
        error,
        login_failure: LoginFailure::None,
        remote_missing: false,
        target_missing: false,
    })?;
    let output = command
        .arg(ssh_target)
        .args([remote_bin, "__ssh-probe"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|error| {
            remote::local_ssh_failure(error, "无法启动本机 ssh；请先安装 OpenSSH 客户端")
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
    if output.stdout.len() > 64 * 1024 {
        return Err(platform_probe_failure("远端平台探测响应超过 64 KiB"));
    }
    let probe: PlatformProbe = serde_json::from_slice(&output.stdout)
        .map_err(|_| platform_probe_failure("远端返回了无效的 Procora 能力响应"))?;
    if probe.name != "procora-ssh" {
        return Err(platform_probe_failure(
            "远端命令不是兼容的 Procora SSH 接收器",
        ));
    }
    if let Some(range) = probe.deploy_protocol
        && !(range.min..=range.max).contains(&DEPLOY_PROTOCOL_VERSION)
    {
        return Err(platform_probe_failure(&format!(
            "远端全托管部署协议为 {}–{}，本机需要版本 {DEPLOY_PROTOCOL_VERSION}；请升级远端 Procora",
            range.min, range.max
        )));
    }
    let platform = probe.platform.ok_or_else(|| {
        platform_probe_failure("远端 Procora 版本不支持平台感知部署；请先升级远端 Procora")
    })?;
    platform
        .normalized()
        .map_err(|message| platform_probe_failure(&message))
}

/// 构造已登录但能力响应不兼容的探测失败。
fn platform_probe_failure(message: &str) -> SessionFailure {
    SessionFailure {
        error: anyhow!(message.to_owned()),
        login_failure: LoginFailure::None,
        remote_missing: false,
        target_missing: false,
    }
}
