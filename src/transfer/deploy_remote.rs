//! 通过 OpenSSH 向远端 Procora 发送完整 Service release。

use std::{
    io::{BufRead, BufReader, Read, Write},
    path::{Component, Path},
    process::Stdio,
};

use anyhow::{Context, anyhow, bail};

use super::{
    archive::{self, PreparedArchive},
    deploy_protocol::{DEPLOY_PROTOCOL_VERSION, DeployInit, DeployResponse, DeployResult},
    remote::{self, SessionFailure},
    remote_auth::{SshAuth, base_ssh, confirm_host_key},
    remote_error::{
        LoginFailure, classify_login_failure, managed_deploy_unsupported, process_error,
        remote_command_missing,
    },
};

/// CLI 展示所需的成功部署摘要。
pub(crate) struct DeployOutcome {
    pub(crate) project: String,
    pub(crate) release: String,
    pub(crate) previous_release: Option<String>,
}

/// 一次部署会话中保持不变的验收参数。
#[derive(Clone, Copy)]
struct DeployOptions {
    timeout_ms: u64,
    stable_for_ms: u64,
    keep: u32,
}

/// 校验、归档并发送完整 Service。
#[allow(clippy::too_many_arguments)]
pub(crate) fn deploy(
    source: &Path,
    project: &str,
    config_path: &Path,
    configured_target: Option<&str>,
    configured_remote_bin: Option<&str>,
    timeout_ms: u64,
    stable_for_ms: u64,
    keep: u32,
    batch: bool,
) -> anyhow::Result<DeployOutcome> {
    let mut remote_bin = configured_remote_bin.unwrap_or("procora").to_owned();
    remote::validate_remote_bin(&remote_bin)?;
    let archive = archive::prepare(source)?;
    println!(
        "已准备 Service `{project}`：{}（压缩后 {}）",
        remote::human_bytes(archive.content_bytes),
        remote::human_bytes(archive.archive_bytes)
    );
    let ssh_target = remote::resolve_ssh_target(configured_target, batch)?;
    let options = DeployOptions {
        timeout_ms,
        stable_for_ms,
        keep,
    };
    let mut auth = SshAuth::automatic();
    let mut attempt = transfer(
        &ssh_target,
        &remote_bin,
        &auth,
        project,
        config_path,
        &archive,
        options,
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
            confirm_host_key(&ssh_target)?;
            attempt = transfer(
                &ssh_target,
                &remote_bin,
                &auth,
                project,
                config_path,
                &archive,
                options,
            );
        }
        if attempt
            .as_ref()
            .is_err_and(|failure| failure.login_failure == LoginFailure::Authentication)
        {
            eprintln!("SSH 密钥自动登录不可用，改用一次性内存密码。");
            auth = SshAuth::prompt_password(&ssh_target)?;
            attempt = transfer(
                &ssh_target,
                &remote_bin,
                &auth,
                project,
                config_path,
                &archive,
                options,
            );
        }
    }
    let result = match attempt {
        Ok(result) => result,
        Err(failure) if failure.remote_missing => {
            remote_bin = super::remote_binary::resolve_after_missing(
                &ssh_target,
                configured_remote_bin,
                batch,
                &auth,
                failure.error,
            )?;
            transfer(
                &ssh_target,
                &remote_bin,
                &auth,
                project,
                config_path,
                &archive,
                options,
            )
            .map_err(|failure| failure.error)?
        }
        Err(failure) => return Err(failure.error),
    };
    Ok(DeployOutcome {
        project: result.project,
        release: result.release,
        previous_release: result.previous_release,
    })
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
    options: DeployOptions,
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
    );
    drop(input);
    let status = child
        .wait()
        .map_err(|error| remote::local_ssh_failure(error, "等待 SSH 部署进程失败"))?;
    let stderr = stderr_reader.join().unwrap_or_default();
    match (operation, status.success()) {
        (Ok(result), true) => Ok(result),
        (operation, _) => {
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
                remote_missing: !negotiated
                    && remote_command_missing(status.code(), &String::from_utf8_lossy(&stderr)),
                target_missing: false,
            })
        }
    }
}

/// 交换部署元数据、归档正文和最终结果。
fn exchange(
    input: &mut impl Write,
    output: &mut impl BufRead,
    project: &str,
    config_path: &Path,
    archive: &PreparedArchive,
    options: DeployOptions,
    negotiated: &mut bool,
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
                eprintln!("[{}] {message}", phase.label());
            }
            DeployResponse::Complete { result } => return Ok(result),
            DeployResponse::Ready { .. } => bail!("远端重复返回了部署接收确认"),
        }
    }
}

/// 把本机配置相对路径编码为与远端平台无关的 `/` 分隔文本。
fn portable_relative_path(path: &Path) -> anyhow::Result<String> {
    let mut segments = Vec::new();
    for component in path.components() {
        let Component::Normal(segment) = component else {
            bail!("部署配置入口必须是普通相对路径");
        };
        let segment = segment.to_str().context("部署配置入口必须是 UTF-8")?;
        if !portable_segment(segment) {
            bail!("部署配置入口包含不可移植的路径字符");
        }
        segments.push(segment);
    }
    if segments.is_empty() {
        bail!("部署配置入口不能为空");
    }
    Ok(segments.join("/"))
}

/// 拒绝 Windows 与 Unix 之间含义不一致的路径片段。
fn portable_segment(segment: &str) -> bool {
    !segment.is_empty()
        && !segment.ends_with(['.', ' '])
        && !segment
            .chars()
            .any(|character| character.is_control() || r#"\/<>:"|?*"#.contains(character))
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
