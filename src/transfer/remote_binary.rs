use std::{
    io::{self, IsTerminal, Write},
    process::Stdio,
};

use anyhow::{Context, bail};
use serde::Deserialize;

use super::remote::{base_ssh, validate_remote_bin};

/// Unix 远端一次性扫描的常见 Procora 安装位置。
const UNIX_DISCOVERY: &str = r#"for p in "$HOME/.local/bin/procora" "$HOME/bin/procora" /usr/local/bin/procora /opt/homebrew/bin/procora /usr/bin/procora; do if [ -x "$p" ] && "$p" __ssh-probe >/dev/null 2>&1; then printf '__PROCORA_PATH__%s\n' "$p"; exit 0; fi; done; exit 127"#;

/// Windows SSH 环境中无需插入不可信内容即可尝试的常见路径。
const WINDOWS_CANDIDATES: &[&str] = &[
    "procora.exe",
    "~/.local/bin/procora.exe",
    "~/AppData/Local/Procora/bin/procora.exe",
];

/// 远端能力握手的最小解析结构。
#[derive(Deserialize)]
struct Probe {
    name: String,
}

/// PATH 失败后智能查找远端 Procora，并提供人工路径回退。
pub(super) fn resolve_after_missing(
    ssh_target: &str,
    configured: Option<&str>,
    batch: bool,
    interactive_login: bool,
    original_error: anyhow::Error,
) -> anyhow::Result<String> {
    if let Some(path) = discover_unix(ssh_target, interactive_login)? {
        eprintln!("已自动找到远端 Procora：{path}");
        return Ok(path);
    }
    for candidate in WINDOWS_CANDIDATES {
        if Some(*candidate) == configured {
            continue;
        }
        if probe_candidate(ssh_target, candidate, interactive_login)? {
            eprintln!("已自动找到远端 Procora：{candidate}");
            return Ok((*candidate).to_owned());
        }
    }
    if batch {
        return Err(original_error.context(
            "已检查远端 PATH 与常见安装位置；batch 模式无法询问路径，请指定 `--remote-bin <PATH>`",
        ));
    }
    if !io::stdin().is_terminal() || !io::stderr().is_terminal() {
        return Err(original_error.context(
            "已检查远端 PATH 与常见安装位置；当前不是交互终端，请指定 `--remote-bin <PATH>`",
        ));
    }
    eprintln!("未在远端 PATH 或常见安装位置找到 Procora。");
    loop {
        eprint!("远端 Procora 路径：");
        io::stderr().flush()?;
        let mut value = String::new();
        if io::stdin().read_line(&mut value)? == 0 {
            bail!("输入已结束");
        }
        let value = value.trim();
        if let Err(error) = validate_remote_bin(value) {
            eprintln!("{error}");
            continue;
        }
        return Ok(value.to_owned());
    }
}

/// 在 Unix/macOS 常见目录中通过一次 SSH 会话查找可握手的 Procora。
fn discover_unix(ssh_target: &str, interactive_login: bool) -> anyhow::Result<Option<String>> {
    let mut command = base_ssh(interactive_login);
    let output = command
        .arg(ssh_target)
        .arg(UNIX_DISCOVERY)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .context("无法启动本机 ssh；请先安装 OpenSSH 客户端")?;
    if output.status.success() {
        let stdout = String::from_utf8(output.stdout).context("远端路径不是 UTF-8")?;
        let path = stdout
            .lines()
            .find_map(|line| line.strip_prefix("__PROCORA_PATH__"))
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .context("远端路径探测成功但没有返回 Procora 路径")?;
        validate_remote_bin(path)?;
        return Ok(Some(path.to_owned()));
    }
    if output.status.code() == Some(255) {
        let detail = String::from_utf8_lossy(&output.stderr);
        bail!("远端 Procora 路径探测的 SSH 登录失败：{}", detail.trim());
    }
    Ok(None)
}

/// 调用候选可执行文件的能力握手，避免把同名无关程序误判为 Procora。
fn probe_candidate(
    ssh_target: &str,
    candidate: &str,
    interactive_login: bool,
) -> anyhow::Result<bool> {
    let mut command = base_ssh(interactive_login);
    let output = command
        .arg(ssh_target)
        .args([candidate, "__ssh-probe"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .context("无法启动本机 ssh；请先安装 OpenSSH 客户端")?;
    if !output.status.success() {
        if output.status.code() == Some(255) {
            let detail = String::from_utf8_lossy(&output.stderr);
            bail!("远端 Procora 路径探测的 SSH 登录失败：{}", detail.trim());
        }
        return Ok(false);
    }
    if output.stdout.len() > 64 * 1024 {
        bail!("远端 Procora 能力探测响应超过 64 KiB");
    }
    let probe: Probe = match serde_json::from_slice(&output.stdout) {
        Ok(probe) => probe,
        Err(_) => return Ok(false),
    };
    Ok(probe.name == "procora-ssh")
}
