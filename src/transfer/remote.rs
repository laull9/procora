use std::{
    env,
    io::{self, BufRead, BufReader, IsTerminal, Read, Write},
    path::Path,
    process::Stdio,
    time::{Duration, Instant},
};

use anyhow::{Context, anyhow, bail};

use super::{
    archive::{self, PreparedArchive},
    protocol::{TransferInit, TransferResponse, TransferResult, TransferSelection, protocol_for},
    remote_auth::{SshAuth, base_ssh, confirm_host_key},
    remote_error::{
        LoginFailure, classify_login_failure, process_error, remote_command_missing,
        remote_target_missing, transfer_protocol_incompatible,
    },
};

/// 一次 SSH 会话失败后的分类信息。
pub(super) struct SessionFailure {
    pub(super) error: anyhow::Error,
    pub(super) login_failure: LoginFailure,
    pub(super) remote_missing: bool,
    pub(super) target_missing: bool,
}

/// 一次成功上传后供 CLI 记忆与反馈使用的结果。
pub(crate) struct PushOutcome {
    pub(crate) target: String,
    pub(crate) ssh_target: String,
    pub(crate) remote_bin: String,
}

/// 单次 SSH 传输的交互与部署选项。
#[derive(Clone, Copy)]
struct TransferOptions<'a> {
    batch_selection: bool,
    restart: bool,
    preferred_target: Option<&'a str>,
}

/// 准备本地内容、自动登录 SSH，并在连接或认证失败时进入人工回退。
pub(crate) fn push(
    source: &Path,
    selector: Option<&str>,
    configured_target: Option<&str>,
    remote_bin: Option<&str>,
    batch: bool,
    restart: bool,
    preferred_target: Option<&str>,
) -> anyhow::Result<PushOutcome> {
    let configured_remote_bin = remote_bin;
    let mut remote_bin = remote_bin.unwrap_or("procora").to_owned();
    validate_remote_bin(&remote_bin)?;
    let archive = archive::prepare(source)?;
    println!(
        "已准备：{}，{}（压缩后 {}）",
        source.display(),
        human_bytes(archive.content_bytes),
        human_bytes(archive.archive_bytes)
    );
    let ssh_target = resolve_ssh_target(configured_target, batch)?;
    let mut auth = SshAuth::automatic();
    let options = TransferOptions {
        batch_selection: batch,
        restart,
        preferred_target,
    };
    let mut attempt = transfer(&ssh_target, selector, &archive, &remote_bin, &auth, options);
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
            attempt = transfer(&ssh_target, selector, &archive, &remote_bin, &auth, options);
        }
        if attempt
            .as_ref()
            .is_err_and(|failure| failure.login_failure == LoginFailure::Authentication)
        {
            eprintln!("SSH 密钥自动登录不可用，改用一次性内存密码。");
            auth = SshAuth::prompt_password(&ssh_target)?;
            attempt = transfer(&ssh_target, selector, &archive, &remote_bin, &auth, options);
        }
    }
    let mut result = match attempt {
        Ok(result) => Ok(result),
        Err(failure) if failure.remote_missing => {
            remote_bin = super::remote_binary::resolve_after_missing(
                &ssh_target,
                configured_remote_bin,
                batch,
                &auth,
                failure.error,
            )?;
            transfer(&ssh_target, selector, &archive, &remote_bin, &auth, options)
        }
        Err(failure) => Err(failure),
    };
    if let Some(selector) = selector
        && !batch
        && result.as_ref().is_err_and(|failure| failure.target_missing)
    {
        eprintln!("远端没有上传目标 `{selector}`，正在读取可用列表供重新选择。");
        result = transfer(&ssh_target, None, &archive, &remote_bin, &auth, options);
    }
    let result = result.map_err(|failure| failure.error)?;
    println!(
        "上传完成：{} → {}（{}，SHA-256 {}）",
        source.display(),
        result.target,
        human_bytes(result.content_bytes),
        result.sha256
    );
    if result.restarted {
        println!("已自动重启：{}", service_from_selector(&result.target));
    }
    Ok(PushOutcome {
        target: result.target,
        ssh_target,
        remote_bin,
    })
}

/// 按显式参数和环境变量顺序确定 SSH 目标。
pub(crate) fn resolve_ssh_target(
    configured_target: Option<&str>,
    batch: bool,
) -> anyhow::Result<String> {
    let inferred = configured_target.map(str::to_owned).or_else(|| {
        env::var("PROCORA_SSH_TARGET")
            .ok()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
    });
    let target = match inferred {
        Some(target) => target,
        None if batch => {
            bail!(
                "缺少 SSH 地址；`--ssh <[user@]host>` 指定服务器，`--target <service::name>` 指定该服务器内的上传目标"
            )
        }
        None => prompt_target()?,
    };
    validate_ssh_target(&target)?;
    Ok(target)
}

/// 在单条 SSH 连接中完成目标协商、正文发送和结果读取。
fn transfer(
    ssh_target: &str,
    selector: Option<&str>,
    archive: &PreparedArchive,
    remote_bin: &str,
    auth: &SshAuth,
    options: TransferOptions<'_>,
) -> Result<TransferResult, SessionFailure> {
    let mut command = base_ssh(auth).map_err(|error| SessionFailure {
        error,
        login_failure: LoginFailure::None,
        remote_missing: false,
        target_missing: false,
    })?;
    command
        .arg(ssh_target)
        .args([remote_bin, "__receive"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|error| local_ssh_failure(error, "无法启动本机 ssh；请先安装 OpenSSH 客户端"))?;
    let stderr = child.stderr.take().expect("SSH 子进程已配置 stderr 管道");
    let stderr_reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let mut stderr = stderr;
        let _ = stderr.read_to_end(&mut bytes);
        bytes
    });
    let mut input = child.stdin.take().expect("SSH 子进程已配置 stdin 管道");
    let stdout = child.stdout.take().expect("SSH 子进程已配置 stdout 管道");
    let mut output = BufReader::new(stdout);
    let mut negotiated = false;
    let operation = exchange(
        &mut input,
        &mut output,
        selector,
        archive,
        options,
        &mut negotiated,
    );
    drop(input);
    let status = child.wait();
    let stderr = stderr_reader.join().unwrap_or_default();
    let status = status.map_err(|error| local_ssh_failure(error, "等待 SSH 上传进程失败"))?;
    match (operation, status.success()) {
        (Ok(result), true) => Ok(result),
        (operation, _) => {
            let process_error = process_error(status, &stderr, remote_bin);
            let stderr_text = crate::platform::decode_external_output(&stderr);
            let mut error = match operation {
                Err(error) if stderr.is_empty() => error,
                Ok(_) | Err(_) => process_error,
            };
            if options.restart && transfer_protocol_incompatible(&stderr) {
                error = error.context(
                    "远端不支持客户端请求的上传后重启能力；可升级远端 Procora，或移除 `--restart` 仅执行兼容覆盖",
                );
            }
            if !negotiated
                && remote_target_missing(&stderr_text)
                && let Some(selector) = selector
            {
                let service = service_from_selector(selector);
                error = error.context(format!(
                    "上传选择器 `{selector}` 中的 `{service}` 是远端 Procora 服务名，不是 SSH 地址；可先运行 `procora uploads --ssh <同一地址>` 查看可用选择器"
                ));
            }
            Err(SessionFailure {
                error,
                login_failure: if negotiated {
                    LoginFailure::None
                } else {
                    classify_login_failure(status.code(), &stderr)
                },
                remote_missing: !negotiated && remote_command_missing(status.code(), &stderr_text),
                target_missing: !negotiated && remote_target_missing(&stderr_text),
            })
        }
    }
}

/// 在已建立的 SSH 流中完成目标协商、正文发送与结果读取。
fn exchange(
    input: &mut impl Write,
    output: &mut impl BufRead,
    selector: Option<&str>,
    archive: &PreparedArchive,
    options: TransferOptions<'_>,
    negotiated: &mut bool,
) -> anyhow::Result<TransferResult> {
    send_json(
        input,
        &TransferInit {
            protocol: protocol_for(options.restart),
            target: selector.map(str::to_owned),
            source_kind: archive.kind,
            archive_bytes: archive.archive_bytes,
            content_bytes: archive.content_bytes,
            sha256: archive.sha256.clone(),
            select_target: !options.batch_selection,
            restart: options.restart,
        },
    )?;
    let selected = match read_response(output)? {
        TransferResponse::Ready { target } => target,
        TransferResponse::Choose {
            targets,
            invalid_target,
        } => {
            if let Some(invalid) = invalid_target {
                eprintln!("远端没有上传目标 `{invalid}`，请从可用列表重新选择。");
            }
            let target = super::remote_selection::choose_target(
                &targets,
                options.batch_selection,
                options.preferred_target,
            )?;
            send_json(
                input,
                &TransferSelection {
                    target: target.clone(),
                },
            )?;
            match read_response(output)? {
                TransferResponse::Ready { target: ready } if ready == target => ready,
                TransferResponse::Ready { target: ready } => {
                    bail!("远端确认了意外上传目标 `{ready}`")
                }
                _ => bail!("远端没有确认所选上传目标"),
            }
        }
        TransferResponse::Complete { .. } => bail!("远端在接收正文前提前结束上传"),
    };
    *negotiated = true;
    if selector.is_none() {
        eprintln!("使用远端上传目标：{selected}");
    }
    copy_with_progress(&mut archive.open()?, input, archive.archive_bytes)?;
    input.flush()?;
    match read_response(output)? {
        TransferResponse::Complete { result } => Ok(result),
        _ => bail!("远端没有返回上传完成结果"),
    }
}

/// 把本机 SSH 进程错误转换为不会触发远端或登录回退的失败。
pub(super) fn local_ssh_failure(error: io::Error, message: &'static str) -> SessionFailure {
    SessionFailure {
        error: anyhow!(error).context(message),
        login_failure: LoginFailure::None,
        remote_missing: false,
        target_missing: false,
    }
}

/// 从远端读取一条有界 JSON 协议消息。
fn read_response(input: &mut impl BufRead) -> anyhow::Result<TransferResponse> {
    let mut bytes = Vec::new();
    input.take(64 * 1024).read_until(b'\n', &mut bytes)?;
    if bytes.is_empty() || !bytes.ends_with(b"\n") {
        bail!("远端没有返回完整的上传协商消息");
    }
    serde_json::from_slice(&bytes).context("远端返回了无效上传协商消息")
}

/// 向远端发送并刷新一条 JSON 协议消息。
fn send_json(output: &mut impl Write, value: &impl serde::Serialize) -> anyhow::Result<()> {
    serde_json::to_writer(&mut *output, value)?;
    output.write_all(b"\n")?;
    output.flush()?;
    Ok(())
}

/// 复制归档正文，并仅在真实终端中显示节流后的覆盖式进度。
pub(super) fn copy_with_progress(
    input: &mut impl Read,
    output: &mut impl Write,
    total: u64,
) -> anyhow::Result<()> {
    let show_progress = io::stderr().is_terminal();
    let mut buffer = vec![0_u8; 64 * 1024];
    let mut copied = 0_u64;
    let mut last_update = None;
    loop {
        let read = input.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        output.write_all(&buffer[..read])?;
        copied = copied.saturating_add(read as u64);
        if show_progress
            && (last_update
                .is_none_or(|last: Instant| last.elapsed() >= Duration::from_millis(100))
                || copied == total)
        {
            let percent = copied.saturating_mul(100).checked_div(total).unwrap_or(100);
            eprint!(
                "\r上传中：{percent:>3}%  {} / {}",
                human_bytes(copied),
                human_bytes(total)
            );
            io::stderr().flush()?;
            last_update = Some(Instant::now());
        }
    }
    if show_progress {
        eprintln!();
    }
    Ok(())
}

/// 读取尚未由参数、环境变量或上层引导确定的 SSH 地址。
fn prompt_target() -> anyhow::Result<String> {
    if !io::stdin().is_terminal() || !io::stderr().is_terminal() {
        bail!("缺少 SSH 地址且当前不是交互终端；请用 `--ssh <[user@]host>` 指定服务器");
    }
    eprint!("SSH 地址（SSH config 别名或 [user@]host）：");
    io::stderr().flush()?;
    let mut value = String::new();
    io::stdin().read_line(&mut value)?;
    let value = value.trim();
    if value.is_empty() {
        bail!("SSH 地址不能为空")
    }
    Ok(value.to_owned())
}

/// 避免 SSH 目标被解释成额外命令行选项或不可见控制输入。
pub(super) fn validate_ssh_target(target: &str) -> anyhow::Result<()> {
    if target.is_empty()
        || target.starts_with('-')
        || target
            .chars()
            .any(|character| character.is_whitespace() || character.is_control())
    {
        bail!("SSH 目标格式无效；应为 SSH config 别名或 `[user@]host`");
    }
    Ok(())
}

/// 限制远端可执行文件参数，避免经由 SSH 远端 shell 注入命令。
pub(super) fn validate_remote_bin(value: &str) -> anyhow::Result<()> {
    if value.is_empty()
        || value.starts_with('-')
        || !value.chars().all(|character| {
            character.is_alphanumeric()
                || matches!(character, '/' | '\\' | ':' | '.' | '_' | '-' | '~')
        })
    {
        bail!("远端 Procora 路径格式无效；请使用不含空格的命令名、Unix 路径或 Windows 绝对路径");
    }
    Ok(())
}

/// 从已经过远端校验的选择器提取所属 Service。
fn service_from_selector(selector: &str) -> &str {
    selector.split("::").next().unwrap_or(selector)
}

/// 以紧凑二进制单位展示传输大小。
pub(crate) fn human_bytes(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = 1024 * KIB;
    const GIB: u64 = 1024 * MIB;
    if bytes >= GIB {
        format_unit(bytes, GIB, "GiB")
    } else if bytes >= MIB {
        format_unit(bytes, MIB, "MiB")
    } else if bytes >= KIB {
        format_unit(bytes, KIB, "KiB")
    } else {
        format!("{bytes} B")
    }
}

/// 不经过浮点数损失地保留一位二进制单位小数。
fn format_unit(bytes: u64, unit: u64, label: &str) -> String {
    let whole = bytes / unit;
    let decimal = (bytes % unit).saturating_mul(10) / unit;
    format!("{whole}.{decimal} {label}")
}

#[cfg(test)]
mod tests;
