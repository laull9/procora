use std::{env, path::Path};

use crate::daemon::{CenterClient, ServiceHost, run_center_server};
use crate::platform::capabilities;
use crate::protocol::{
    CenterHello, CenterRequest, CenterResponse, ConfigCandidateDto, ServiceActionDto,
    ServiceSelectorDto, ServiceStatusDto, ServiceViewDto, SnapshotSourceDto,
};
use anyhow::{Context, bail};
use clap::CommandFactory;
use clap_complete::generate;

use super::{
    Cli, Command, ServerArgs, ServerCommand, api, autostart_command, center_runtime, project,
    session, source, suggestion, template,
};

/// 分发默认路径行为和全部顶层命令。
pub fn dispatch(command: Option<Command>, target: Option<&Path>) -> anyhow::Result<()> {
    match command {
        None => open_tui(target),
        Some(Command::Init {
            config,
            force,
            no_edit,
        }) => {
            let current = env::current_dir().context("无法读取当前目录")?;
            let path = template::initialize(&current, config, force)?;
            project::edit_after_init(&path, no_edit)
        }
        Some(Command::Edit { path }) => project::edit(path.as_deref()),
        Some(Command::Deps { path, check }) => project::dependencies(&path, check),
        Some(Command::Clean { path }) => project::clean(path.as_deref()),
        Some(Command::Up) => up(),
        Some(Command::Down) => down(),
        Some(Command::Status) => status(),
        Some(Command::Enable) => autostart_command::enable(),
        Some(Command::Disable) => autostart_command::disable(),
        #[cfg(target_os = "windows")]
        Some(Command::ElevatedAutostart { action, result }) => {
            autostart_command::complete_elevated(&action, &result)
        }
        Some(Command::Add { path }) => add(path),
        Some(Command::List) => list(),
        Some(Command::History { target }) => history(&target),
        Some(Command::Start { target }) => manage(ServiceActionDto::Start, &target),
        Some(Command::Restart { target }) => manage(ServiceActionDto::Restart, &target),
        Some(Command::Preview { target }) => preview_config(&target),
        Some(Command::Apply { target, revision }) => apply_config(&target, &revision),
        Some(Command::Stop { target }) => manage(ServiceActionDto::Stop, &target),
        Some(Command::Remove { target }) => remove(&target),
        Some(Command::Server(arguments)) => server(arguments),
        Some(Command::Source(arguments)) => source::run(arguments.command),
        Some(Command::Show { target }) => show(&target),
        Some(Command::Validate { path }) => project::validate(&path),
        Some(Command::Graph { path }) => project::graph(&path),
        Some(Command::Config { path }) => project::effective_config(&path),
        Some(Command::Doctor) => {
            doctor();
            Ok(())
        }
        Some(Command::Completions { shell }) => {
            completions(shell);
            Ok(())
        }
        Some(Command::Mcp) => crate::mcp::run_stdio(),
        Some(Command::Daemon { endpoint, database }) => {
            run_center_server(&endpoint, &database).context("全局 Procora 服务器退出")
        }
    }
}

/// 把目标 shell 的补全脚本写到标准输出。
fn completions(shell: clap_complete::Shell) {
    let mut command = Cli::command();
    generate(shell, &mut command, "procora", &mut std::io::stdout());
}

/// 在指定路径连接已有服务，或创建与 TUI 同生命周期的临时宿主。
fn open_tui(target: Option<&Path>) -> anyhow::Result<()> {
    let target = target.map_or_else(
        || env::current_dir().context("无法读取当前目录"),
        |path| Ok(path.to_path_buf()),
    )?;
    project::warn_python_execution(&target);
    if let Some(client) = center_runtime::running_center()? {
        let hello = client.hello("procora-tui")?;
        let mut selector = ServiceSelectorDto::Path(target.clone());
        let snapshot = match client.request(&CenterRequest::Snapshot {
            selector: selector.clone(),
        })? {
            CenterResponse::Snapshot(snapshot) => snapshot,
            CenterResponse::Error { .. } => {
                let service =
                    expect_service(client.request(&CenterRequest::Open { path: target })?)?;
                selector = ServiceSelectorDto::Name(service.name);
                expect_snapshot(client.request(&CenterRequest::Snapshot {
                    selector: selector.clone(),
                })?)?
            }
            response => unexpected_response(&response)?,
        };
        session::run_center_tui(
            client,
            selector,
            snapshot,
            hello.event_sequence,
            hello.control_allowed,
        )?;
        return Ok(());
    }

    let mut discovered = crate::config::discover_path(&target)
        .with_context(|| format!("无法打开 Procora 服务：{}", target.display()))?;
    project::prepare(&mut discovered)?;
    let mut host = ServiceHost::from_compiled_at(discovered.compiled, &discovered.root);
    host.start().context("嵌入式服务 Task 启动失败")?;
    let snapshot = host.snapshot(SnapshotSourceDto::EmbeddedLive, true);
    let result = session::run_embedded_tui(&mut host, snapshot);
    host.stop().context("嵌入式服务 Task 停止失败")?;
    result?;
    Ok(())
}

/// 执行服务注册、列表和生命周期管理命令。
fn server(arguments: ServerArgs) -> anyhow::Result<()> {
    match (arguments.path, arguments.command) {
        (Some(path), None) => {
            if let Some(suggestion) = suggestion::for_missing_path(&path, suggestion::SERVER) {
                bail!(
                    "未知 server 子命令 `{}`；是否要运行 `procora server {suggestion}`？",
                    path.display()
                );
            }
            add(path)
        }
        (None, Some(ServerCommand::List)) => list(),
        (None, Some(ServerCommand::History { target })) => history(&target),
        (None, Some(ServerCommand::Start { target })) => manage(ServiceActionDto::Start, &target),
        (None, Some(ServerCommand::Restart { target })) => {
            manage(ServiceActionDto::Restart, &target)
        }
        (None, Some(ServerCommand::Preview { target })) => preview_config(&target),
        (None, Some(ServerCommand::Apply { target, revision })) => apply_config(&target, &revision),
        (None, Some(ServerCommand::Stop { target })) => manage(ServiceActionDto::Stop, &target),
        (None, Some(ServerCommand::Remove { target })) => remove(&target),
        (None, None) => {
            bail!(
                "`procora server` 需要 PATH 或 list/history/start/restart/preview/apply/stop/remove 子命令"
            )
        }
        (Some(_), Some(_)) => bail!("服务路径和管理子命令不能同时使用"),
    }
}

/// 注册并启动一个持久托管服务。
fn add(path: std::path::PathBuf) -> anyhow::Result<()> {
    project::warn_python_execution(&path);
    let service = api::add_service(path)?;
    print_service(&service);
    Ok(())
}

/// 列出全部持久托管服务且不隐式启动全局服务器。
fn list() -> anyhow::Result<()> {
    let Some(services) = api::list_services()? else {
        print_global_offline();
        return Ok(());
    };
    print_services(&services);
    Ok(())
}

/// 启动全局 Procora 服务器并输出状态。
fn up() -> anyhow::Result<()> {
    let client = center_runtime::ensure_center()?;
    let hello = client.hello("procora-cli")?;
    print_center_status(&hello);
    Ok(())
}

/// 请求全局 Procora 服务器正常退出并等待端点关闭。
fn down() -> anyhow::Result<()> {
    let paths = center_runtime::center_paths()?;
    let client = CenterClient::new(paths.endpoint);
    if !client.ping() {
        print_global_offline();
        return Ok(());
    }
    center_runtime::shutdown_center(&client)?;
    println!("全局 Procora：已停止");
    Ok(())
}

/// 查询全局 Procora 服务器状态但不隐式启动后台进程。
fn status() -> anyhow::Result<()> {
    let Some(hello) = api::center_status()? else {
        print_global_offline();
        return Ok(());
    };
    print_center_status(&hello);
    Ok(())
}

/// 查询并输出指定服务的状态历史。
fn history(target: &str) -> anyhow::Result<()> {
    let records = api::service_history(target)?;
    println!("时间戳(ms)\t状态\t说明");
    for record in records {
        println!(
            "{}\t{}\t{}",
            record.recorded_at_ms,
            status_label(record.status),
            record.message.as_deref().unwrap_or("-")
        );
    }
    Ok(())
}

/// 打开指定服务的 TUI。
fn show(target: &str) -> anyhow::Result<()> {
    let client = center_runtime::ensure_center()?;
    let hello = client.hello("procora-tui")?;
    let mut selector = api::selector(target);
    let snapshot = match client.request(&CenterRequest::Snapshot {
        selector: selector.clone(),
    })? {
        CenterResponse::Snapshot(snapshot) => snapshot,
        CenterResponse::Error { message } => {
            let recovery_path = match &selector {
                ServiceSelectorDto::Path(path) => Some(path.clone()),
                ServiceSelectorDto::Name(name) => current_service_path(name),
            };
            let Some(path) = recovery_path else {
                bail!(message);
            };
            let service = expect_service(client.request(&CenterRequest::Open { path })?)?;
            selector = ServiceSelectorDto::Name(service.name);
            expect_snapshot(client.request(&CenterRequest::Snapshot {
                selector: selector.clone(),
            })?)?
        }
        response => unexpected_response(&response)?,
    };
    session::run_center_tui(
        client,
        selector,
        snapshot,
        hello.event_sequence,
        hello.control_allowed,
    )?;
    Ok(())
}

/// 仅当当前目录项目与名称选择器一致时返回可安全自愈的服务根目录。
fn current_service_path(name: &str) -> Option<std::path::PathBuf> {
    let current = env::current_dir().ok()?;
    let discovered = crate::config::discover_path(current).ok()?;
    (discovered.compiled.spec.project == name).then_some(discovered.root)
}

/// 对指定服务执行生命周期动作并输出结果。
fn manage(action: ServiceActionDto, target: &str) -> anyhow::Result<()> {
    let service = api::manage_service(action, target)?;
    print_service(&service);
    Ok(())
}

/// 预览配置候选并输出稳定修订与确定性 Task 影响集合。
fn preview_config(target: &str) -> anyhow::Result<()> {
    let candidate = api::preview_config(target)?;
    print_candidate(&candidate);
    if !candidate.valid {
        bail!("候选配置无效，当前有效修订保持不变");
    }
    Ok(())
}

/// 应用用户已经预览的精确配置修订。
fn apply_config(target: &str, revision: &str) -> anyhow::Result<()> {
    let service = api::apply_config(target, revision)?;
    print_service(&service);
    Ok(())
}

/// 停止并从中心注册表删除指定服务，但保留用户服务目录。
fn remove(target: &str) -> anyhow::Result<()> {
    let service = api::remove_service(target)?;
    println!("已删除服务：{}\t{}", service.name, service.root.display());
    Ok(())
}

/// 输出当前平台的基础运行能力。
fn doctor() {
    let capabilities = capabilities();
    println!("平台: {:?}", capabilities.platform);
    println!("受管进程树: {}", capabilities.managed_process_tree);
    println!("systemd: {}", capabilities.systemd);
}

/// 从响应中提取单个服务或转成命令错误。
fn expect_service(response: CenterResponse) -> anyhow::Result<ServiceViewDto> {
    match response {
        CenterResponse::Service(service) => Ok(service),
        CenterResponse::Error { message } => bail!(message),
        response => unexpected_response(&response),
    }
}

/// 从响应中提取项目快照或转成命令错误。
fn expect_snapshot(response: CenterResponse) -> anyhow::Result<crate::protocol::ProjectSnapshot> {
    match response {
        CenterResponse::Snapshot(snapshot) => Ok(snapshot),
        CenterResponse::Error { message } => bail!(message),
        response => unexpected_response(&response),
    }
}

/// 把不符合请求类型的响应转换为协议错误。
fn unexpected_response<T>(response: &CenterResponse) -> anyhow::Result<T> {
    bail!("全局 Procora 服务器返回了意外响应: {response:?}")
}

/// 输出单个服务的稳定人类可读摘要。
fn print_service(service: &ServiceViewDto) {
    println!(
        "{}\t{}\t{}\t{} 个任务",
        service.name,
        status_label(service.status),
        service.root.display(),
        service.task_count
    );
    if let Some(message) = &service.message {
        println!("  {message}");
    }
}

/// 输出配置候选及按类别排序的 Task 影响。
fn print_candidate(candidate: &ConfigCandidateDto) {
    println!(
        "修订：{}",
        candidate.revision.as_deref().unwrap_or("<unreadable>")
    );
    println!("有效：{}", if candidate.valid { "是" } else { "否" });
    if let Some(diff) = &candidate.diff {
        println!("新增：{}", task_ids(&diff.added));
        println!("删除：{}", task_ids(&diff.removed));
        println!("重启：{}", task_ids(&diff.restart));
        println!("原地更新：{}", task_ids(&diff.update_in_place));
        println!("无影响：{}", task_ids(&diff.unchanged));
    }
    if let Some(message) = &candidate.message {
        println!("说明：{message}");
    }
}

/// 把确定性 Task 标识集合格式化为紧凑终端文本。
fn task_ids(task_ids: &[crate::core::TaskId]) -> String {
    if task_ids.is_empty() {
        "-".to_owned()
    } else {
        task_ids
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(",")
    }
}

/// 输出适合终端扫描的服务列表。
fn print_services(services: &[ServiceViewDto]) {
    println!("名称\t状态\t任务\t服务目录\t配置文件");
    for service in services {
        println!(
            "{}\t{}\t{}\t{}\t{}",
            service.name,
            status_label(service.status),
            service.task_count,
            service.root.display(),
            service.config_path.display()
        );
    }
}

/// 输出全局 Procora 服务器状态与当前注册数量。
fn print_center_status(hello: &CenterHello) {
    println!("全局 Procora：运行中");
    println!("版本：{}", hello.procora_version);
    println!("服务：{}", hello.service_count);
}

/// 输出全局 Procora 服务器未运行的稳定状态。
fn print_global_offline() {
    println!("全局 Procora：未运行");
}

/// 返回服务状态的中文命令行标签。
const fn status_label(status: ServiceStatusDto) -> &'static str {
    match status {
        ServiceStatusDto::Running => "运行中",
        ServiceStatusDto::Stopped => "已停止",
        ServiceStatusDto::Failed => "失败",
    }
}
