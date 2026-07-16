use std::{
    collections::hash_map::DefaultHasher,
    env, fs,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    process::{Command as ProcessCommand, Stdio},
    thread,
    time::Duration,
};

use anyhow::{Context, bail};
use directories::ProjectDirs;
use procora_daemon::{CenterClient, ServiceHost, run_center_server};
use procora_platform::{
    autostart::{self, DaemonAutostart},
    capabilities,
};
use procora_protocol::{
    CenterHello, CenterRequest, CenterResponse, ServiceActionDto, ServiceSelectorDto,
    ServiceStatusDto, ServiceStatusRecordDto, ServiceViewDto, SnapshotSourceDto,
};

use crate::{Command, ServerArgs, ServerCommand, project, session, suggestion, template};

/// 当前用户全局 Procora 服务器使用的 IPC 与持久化位置。
#[derive(Clone, Debug)]
struct CenterPaths {
    endpoint: String,
    database: PathBuf,
}

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
        Some(Command::Up) => up(),
        Some(Command::Down) => down(),
        Some(Command::Status) => status(),
        Some(Command::Enable) => enable_autostart(),
        Some(Command::Disable) => disable_autostart(),
        Some(Command::Server(arguments)) => server(arguments),
        Some(Command::Show { target }) => show(&target),
        Some(Command::Validate { path }) => project::validate(&path),
        Some(Command::Graph { path }) => project::graph(&path),
        Some(Command::Doctor) => {
            doctor();
            Ok(())
        }
        Some(Command::Daemon { endpoint, database }) => {
            run_center_server(&endpoint, &database).context("全局 Procora 服务器退出")
        }
    }
}

/// 在指定路径连接已有服务，或创建与 TUI 同生命周期的临时宿主。
fn open_tui(target: Option<&Path>) -> anyhow::Result<()> {
    let target = target.map_or_else(
        || env::current_dir().context("无法读取当前目录"),
        |path| Ok(path.to_path_buf()),
    )?;
    let paths = center_paths()?;
    let client = CenterClient::new(paths.endpoint);
    if client.ping() {
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

    let mut discovered = procora_config::discover_path(&target)
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
            let client = ensure_center()?;
            let service = expect_service(client.request(&CenterRequest::Open { path })?)?;
            print_service(&service);
            Ok(())
        }
        (None, Some(ServerCommand::List)) => {
            let Some(client) = running_center()? else {
                print_global_offline();
                return Ok(());
            };
            let services = expect_services(client.request(&CenterRequest::List)?)?;
            print_services(&services);
            Ok(())
        }
        (None, Some(ServerCommand::History { target })) => history(&target),
        (None, Some(ServerCommand::Start { target })) => manage(ServiceActionDto::Start, &target),
        (None, Some(ServerCommand::Restart { target })) => {
            manage(ServiceActionDto::Restart, &target)
        }
        (None, Some(ServerCommand::Stop { target })) => manage(ServiceActionDto::Stop, &target),
        (None, None) => {
            bail!("`procora server` 需要 PATH 或 list/history/start/restart/stop 子命令")
        }
        (Some(_), Some(_)) => bail!("服务路径和管理子命令不能同时使用"),
    }
}

/// 启动全局 Procora 服务器并输出状态。
fn up() -> anyhow::Result<()> {
    let client = ensure_center()?;
    let hello = client.hello("procora-cli")?;
    print_center_status(&hello);
    Ok(())
}

/// 请求全局 Procora 服务器正常退出并等待端点关闭。
fn down() -> anyhow::Result<()> {
    let paths = center_paths()?;
    let client = CenterClient::new(paths.endpoint);
    if !client.ping() {
        print_global_offline();
        return Ok(());
    }
    shutdown_center(&client)?;
    println!("全局 Procora：已停止");
    Ok(())
}

/// 请求全局 Procora 服务器正常退出并等待端点关闭。
fn shutdown_center(client: &CenterClient) -> anyhow::Result<()> {
    match client.request(&CenterRequest::Shutdown)? {
        CenterResponse::ShuttingDown => {}
        CenterResponse::Error { message } => bail!(message),
        response => return unexpected_response(&response),
    }
    for _ in 0..100 {
        if !client.ping() {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(20));
    }
    bail!("全局 Procora 服务器未在 2 秒内退出")
}

/// 把中心 daemon 注册到当前平台的用户级自启动托管器。
fn enable_autostart() -> anyhow::Result<()> {
    let paths = center_paths()?;
    let client = CenterClient::new(paths.endpoint.clone());
    if client.ping() {
        shutdown_center(&client).context("无法把现有全局服务器移交给系统托管")?;
    }
    if let Some(parent) = paths.database.parent() {
        fs::create_dir_all(parent).context("无法创建 Procora 状态目录")?;
    }
    let executable = env::current_exe().context("无法定位 procora 可执行文件")?;
    let definition = DaemonAutostart::new(executable, &paths.endpoint, &paths.database);
    let backend = definition.enable().context("注册开机自启动失败")?;

    for _ in 0..250 {
        if client.ping() {
            client.hello("procora-cli")?;
            println!("已启用开机自启动：{}", backend.label());
            return Ok(());
        }
        thread::sleep(Duration::from_millis(20));
    }
    bail!(
        "{} 已注册，但全局 Procora 服务器未在 5 秒内就绪",
        backend.label()
    )
}

/// 正常停止中心 daemon 并移除用户级自启动注册。
fn disable_autostart() -> anyhow::Result<()> {
    let paths = center_paths()?;
    let client = CenterClient::new(paths.endpoint);
    if client.ping() {
        shutdown_center(&client).context("停止自启动全局服务器失败")?;
    }
    let backend = autostart::disable().context("移除开机自启动失败")?;
    println!("已禁用开机自启动：{}", backend.label());
    Ok(())
}

/// 查询全局 Procora 服务器状态但不隐式启动后台进程。
fn status() -> anyhow::Result<()> {
    let Some(client) = running_center()? else {
        print_global_offline();
        return Ok(());
    };
    print_center_status(&client.hello("procora-cli")?);
    Ok(())
}

/// 查询并输出指定服务的状态历史。
fn history(target: &str) -> anyhow::Result<()> {
    let client = running_center()?.context("全局 Procora 服务器未运行")?;
    let records = expect_history(client.request(&CenterRequest::History {
        selector: selector(target),
    })?)?;
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
    let client = ensure_center()?;
    let hello = client.hello("procora-tui")?;
    let selector = selector(target);
    let snapshot = expect_snapshot(client.request(&CenterRequest::Snapshot {
        selector: selector.clone(),
    })?)?;
    session::run_center_tui(
        client,
        selector,
        snapshot,
        hello.event_sequence,
        hello.control_allowed,
    )?;
    Ok(())
}

/// 对指定服务执行生命周期动作并输出结果。
fn manage(action: ServiceActionDto, target: &str) -> anyhow::Result<()> {
    let client = ensure_center()?;
    let service = expect_service(client.request(&CenterRequest::Manage {
        action,
        selector: selector(target),
    })?)?;
    print_service(&service);
    Ok(())
}

/// 输出当前平台的基础运行能力。
fn doctor() {
    let capabilities = capabilities();
    println!("平台: {:?}", capabilities.platform);
    println!("受管进程树: {}", capabilities.managed_process_tree);
    println!("systemd: {}", capabilities.systemd);
}

/// 只连接正在运行的全局 Procora 服务器。
fn running_center() -> anyhow::Result<Option<CenterClient>> {
    let paths = center_paths()?;
    let client = CenterClient::new(paths.endpoint);
    Ok(client.ping().then_some(client))
}

/// 连接全局 Procora 服务器，不存在时启动独立后台进程并等待就绪。
fn ensure_center() -> anyhow::Result<CenterClient> {
    let paths = center_paths()?;
    let client = CenterClient::new(paths.endpoint.clone());
    if client.ping() {
        client.hello("procora-cli")?;
        return Ok(client);
    }
    if let Some(parent) = paths.database.parent() {
        fs::create_dir_all(parent).context("无法创建 Procora 状态目录")?;
    }
    let executable = env::current_exe().context("无法定位 procora 可执行文件")?;
    spawn_center_process(&executable, &paths).context("无法启动全局 Procora 服务器")?;

    for _ in 0..100 {
        if client.ping() {
            client.hello("procora-cli")?;
            return Ok(client);
        }
        thread::sleep(Duration::from_millis(20));
    }
    bail!("全局 Procora 服务器未在 2 秒内就绪")
}

/// 启动与当前终端会话分离的中心服务器子进程。
fn spawn_center_process(executable: &Path, paths: &CenterPaths) -> std::io::Result<()> {
    let mut command = ProcessCommand::new(executable);
    command
        .arg("__daemon")
        .arg("--endpoint")
        .arg(&paths.endpoint)
        .arg("--database")
        .arg(&paths.database)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    #[cfg(unix)]
    {
        use process_wrap::std::{CommandWrap, ProcessSession};

        let mut command = CommandWrap::from(command);
        command.wrap(ProcessSession);
        command.spawn()?;
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;

        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        command
            .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP)
            .spawn()?;
    }
    Ok(())
}

/// 计算当前用户独立的端点名称和注册表路径。
fn center_paths() -> anyhow::Result<CenterPaths> {
    let home = if let Some(path) = env::var_os("PROCORA_HOME") {
        PathBuf::from(path)
    } else {
        ProjectDirs::from("dev", "procora", "Procora")
            .context("当前平台没有可用的用户数据目录")?
            .data_local_dir()
            .to_path_buf()
    };
    let mut hasher = DefaultHasher::new();
    home.hash(&mut hasher);
    Ok(CenterPaths {
        endpoint: format!("procora-center-{:016x}", hasher.finish()),
        database: home.join("procora.sqlite3"),
    })
}

/// 根据用户输入区分稳定名称和文件系统路径。
fn selector(target: &str) -> ServiceSelectorDto {
    let path = Path::new(target);
    if path.exists()
        || path.is_absolute()
        || target == "."
        || target == ".."
        || target.contains('/')
        || target.contains('\\')
    {
        ServiceSelectorDto::Path(path.to_path_buf())
    } else {
        ServiceSelectorDto::Name(target.to_owned())
    }
}

/// 从响应中提取单个服务或转成命令错误。
fn expect_service(response: CenterResponse) -> anyhow::Result<ServiceViewDto> {
    match response {
        CenterResponse::Service(service) => Ok(service),
        CenterResponse::Error { message } => bail!(message),
        response => unexpected_response(&response),
    }
}

/// 从响应中提取服务列表或转成命令错误。
fn expect_services(response: CenterResponse) -> anyhow::Result<Vec<ServiceViewDto>> {
    match response {
        CenterResponse::Services(services) => Ok(services),
        CenterResponse::Error { message } => bail!(message),
        response => unexpected_response(&response),
    }
}

/// 从响应中提取项目快照或转成命令错误。
fn expect_snapshot(response: CenterResponse) -> anyhow::Result<procora_protocol::ProjectSnapshot> {
    match response {
        CenterResponse::Snapshot(snapshot) => Ok(snapshot),
        CenterResponse::Error { message } => bail!(message),
        response => unexpected_response(&response),
    }
}

/// 从响应中提取服务状态历史或转成命令错误。
fn expect_history(response: CenterResponse) -> anyhow::Result<Vec<ServiceStatusRecordDto>> {
    match response {
        CenterResponse::History(records) => Ok(records),
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
