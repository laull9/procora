use std::{
    env,
    io::{self, BufRead, BufReader, IsTerminal, Read, Write},
    path::Path,
    process::{Command, ExitStatus, Stdio},
    time::{Duration, Instant},
};

use anyhow::{Context, anyhow, bail};

use super::{
    archive::{self, PreparedArchive},
    protocol::{
        TRANSFER_PROTOCOL_VERSION, TransferInit, TransferResponse, TransferResult,
        TransferSelection, TransferTarget,
    },
};

/// 一次 SSH 会话失败后的登录回退判定。
struct SessionFailure {
    error: anyhow::Error,
    retryable_login: bool,
}

/// 准备本地内容、自动登录 SSH，并在连接或认证失败时进入人工回退。
pub(crate) fn push(
    source: &Path,
    selector: Option<&str>,
    configured_target: Option<&str>,
    remote_bin: &str,
    batch: bool,
) -> anyhow::Result<()> {
    validate_remote_bin(remote_bin)?;
    let archive = archive::prepare(source)?;
    println!(
        "已准备：{}，{}（压缩后 {}）",
        source.display(),
        human_bytes(archive.content_bytes),
        human_bytes(archive.archive_bytes)
    );
    let inferred = configured_target
        .map(str::to_owned)
        .or_else(|| {
            env::var("PROCORA_SSH_TARGET")
                .ok()
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty())
        })
        .or_else(|| selector.and_then(|value| value.split("::").next().map(str::to_owned)));
    let initial_target = match inferred {
        Some(target) => target,
        None if batch => {
            bail!("无法推断 SSH 目标；未指定 `--target` 时请同时提供 `--ssh <目标>`")
        }
        None => prompt_target(None)?,
    };
    validate_ssh_target(&initial_target)?;

    let result = match transfer(
        &initial_target,
        selector,
        &archive,
        remote_bin,
        false,
        batch,
    ) {
        Ok(result) => result,
        Err(failure) if failure.retryable_login && !batch => {
            eprintln!("SSH 自动登录失败：{:#}", failure.error);
            let target = prompt_target(Some(&initial_target))?;
            validate_ssh_target(&target)?;
            eprintln!("将由 OpenSSH 请求主机确认或密码；Procora 不读取或保存密码。");
            transfer(&target, selector, &archive, remote_bin, true, false)
                .map_err(|failure| failure.error)?
        }
        Err(failure) if failure.retryable_login => {
            return Err(failure
                .error
                .context("SSH 自动登录失败（batch 模式不会询问密码）"));
        }
        Err(failure) => return Err(failure.error),
    };
    println!(
        "上传完成：{} → {}（{}，SHA-256 {}）",
        source.display(),
        result.target,
        human_bytes(result.content_bytes),
        result.sha256
    );
    Ok(())
}

/// 在单条 SSH 连接中完成目标协商、正文发送和结果读取。
fn transfer(
    ssh_target: &str,
    selector: Option<&str>,
    archive: &PreparedArchive,
    remote_bin: &str,
    interactive_login: bool,
    batch_selection: bool,
) -> Result<TransferResult, SessionFailure> {
    let mut command = base_ssh(interactive_login);
    command
        .arg(ssh_target)
        .args([remote_bin, "__receive"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().map_err(|error| SessionFailure {
        error: anyhow!(error).context("无法启动本机 ssh；请先安装 OpenSSH 客户端"),
        retryable_login: false,
    })?;
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
    let operation = (|| -> anyhow::Result<TransferResult> {
        send_json(
            &mut input,
            &TransferInit {
                protocol: TRANSFER_PROTOCOL_VERSION,
                target: selector.map(str::to_owned),
                source_kind: archive.kind,
                archive_bytes: archive.archive_bytes,
                content_bytes: archive.content_bytes,
                sha256: archive.sha256.clone(),
            },
        )?;
        let selected = match read_response(&mut output)? {
            TransferResponse::Ready { target } => target,
            TransferResponse::Choose { targets } => {
                let target = choose_target(&targets, batch_selection)?;
                send_json(
                    &mut input,
                    &TransferSelection {
                        target: target.clone(),
                    },
                )?;
                match read_response(&mut output)? {
                    TransferResponse::Ready { target: ready } if ready == target => ready,
                    TransferResponse::Ready { target: ready } => {
                        bail!("远端确认了意外上传目标 `{ready}`")
                    }
                    _ => bail!("远端没有确认所选上传目标"),
                }
            }
            TransferResponse::Complete { .. } => bail!("远端在接收正文前提前结束上传"),
        };
        negotiated = true;
        if selector.is_none() {
            eprintln!("使用远端上传目标：{selected}");
        }
        copy_with_progress(&mut archive.open()?, &mut input, archive.archive_bytes)?;
        input.flush()?;
        match read_response(&mut output)? {
            TransferResponse::Complete { result } => Ok(result),
            _ => bail!("远端没有返回上传完成结果"),
        }
    })();
    drop(input);
    let status = child.wait();
    let stderr = stderr_reader.join().unwrap_or_default();
    let status = status.map_err(|error| SessionFailure {
        error: anyhow!(error).context("等待 SSH 上传进程失败"),
        retryable_login: false,
    })?;
    match (operation, status.success()) {
        (Ok(result), true) => Ok(result),
        (operation, _) => {
            let process_error = process_error(status, &stderr, remote_bin);
            let error = match operation {
                Ok(_) => process_error,
                Err(error) => anyhow!("{error:#}; {process_error:#}"),
            };
            Err(SessionFailure {
                error,
                retryable_login: !negotiated && status.code() == Some(255),
            })
        }
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

/// 在交互终端列出远端候选目标并读取编号。
fn choose_target(targets: &[TransferTarget], batch: bool) -> anyhow::Result<String> {
    if batch || !io::stdin().is_terminal() || !io::stderr().is_terminal() {
        let selectors = targets
            .iter()
            .map(|target| target.selector.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        bail!("远端有多个兼容上传目标：{selectors}；请用 `--target <选择器>` 明确指定");
    }
    eprintln!("发现多个兼容上传目标：");
    for (index, target) in targets.iter().enumerate() {
        eprintln!(
            "  {}. {}  {:?}  上限 {}",
            index + 1,
            target.selector,
            target.kind,
            human_bytes(target.max_bytes)
        );
    }
    loop {
        eprint!("选择 [1]：");
        io::stderr().flush()?;
        let mut value = String::new();
        if io::stdin().read_line(&mut value)? == 0 {
            bail!("未选择上传目标");
        }
        let value = value.trim();
        let index = if value.is_empty() {
            Some(1)
        } else {
            value.parse::<usize>().ok()
        };
        if let Some(target) = index.and_then(|index| targets.get(index.saturating_sub(1))) {
            return Ok(target.selector.clone());
        }
        eprintln!("请输入 1 到 {} 之间的编号。", targets.len());
    }
}

/// 复制归档正文，并仅在真实终端中显示节流后的覆盖式进度。
fn copy_with_progress(
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

/// 把 SSH 退出状态和远端错误转换成可操作提示。
fn process_error(status: ExitStatus, stderr: &[u8], remote_bin: &str) -> anyhow::Error {
    let message = String::from_utf8_lossy(stderr).trim().to_owned();
    let detail = if message.is_empty() {
        status.to_string()
    } else {
        message
    };
    if remote_command_missing(status.code(), &detail) {
        anyhow!(
            "远端无法启动 `{remote_bin}`：{detail}；可尝试 `--remote-bin ~/.local/bin/procora`，Windows 可使用 `--remote-bin C:/Tools/procora.exe`"
        )
    } else {
        anyhow!("SSH 上传失败：{detail}")
    }
}

/// 同时识别 Unix、PowerShell 与 cmd 的远端命令缺失诊断。
fn remote_command_missing(status_code: Option<i32>, message: &str) -> bool {
    if status_code == Some(127) {
        return true;
    }
    let message = message.to_ascii_lowercase();
    message.contains("command not found")
        || message.contains("commandnotfoundexception")
        || message.contains("is not recognized as an internal or external command")
        || message.contains("不是内部或外部命令")
}

/// 构造自动或交互模式共享的 OpenSSH 安全参数。
fn base_ssh(interactive: bool) -> Command {
    let mut command = Command::new("ssh");
    command.args([
        "-T",
        "-o",
        "ClearAllForwardings=yes",
        "-o",
        "ConnectTimeout=15",
        "-o",
        "ConnectionAttempts=1",
        "-o",
        "ServerAliveInterval=15",
        "-o",
        "ServerAliveCountMax=3",
        "-o",
        "LogLevel=ERROR",
    ]);
    if interactive {
        command.args([
            "-o",
            "BatchMode=no",
            "-o",
            "StrictHostKeyChecking=ask",
            "-o",
            "NumberOfPasswordPrompts=3",
        ]);
    } else {
        command.args(["-o", "BatchMode=yes", "-o", "StrictHostKeyChecking=yes"]);
    }
    command
}

/// 允许用户输入或修改 `[user@]host` / SSH config 别名。
fn prompt_target(default: Option<&str>) -> anyhow::Result<String> {
    if !io::stdin().is_terminal() || !io::stderr().is_terminal() {
        bail!("当前不是交互终端；请用 `--ssh <目标>` 指定地址，或先配置 SSH 密钥后重试");
    }
    if let Some(default) = default {
        eprint!("SSH 目标 [{default}]：");
    } else {
        eprint!("SSH 目标（SSH config 别名或 [user@]host）：");
    }
    io::stderr().flush()?;
    let mut value = String::new();
    io::stdin().read_line(&mut value)?;
    let value = value.trim();
    match (value.is_empty(), default) {
        (false, _) => Ok(value.to_owned()),
        (true, Some(default)) => Ok(default.to_owned()),
        (true, None) => bail!("SSH 目标不能为空"),
    }
}

/// 避免 SSH 目标被解释成额外命令行选项或不可见控制输入。
fn validate_ssh_target(target: &str) -> anyhow::Result<()> {
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
fn validate_remote_bin(value: &str) -> anyhow::Result<()> {
    if value.is_empty()
        || value.starts_with('-')
        || !value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(byte, b'/' | b'\\' | b':' | b'.' | b'_' | b'-' | b'~')
        })
    {
        bail!("远端 Procora 路径格式无效；请使用不含空格的命令名、Unix 路径或 Windows 绝对路径");
    }
    Ok(())
}

/// 以紧凑二进制单位展示传输大小。
fn human_bytes(bytes: u64) -> String {
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
mod tests {
    use super::{remote_command_missing, validate_remote_bin};

    #[test]
    // 远端可执行文件兼容 Unix 与 Windows 的无空格绝对路径。
    fn remote_binary_accepts_cross_platform_paths() {
        assert!(validate_remote_bin("/home/demo/.local/bin/procora").is_ok());
        assert!(validate_remote_bin("C:/Tools/procora.exe").is_ok());
        assert!(validate_remote_bin(r"C:\Tools\procora.exe").is_ok());
    }

    #[test]
    // 远端可执行文件仍拒绝会改变 shell 命令边界的字符。
    fn remote_binary_rejects_shell_metacharacters() {
        assert!(validate_remote_bin("procora;whoami").is_err());
        assert!(validate_remote_bin("C:/Program Files/procora.exe").is_err());
    }

    #[test]
    // Windows shell 不依赖 Unix 退出码也能识别命令缺失。
    fn windows_shell_missing_command_is_recognized() {
        assert!(remote_command_missing(
            Some(1),
            "CommandNotFoundException: procora was not found"
        ));
        assert!(remote_command_missing(
            Some(1),
            "'procora' is not recognized as an internal or external command"
        ));
        assert!(!remote_command_missing(Some(1), "Permission denied"));
    }
}
