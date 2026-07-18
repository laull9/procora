//! CLI、MCP 与其他本地入口共享的程序化接口。

use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use serde::Serialize;

use crate::{
    config::discover_path,
    daemon::{CenterClient, ServiceHost},
    protocol::{
        CenterHello, CenterRequest, CenterResponse, ConfigCandidateDto, ServiceActionDto,
        ServiceSelectorDto, ServiceStatusRecordDto, ServiceViewDto,
    },
};

use super::center_runtime;

/// 配置完整校验后的稳定摘要。
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ValidationReport {
    /// 服务稳定名称。
    pub project: String,
    /// 被选中的规范化配置路径。
    pub config_path: PathBuf,
    /// 当前活动 profile；空值表示基础配置。
    pub active_profile: Option<String>,
    /// 编译后的 Task 数量。
    pub task_count: usize,
    /// 可用命名模板数量。
    pub template_count: usize,
    /// 项目管理依赖数量。
    pub dependency_count: usize,
}

/// 完整发现并校验配置，但不下载、注册或启动服务。
///
/// # Errors
///
/// 当路径发现、配置解析或任务图编译失败时返回错误。
pub fn validate_project(path: &Path) -> anyhow::Result<ValidationReport> {
    let discovered =
        discover_path(path).with_context(|| format!("配置校验失败: {}", path.display()))?;
    Ok(ValidationReport {
        project: discovered.compiled.spec.project,
        config_path: discovered.config_path,
        active_profile: discovered.compiled.active_profile,
        task_count: discovered.compiled.spec.tasks.len(),
        template_count: discovered.compiled.task_template_names.len(),
        dependency_count: discovered.compiled.dependencies.len(),
    })
}

/// 返回配置中确定性的 Task 启动拓扑顺序。
///
/// # Errors
///
/// 当路径发现、配置解析或任务图编译失败时返回错误。
pub fn task_graph(path: &Path) -> anyhow::Result<Vec<String>> {
    let discovered =
        discover_path(path).with_context(|| format!("任务图编译失败: {}", path.display()))?;
    Ok(ServiceHost::from_compiled(discovered.compiled)
        .start_plan()
        .clone())
}

/// 返回包含默认值、来源与规范化路径的有效配置 JSON。
///
/// # Errors
///
/// 当路径发现、配置解析、任务图编译或 JSON 序列化失败时返回错误。
pub fn effective_config(path: &Path) -> anyhow::Result<serde_json::Value> {
    let discovered =
        discover_path(path).with_context(|| format!("有效配置生成失败: {}", path.display()))?;
    let compiled = &discovered.compiled;
    serde_json::to_value(serde_json::json!({
        "vars": compiled.vars,
        "resolved_vars": compiled.resolved_vars,
        "variable_references": compiled.variable_references,
        "version": compiled.spec.version,
        "project": compiled.spec.project,
        "active_profile": compiled.active_profile,
        "profiles": compiled.profile_names,
        "profile_extends": compiled.profile_extends,
        "dependencies": compiled.dependencies,
        "env": compiled.project_env,
        "task_defaults": compiled.task_defaults,
        "task_templates": compiled.task_template_names,
        "tasks": compiled.spec.tasks,
        "origins": compiled.task_origins,
    }))
    .context("有效配置 JSON 序列化失败")
}

/// 返回中心服务器状态；离线时不隐式启动。
///
/// # Errors
///
/// 当用户数据目录不可用、版本对账或握手失败时返回错误。
pub fn center_status() -> anyhow::Result<Option<CenterHello>> {
    let Some(client) = center_runtime::running_center()? else {
        return Ok(None);
    };
    Ok(Some(client.hello("procora-api")?))
}

/// 返回全部持久托管服务；中心离线时不隐式启动。
///
/// # Errors
///
/// 当中心连接或响应失败时返回错误。
pub fn list_services() -> anyhow::Result<Option<Vec<ServiceViewDto>>> {
    let Some(client) = center_runtime::running_center()? else {
        return Ok(None);
    };
    expect_services(client.request(&CenterRequest::List)?).map(Some)
}

/// 注册并启动一个持久托管服务。
///
/// # Errors
///
/// 当中心启动、项目发现、配置加载或服务启动失败时返回错误。
pub fn add_service(path: PathBuf) -> anyhow::Result<ServiceViewDto> {
    let client = center_runtime::ensure_center()?;
    expect_service(client.request(&CenterRequest::Open { path })?)
}

/// 返回指定服务的状态历史；中心必须已经运行。
///
/// # Errors
///
/// 当中心离线、目标不存在或响应失败时返回错误。
pub fn service_history(target: &str) -> anyhow::Result<Vec<ServiceStatusRecordDto>> {
    let client = running_center_required()?;
    expect_history(client.request(&CenterRequest::History {
        selector: selector(target),
    })?)
}

/// 对指定服务执行生命周期动作。
///
/// # Errors
///
/// 当中心启动、目标定位或生命周期操作失败时返回错误。
pub fn manage_service(action: ServiceActionDto, target: &str) -> anyhow::Result<ServiceViewDto> {
    let client = center_runtime::ensure_center()?;
    expect_service(client.request(&CenterRequest::Manage {
        action,
        selector: selector(target),
    })?)
}

/// 预览指定服务的配置候选，不应用任何变更。
///
/// # Errors
///
/// 当中心离线、目标定位或响应失败时返回错误。
pub fn preview_config(target: &str) -> anyhow::Result<ConfigCandidateDto> {
    let client = running_center_required()?;
    match client.request(&CenterRequest::PreviewConfig {
        selector: selector(target),
    })? {
        CenterResponse::ConfigCandidate(candidate) => Ok(candidate),
        CenterResponse::Error { message } => bail!(message),
        response => unexpected_response(&response),
    }
}

/// 应用先前预览且修订仍精确匹配的配置候选。
///
/// # Errors
///
/// 当中心离线、修订变化、配置无效或应用失败时返回错误。
pub fn apply_config(target: &str, revision: &str) -> anyhow::Result<ServiceViewDto> {
    let client = running_center_required()?;
    expect_service(client.request(&CenterRequest::ApplyConfig {
        selector: selector(target),
        revision: revision.to_owned(),
    })?)
}

/// 停止并移除服务注册，但保留服务目录。
///
/// # Errors
///
/// 当中心启动、目标定位、停止或移除失败时返回错误。
pub fn remove_service(target: &str) -> anyhow::Result<ServiceViewDto> {
    let client = center_runtime::ensure_center()?;
    match client.request(&CenterRequest::Remove {
        selector: selector(target),
    })? {
        CenterResponse::Removed(service) => Ok(service),
        CenterResponse::Error { message } => bail!(message),
        response => unexpected_response(&response),
    }
}

/// 根据用户输入区分稳定名称和文件系统路径。
pub fn selector(target: &str) -> ServiceSelectorDto {
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

/// 返回正在运行的中心客户端，否则给出稳定离线错误。
fn running_center_required() -> anyhow::Result<CenterClient> {
    center_runtime::running_center()?.context("全局 Procora 服务器未运行")
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
