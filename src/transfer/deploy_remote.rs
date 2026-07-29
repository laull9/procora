//! 通过 OpenSSH 向远端 Procora 发送完整 Service release。

use std::{
    io::{BufRead, BufReader, Read, Write},
    path::Path,
    process::Stdio,
};

use anyhow::{Context, anyhow, bail};

use crate::config::{DeployBinaries, DeployPlatform};

use super::{
    archive::PreparedArchive,
    deploy_prepare::{build_preview, portable_relative_path, prepare_deployment},
    deploy_protocol::{
        DEPLOY_PROTOCOL_VERSION, DeployBinaryMetadata, DeployInit, DeployResponse, DeployResult,
    },
    deploy_report::{DeployEvent, DeployOutcome, DeployPreview},
    remote::{self, SessionFailure},
    remote_auth::{SshAuth, base_ssh, confirm_host_key},
    remote_error::{
        LoginFailure, classify_login_failure, managed_deploy_unsupported, process_error,
        remote_command_missing,
    },
};

/// 一次部署会话中保持不变的验收参数。
#[derive(Clone, Copy)]
struct DeployOptions<'a> {
    timeout_ms: u64,
    stable_for_ms: u64,
    keep: u32,
    target_platform: Option<&'a DeployPlatform>,
    binaries: &'a [DeployBinaryMetadata],
}

/// 无副作用探测远端平台并构造可确认的部署预检。
#[allow(clippy::too_many_arguments)]
pub(crate) fn preview_deploy(
    source: &Path,
    project: &str,
    config_path: &Path,
    binaries: &DeployBinaries,
    configured_target: &str,
    configured_remote_bin: Option<&str>,
    timeout_ms: u64,
    stable_for_ms: u64,
    keep: u32,
) -> anyhow::Result<DeployPreview> {
    let mut remote_bin = configured_remote_bin.unwrap_or("procora").to_owned();
    remote::validate_remote_bin(&remote_bin)?;
    let ssh_target = remote::resolve_ssh_target(Some(configured_target), true)?;
    let mut auth = SshAuth::automatic();
    let prepared = prepare_deployment(
        source,
        binaries,
        &ssh_target,
        configured_remote_bin,
        &mut remote_bin,
        &mut auth,
        true,
    )?;
    build_preview(
        source,
        project,
        config_path,
        &ssh_target,
        &remote_bin,
        timeout_ms,
        stable_for_ms,
        keep,
        &prepared,
    )
}

/// 校验、归档并发送完整 Service。
#[allow(clippy::too_many_arguments)]
pub(crate) fn deploy(
    source: &Path,
    project: &str,
    config_path: &Path,
    binaries: &DeployBinaries,
    configured_target: Option<&str>,
    configured_remote_bin: Option<&str>,
    timeout_ms: u64,
    stable_for_ms: u64,
    keep: u32,
    batch: bool,
    expected_revision: Option<&str>,
    reporter: &mut dyn FnMut(&DeployEvent),
) -> anyhow::Result<DeployOutcome> {
    let mut remote_bin = configured_remote_bin.unwrap_or("procora").to_owned();
    remote::validate_remote_bin(&remote_bin)?;
    let ssh_target = remote::resolve_ssh_target(configured_target, batch)?;
    let mut auth = SshAuth::automatic();
    let prepared = prepare_deployment(
        source,
        binaries,
        &ssh_target,
        configured_remote_bin,
        &mut remote_bin,
        &mut auth,
        batch,
    )?;
    let preview = build_preview(
        source,
        project,
        config_path,
        &ssh_target,
        &remote_bin,
        timeout_ms,
        stable_for_ms,
        keep,
        &prepared,
    );
    let preview = preview?;
    if let Some(expected) = expected_revision
        && expected != preview.revision
    {
        bail!(
            "部署预检修订已经变化：期望 `{expected}`，当前 `{}`；请重新执行 preview_deploy",
            preview.revision
        );
    }
    let mut events = Vec::new();
    report_prepared(&preview, &prepared.archive, &mut events, reporter);
    let options = DeployOptions {
        timeout_ms,
        stable_for_ms,
        keep,
        target_platform: Some(&prepared.target_platform),
        binaries: &prepared.binaries,
    };
    let result = transfer_with_fallback(
        &ssh_target,
        &mut remote_bin,
        configured_remote_bin,
        batch,
        &mut auth,
        project,
        config_path,
        &prepared.archive,
        options,
        &mut events,
        reporter,
    )?;
    Ok(deploy_outcome(result, preview, events))
}

/// 把协议结果与预检、阶段事件组合为共享输出。
fn deploy_outcome(
    result: DeployResult,
    preview: DeployPreview,
    events: Vec<DeployEvent>,
) -> DeployOutcome {
    DeployOutcome {
        project: result.project,
        release: result.release,
        previous_release: result.previous_release,
        preview,
        events,
    }
}

/// 记录一条共享事件并立即交给当前入口渲染。
fn record_event(
    events: &mut Vec<DeployEvent>,
    reporter: &mut dyn FnMut(&DeployEvent),
    phase: impl Into<String>,
    message: impl Into<String>,
) {
    let event = DeployEvent::new(phase, message);
    reporter(&event);
    events.push(event);
}

/// 把平台、变体和归档摘要转换为共享预检事件。
fn report_prepared(
    preview: &DeployPreview,
    archive: &PreparedArchive,
    events: &mut Vec<DeployEvent>,
    reporter: &mut dyn FnMut(&DeployEvent),
) {
    record_event(
        events,
        reporter,
        "preflight",
        format!("远端平台：{}", preview.target_platform.key()),
    );
    for binary in &preview.binaries {
        record_event(
            events,
            reporter,
            "binary",
            format!(
                "选择 `{}`：{}，{} → {}",
                binary.name,
                binary.selector,
                binary.source.display(),
                binary.target
            ),
        );
    }
    record_event(
        events,
        reporter,
        "archive",
        format!(
            "已准备 Service `{}`：{}（压缩后 {}）",
            preview.project,
            remote::human_bytes(archive.content_bytes),
            remote::human_bytes(archive.archive_bytes)
        ),
    );
}

/// 完成SSH登录回退、远端命令发现和最终部署会话。
#[allow(clippy::too_many_arguments)]
fn transfer_with_fallback(
    ssh_target: &str,
    remote_bin: &mut String,
    configured_remote_bin: Option<&str>,
    batch: bool,
    auth: &mut SshAuth,
    project: &str,
    config_path: &Path,
    archive: &PreparedArchive,
    options: DeployOptions<'_>,
    events: &mut Vec<DeployEvent>,
    reporter: &mut dyn FnMut(&DeployEvent),
) -> anyhow::Result<DeployResult> {
    let mut attempt = transfer(
        ssh_target,
        remote_bin,
        auth,
        project,
        config_path,
        archive,
        options,
        events,
        reporter,
    );
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
            attempt = transfer(
                ssh_target,
                remote_bin,
                auth,
                project,
                config_path,
                archive,
                options,
                events,
                reporter,
            );
        }
        if attempt
            .as_ref()
            .is_err_and(|failure| failure.login_failure == LoginFailure::Authentication)
        {
            record_event(
                events,
                reporter,
                "authentication",
                "SSH 密钥自动登录不可用，改用一次性内存密码",
            );
            *auth = SshAuth::prompt_password(ssh_target)?;
            attempt = transfer(
                ssh_target,
                remote_bin,
                auth,
                project,
                config_path,
                archive,
                options,
                events,
                reporter,
            );
        }
    }
    match attempt {
        Ok(result) => Ok(result),
        Err(failure) if failure.remote_missing => {
            *remote_bin = super::remote_binary::resolve_after_missing(
                ssh_target,
                configured_remote_bin,
                batch,
                auth,
                failure.error,
            )?;
            transfer(
                ssh_target,
                remote_bin,
                auth,
                project,
                config_path,
                archive,
                options,
                events,
                reporter,
            )
            .map_err(|failure| failure.error)
        }
        Err(failure) => Err(failure.error),
    }
}

/// 在一条 SSH 连接中完成部署协商、正文发送和结果读取。
#[allow(clippy::too_many_arguments)]
fn transfer(
    ssh_target: &str,
    remote_bin: &str,
    auth: &SshAuth,
    project: &str,
    config_path: &Path,
    archive: &PreparedArchive,
    options: DeployOptions<'_>,
    events: &mut Vec<DeployEvent>,
    reporter: &mut dyn FnMut(&DeployEvent),
) -> Result<DeployResult, SessionFailure> {
    let mut command = base_ssh(auth).map_err(|error| SessionFailure {
        error,
        login_failure: LoginFailure::None,
        remote_missing: false,
        target_missing: false,
    })?;
    command
        .arg(ssh_target)
        .args([remote_bin, "__receive-deploy"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().map_err(|error| {
        remote::local_ssh_failure(error, "无法启动本机 ssh；请先安装 OpenSSH 客户端")
    })?;
    let stderr = child.stderr.take().expect("SSH 子进程已配置 stderr 管道");
    let stderr_reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let mut stderr = stderr;
        let _ = stderr.read_to_end(&mut bytes);
        bytes
    });
    let mut input = child.stdin.take().expect("SSH 子进程已配置 stdin 管道");
    let mut output = BufReader::new(child.stdout.take().expect("SSH 子进程已配置 stdout 管道"));
    let mut negotiated = false;
    let operation = exchange(
        &mut input,
        &mut output,
        project,
        config_path,
        archive,
        options,
        &mut negotiated,
        events,
        reporter,
    );
    drop(input);
    let status = child
        .wait()
        .map_err(|error| remote::local_ssh_failure(error, "等待 SSH 部署进程失败"))?;
    let stderr = stderr_reader.join().unwrap_or_default();
    match (operation, status.success()) {
        (Ok(result), true) => Ok(result),
        (operation, _) => {
            let stderr_text = crate::platform::decode_external_output(&stderr);
            let error = if !negotiated && managed_deploy_unsupported(&stderr) {
                anyhow!("远端 Procora 尚不支持全托管部署；请升级远端 Procora 后重试 `deploy`")
            } else {
                match operation {
                    Err(error) if stderr.is_empty() => error,
                    Ok(_) | Err(_) => process_error(status, &stderr, remote_bin),
                }
            };
            Err(SessionFailure {
                error,
                login_failure: if negotiated {
                    LoginFailure::None
                } else {
                    classify_login_failure(status.code(), &stderr)
                },
                remote_missing: !negotiated && remote_command_missing(status.code(), &stderr_text),
                target_missing: false,
            })
        }
    }
}

/// 交换部署元数据、归档正文和最终结果。
#[allow(clippy::too_many_arguments)]
fn exchange(
    input: &mut impl Write,
    output: &mut impl BufRead,
    project: &str,
    config_path: &Path,
    archive: &PreparedArchive,
    options: DeployOptions<'_>,
    negotiated: &mut bool,
    events: &mut Vec<DeployEvent>,
    reporter: &mut dyn FnMut(&DeployEvent),
) -> anyhow::Result<DeployResult> {
    send_json(
        input,
        &DeployInit {
            protocol: DEPLOY_PROTOCOL_VERSION,
            project: project.to_owned(),
            config_path: portable_relative_path(config_path)?,
            archive_bytes: archive.archive_bytes,
            content_bytes: archive.content_bytes,
            sha256: archive.sha256.clone(),
            timeout_ms: options.timeout_ms,
            stable_for_ms: options.stable_for_ms,
            keep: options.keep,
            target_platform: options.target_platform.cloned(),
            binaries: options.binaries.to_vec(),
        },
    )?;
    match read_response(output)? {
        DeployResponse::Ready { project: ready } if ready == project => {}
        DeployResponse::Ready { project: ready } => {
            bail!("远端确认了意外 Service `{ready}`")
        }
        DeployResponse::Progress { .. } => bail!("远端在接收正文前提前报告部署进度"),
        DeployResponse::Complete { .. } => bail!("远端在接收正文前提前结束部署"),
    }
    *negotiated = true;
    remote::copy_with_progress(&mut archive.open()?, input, archive.archive_bytes)?;
    input.flush()?;
    loop {
        match read_response(output)? {
            DeployResponse::Progress { phase, message } => {
                record_event(events, reporter, phase.label(), message);
            }
            DeployResponse::Complete { result } => return Ok(result),
            DeployResponse::Ready { .. } => bail!("远端重复返回了部署接收确认"),
        }
    }
}

/// 读取一条有界部署响应。
fn read_response(input: &mut impl BufRead) -> anyhow::Result<DeployResponse> {
    let mut bytes = Vec::new();
    input.take(64 * 1024).read_until(b'\n', &mut bytes)?;
    if bytes.is_empty() || !bytes.ends_with(b"\n") {
        bail!("远端没有返回完整的部署协议消息");
    }
    serde_json::from_slice(&bytes).context("远端返回了无效部署协议消息")
}

/// 写入并刷新一条 JSON 协议消息。
fn send_json(output: &mut impl Write, value: &impl serde::Serialize) -> anyhow::Result<()> {
    serde_json::to_writer(&mut *output, value)?;
    output.write_all(b"\n")?;
    output.flush()?;
    Ok(())
}
