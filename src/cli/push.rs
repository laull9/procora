use std::{
    collections::HashSet,
    env, fs,
    io::{self, IsTerminal, Write},
    path::{Path, PathBuf},
    process::{Command, Output},
};

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};

use crate::{
    config::UploadKind,
    protocol::UploadTargetViewDto,
    transfer,
    tui::{SelectionItem, select_inline, select_path_inline},
};

use super::push_memory::{PushMemory, load_memory, save_memory};

/// 交互引导支持的本机来源选择方式。
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum SourceMethod {
    #[default]
    Input,
    Terminal,
    Native,
}

/// 原生文件选择器要返回的路径类型。
#[derive(Clone, Copy, Debug)]
enum NativePathKind {
    File,
    Directory,
}

/// SSH 引导列表的选择结果。
#[derive(Clone)]
enum SshChoice {
    Target(String),
    Manual,
}

/// 完成 push 缺失参数的交互引导并执行传输。
pub(super) struct PushRequest<'a> {
    pub(super) source: Option<PathBuf>,
    pub(super) target: Option<&'a str>,
    pub(super) package_entry: Option<&'a str>,
    pub(super) package_platform: &'a str,
    pub(super) ssh: Option<String>,
    pub(super) remote_bin: Option<String>,
    pub(super) batch: bool,
    pub(super) restart: bool,
}

/// 完成 push 缺失参数的交互引导并执行传输。
pub(super) fn run(request: PushRequest<'_>) -> anyhow::Result<()> {
    let PushRequest {
        source,
        target,
        package_entry,
        package_platform,
        ssh,
        remote_bin,
        batch,
        restart,
    } = request;
    let complete =
        source.is_some() && (target.is_some() || package_entry.is_some()) && ssh.is_some();
    let interactive =
        io::stdin().is_terminal() && io::stdout().is_terminal() && io::stderr().is_terminal();
    if !complete && !batch && !interactive {
        bail!(
            "push 参数不完整时需要交互终端；脚本请至少指定来源和 SSH，推荐完整使用 `procora push <来源> --target <选择器> --ssh <目标> [--restart] --batch`"
        );
    }

    let mut memory = if !complete && !batch {
        load_memory()
    } else {
        PushMemory::default()
    };
    let (source, source_method) = match source {
        Some(source) => (source, memory.source_method),
        None if batch => bail!("batch 模式必须指定本机上传来源"),
        None => choose_source(&memory)?,
    };
    let source = crate::platform::canonicalize(&source)
        .with_context(|| format!("无法访问本机上传来源 `{}`", source.display()))?;
    let packaged = package_entry
        .map(|entry| super::push_package::materialize(&source, entry, package_platform))
        .transpose()?;
    let upload_source = packaged
        .as_ref()
        .map_or(source.as_path(), |packaged| packaged.source.as_path());
    let package_target = packaged
        .as_ref()
        .map(|packaged| packaged.default_target.as_str());
    let target = target.or(package_target);
    let ssh = match ssh {
        Some(ssh) => Some(ssh),
        None if batch => None,
        None => Some(choose_ssh_target(&memory)?),
    };
    let remote_bin = remote_bin.or_else(|| {
        (!complete && !batch)
            .then(|| memory.remote_bin.clone())
            .flatten()
    });
    let restart = if complete || batch || restart {
        restart
    } else {
        choose_restart(memory.restart)?
    };

    let outcome = transfer::push(
        upload_source,
        target,
        ssh.as_deref(),
        remote_bin.as_deref(),
        batch,
        restart,
        memory.upload_target.as_deref(),
    )?;
    if !batch {
        memory.source_method = source_method;
        memory.source = Some(source);
        memory.ssh_target = Some(outcome.ssh_target);
        memory.remote_bin = Some(outcome.remote_bin);
        memory.upload_target = Some(outcome.target);
        memory.restart = restart;
        if let Err(error) = save_memory(&memory) {
            eprintln!("警告：上传已完成，但无法保存 push 引导记忆：{error:#}");
        }
    }
    Ok(())
}

/// 列出本机或指定 SSH 远端的活动上传项。
pub(super) fn list(
    ssh: Option<&str>,
    remote_bin: Option<&str>,
    batch: bool,
    json: bool,
) -> anyhow::Result<()> {
    let mut targets = if let Some(ssh) = ssh {
        transfer::list_remote(ssh, remote_bin, batch)?
    } else {
        transfer::list_local()?
    };
    targets.sort_by(|left, right| left.selector.cmp(&right.selector));
    if json {
        serde_json::to_writer_pretty(io::stdout(), &targets)?;
        println!();
        return Ok(());
    }
    print_targets(&targets);
    Ok(())
}

/// 选择来源输入方式并获取本机路径。
fn choose_source(memory: &PushMemory) -> anyhow::Result<(PathBuf, SourceMethod)> {
    let methods = [
        (SourceMethod::Input, "直接输入", "粘贴或键入文件/文件夹路径"),
        (
            SourceMethod::Terminal,
            "终端浏览",
            "在小 TUI 中浏览并选择文件或文件夹",
        ),
        (
            SourceMethod::Native,
            "系统选择器",
            "唤起桌面文件或文件夹选择界面",
        ),
    ];
    let mut methods = methods.into_iter().collect::<Vec<_>>();
    if let Some(index) = methods
        .iter()
        .position(|(method, _, _)| *method == memory.source_method)
    {
        methods.swap(0, index);
    }
    let items = methods
        .into_iter()
        .map(|(method, label, description)| SelectionItem::new(label, description, method))
        .collect();
    let method = select_inline(
        "选择本机上传来源",
        "CLI 参数中没有来源，请选择获取文件或文件夹的方式。",
        items,
    )?
    .context("已取消上传来源选择")?;
    let source = match method {
        SourceMethod::Input => prompt_path(memory.source.as_deref())?,
        SourceMethod::Terminal => {
            select_path_inline(memory.source.as_deref())?.context("已取消上传来源选择")?
        }
        SourceMethod::Native => choose_native_path()?,
    };
    Ok((source, method))
}

/// 读取一条本机来源路径。
fn prompt_path(default: Option<&Path>) -> anyhow::Result<PathBuf> {
    let label = default.map_or_else(
        || "本机文件或文件夹：".to_owned(),
        |path| format!("本机文件或文件夹 [{}]：", path.display()),
    );
    let value = prompt_text(&label)?;
    if value.is_empty() {
        return default
            .map(Path::to_path_buf)
            .context("本机上传来源不能为空");
    }
    Ok(crate::platform::simplify_path(Path::new(&value)))
}

/// 从记忆、环境变量与 SSH config 中引导选择连接目标。
fn choose_ssh_target(memory: &PushMemory) -> anyhow::Result<String> {
    let mut candidates = Vec::<(String, String)>::new();
    let mut seen = HashSet::new();
    add_ssh_candidate(
        &mut candidates,
        &mut seen,
        memory.ssh_target.as_deref(),
        "上次使用",
    );
    let environment = env::var("PROCORA_SSH_TARGET").ok();
    add_ssh_candidate(
        &mut candidates,
        &mut seen,
        environment.as_deref(),
        "PROCORA_SSH_TARGET",
    );
    for alias in ssh_config_aliases() {
        add_ssh_candidate(&mut candidates, &mut seen, Some(&alias), "SSH config");
    }
    if candidates.is_empty() {
        return non_empty_prompt("SSH 目标（SSH config 别名或 [user@]host）：");
    }
    let mut items = candidates
        .into_iter()
        .map(|(target, source)| {
            SelectionItem::new(&target, source, SshChoice::Target(target.clone()))
        })
        .collect::<Vec<_>>();
    items.push(SelectionItem::new(
        "手动输入",
        "输入新的 SSH config 别名或 [user@]host",
        SshChoice::Manual,
    ));
    match select_inline(
        "选择 SSH 连接",
        "这里选择服务器；上传目标稍后从该服务器读取。密码仅在本次命令内存中复用，绝不落盘。",
        items,
    )?
    .context("已取消 SSH 目标选择")?
    {
        SshChoice::Target(target) => Ok(target),
        SshChoice::Manual => non_empty_prompt("SSH 目标（SSH config 别名或 [user@]host）："),
    }
}

/// 添加去重且非空的 SSH 候选。
fn add_ssh_candidate(
    candidates: &mut Vec<(String, String)>,
    seen: &mut HashSet<String>,
    value: Option<&str>,
    source: &str,
) {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return;
    };
    if seen.insert(value.to_owned()) {
        candidates.push((value.to_owned(), source.to_owned()));
    }
}

/// 读取用户 SSH config 中不含通配符的 Host 别名。
fn ssh_config_aliases() -> Vec<String> {
    let Some(base) = directories::BaseDirs::new() else {
        return Vec::new();
    };
    let Ok(content) = fs::read_to_string(base.home_dir().join(".ssh/config")) else {
        return Vec::new();
    };
    content
        .lines()
        .map(str::trim)
        .filter(|line| {
            line.get(..4)
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case("host"))
                && line.as_bytes().get(4).is_some_and(u8::is_ascii_whitespace)
        })
        .flat_map(|line| line[4..].split_ascii_whitespace())
        .filter(|alias| {
            !alias.starts_with('!')
                && !alias
                    .bytes()
                    .any(|byte| matches!(byte, b'*' | b'?' | b'[' | b']'))
        })
        .map(str::to_owned)
        .collect()
}

/// 询问是否为本次上传强制开启重启。
fn choose_restart(previous: bool) -> anyhow::Result<bool> {
    let mut choices = vec![
        SelectionItem::new(
            "遵循远端配置",
            "远端目标 restart=true 时重启，否则只覆盖",
            false,
        ),
        SelectionItem::new(
            "本次强制重启",
            "无论远端默认值如何，提交成功后都重启 Service",
            true,
        ),
    ];
    if previous {
        choices.swap(0, 1);
    }
    select_inline(
        "上传后的运行行为",
        "默认遵循远端上传目标配置；此选择会记住，但不会保存任何密码。",
        choices,
    )?
    .context("已取消上传后的运行行为选择")
}

/// 调用桌面环境原生选择器。
fn choose_native_path() -> anyhow::Result<PathBuf> {
    let kind = select_inline(
        "系统选择器类型",
        "请选择要由桌面界面返回的来源类型。",
        vec![
            SelectionItem::new("选择文件", "打开系统文件选择窗口", NativePathKind::File),
            SelectionItem::new(
                "选择文件夹",
                "打开系统文件夹选择窗口",
                NativePathKind::Directory,
            ),
        ],
    )?
    .context("已取消系统选择器类型")?;
    native_path_dialog(kind)?.context("已取消系统文件选择")
}

/// 在当前平台打开原生文件或目录选择窗口。
fn native_path_dialog(kind: NativePathKind) -> anyhow::Result<Option<PathBuf>> {
    #[cfg(target_os = "macos")]
    {
        let script = match kind {
            NativePathKind::File => "POSIX path of (choose file with prompt \"选择上传文件\")",
            NativePathKind::Directory => {
                "POSIX path of (choose folder with prompt \"选择上传文件夹\")"
            }
        };
        return dialog_output(Command::new("osascript").args(["-e", script]).output()?);
    }
    #[cfg(target_os = "linux")]
    {
        let mut command = Command::new("zenity");
        command.args(["--file-selection", "--title=选择 Procora 上传来源"]);
        if matches!(kind, NativePathKind::Directory) {
            command.arg("--directory");
        }
        match command.output() {
            Ok(output) => return dialog_output(output),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).context("无法启动系统文件选择器 `zenity`"),
        }
        let argument = match kind {
            NativePathKind::File => "--getopenfilename",
            NativePathKind::Directory => "--getexistingdirectory",
        };
        return Command::new("kdialog")
            .arg(argument)
            .arg(".")
            .output()
            .context("无法启动系统文件选择器 `zenity` 或 `kdialog`；可改用终端浏览或直接输入")
            .and_then(dialog_output);
    }
    #[cfg(target_os = "windows")]
    {
        let script = match kind {
            NativePathKind::File => {
                "Add-Type -AssemblyName System.Windows.Forms; $d=New-Object System.Windows.Forms.OpenFileDialog; if($d.ShowDialog() -eq 'OK'){[Console]::Write($d.FileName)}"
            }
            NativePathKind::Directory => {
                "Add-Type -AssemblyName System.Windows.Forms; $d=New-Object System.Windows.Forms.FolderBrowserDialog; if($d.ShowDialog() -eq 'OK'){[Console]::Write($d.SelectedPath)}"
            }
        };
        let mut command = Command::new("powershell");
        command.args(["-NoProfile", "-STA", "-Command", script]);
        crate::process::configure_background_command(&mut command);
        return dialog_output(command.output()?);
    }
    #[allow(unreachable_code)]
    {
        let _ = kind;
        bail!("当前平台没有可用的系统文件选择器；请改用终端浏览或直接输入")
    }
}

/// 解析原生选择器的退出状态与路径输出。
fn dialog_output(output: Output) -> anyhow::Result<Option<PathBuf>> {
    let Output {
        status,
        stdout,
        stderr,
    } = output;
    if !status.success() {
        if matches!(status.code(), Some(1 | 130)) {
            return Ok(None);
        }
        let message = crate::platform::decode_external_output(&stderr)
            .trim()
            .to_owned();
        bail!("系统文件选择器失败：{message}");
    }
    let value = crate::platform::decode_external_output(&stdout)
        .trim()
        .to_owned();
    Ok((!value.is_empty()).then(|| crate::platform::simplify_path(Path::new(&value))))
}

/// 从标准输入读取一行文本。
fn prompt_text(label: &str) -> anyhow::Result<String> {
    eprint!("{label}");
    io::stderr().flush()?;
    let mut value = String::new();
    if io::stdin().read_line(&mut value)? == 0 {
        bail!("输入已结束")
    }
    Ok(value.trim().to_owned())
}

/// 读取一条必填文本。
fn non_empty_prompt(label: &str) -> anyhow::Result<String> {
    let value = prompt_text(label)?;
    if value.is_empty() {
        bail!("输入不能为空");
    }
    Ok(value)
}

/// 输出活动上传项表格。
fn print_targets(targets: &[UploadTargetViewDto]) {
    if targets.is_empty() {
        println!("没有活动上传项");
        return;
    }
    println!("选择器\t类型\t上限\t自动重启\tService 内路径");
    for target in targets {
        println!(
            "{}\t{}\t{}\t{}\t{}",
            target.selector,
            kind_label(target.kind),
            human_bytes(target.max_bytes),
            if target.restart { "是" } else { "否" },
            target.path.display()
        );
    }
}

/// 返回上传目标类型中文名称。
const fn kind_label(kind: UploadKind) -> &'static str {
    match kind {
        UploadKind::File => "文件",
        UploadKind::Directory => "目录",
    }
}

/// 以紧凑二进制单位展示字节数。
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

/// 不经过浮点转换保留一位二进制单位小数。
fn format_unit(bytes: u64, unit: u64, label: &str) -> String {
    let whole = bytes / unit;
    let decimal = (bytes % unit).saturating_mul(10) / unit;
    format!("{whole}.{decimal} {label}")
}
