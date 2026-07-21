use std::path::PathBuf;

use crate::protocol::{
    ConfigCandidateDto, ProjectSnapshot, ResourceUsageDto, ServiceStatusDto, ServiceViewDto,
    SnapshotSourceDto,
};
use crate::storage::{StoredService, StoredServiceStatus};
use crate::{
    config::{CompiledProject, ManagedDependencies, ProjectDiff, discover_path},
    core::ProjectSpec,
};

use super::{ServiceHost, status::stored_status};

/// 已编译但尚未产生依赖下载或进程副作用的配置候选。
#[derive(Debug)]
pub(crate) struct PendingConfig {
    pub(crate) revision: String,
    pub(crate) compiled: CompiledProject,
    pub(crate) diff: ProjectDiff,
}

/// 当前宿主对应、尚未替换管理依赖占位符的有效定义。
#[derive(Clone, Debug)]
pub(crate) struct ActiveDefinition {
    pub(crate) spec: ProjectSpec,
    pub(crate) dependencies: ManagedDependencies,
}

impl ActiveDefinition {
    /// 从准备副作用发生前的已编译配置捕获语义输入。
    pub(crate) fn from_compiled(compiled: &CompiledProject) -> Self {
        Self {
            spec: compiled.spec.clone(),
            dependencies: compiled.dependencies.clone(),
        }
    }
}

/// 中心服务器内存中的单个托管服务。
#[derive(Debug)]
pub(crate) struct ManagedService {
    pub(crate) name: String,
    pub(crate) root: PathBuf,
    pub(crate) config_path: PathBuf,
    pub(crate) status: ServiceStatusDto,
    pub(crate) host: Option<ServiceHost>,
    pub(crate) message: Option<String>,
    pub(crate) desired_running: bool,
    pub(crate) pending_config: Option<PendingConfig>,
    pub(crate) candidate_view: Option<ConfigCandidateDto>,
    pub(crate) active_definition: Option<ActiveDefinition>,
}

impl ManagedService {
    /// 返回跨进程展示使用的服务摘要。
    pub(crate) fn view(&self) -> ServiceViewDto {
        ServiceViewDto {
            name: self.name.clone(),
            root: self.root.clone(),
            config_path: self.config_path.clone(),
            status: self.status,
            task_count: self.host.as_ref().map_or(0, |host| host.start_plan().len()),
            resources: None,
            message: self.message.clone(),
        }
    }

    /// 返回包含当前进程树聚合资源的服务摘要。
    pub(crate) fn view_with_resources(&mut self) -> ServiceViewDto {
        let running = self.status == ServiceStatusDto::Running;
        let resources = self.host.as_mut().and_then(|host| {
            let snapshot = host.snapshot(SnapshotSourceDto::CenterLive, running);
            aggregate_resources(&snapshot)
        });
        let mut view = self.view();
        view.resources = resources;
        view
    }

    /// 返回用于注册表恢复的最小持久化信息。
    pub(crate) fn stored(&self) -> StoredService {
        StoredService {
            name: self.name.clone(),
            root: self.root.clone(),
            config_path: self.config_path.clone(),
            desired_running: self.desired_running,
            status: stored_status(self.status),
            message: self.message.clone(),
            task_count: self.host.as_ref().map_or(0, |host| host.start_plan().len()),
        }
    }
}

/// 聚合一个 Service 快照中全部 Task 的 CPU 与常驻内存。
fn aggregate_resources(snapshot: &ProjectSnapshot) -> Option<ResourceUsageDto> {
    let mut cpu = None;
    let mut memory = None;
    for resources in snapshot.tasks.iter().filter_map(|task| task.resources) {
        if let Some(value) = resources.cpu_tenths_percent {
            cpu = Some(cpu.unwrap_or(0_u16).saturating_add(value).min(1000_u16));
        }
        if let Some(value) = resources.memory_bytes {
            memory = Some(memory.unwrap_or(0_u64).saturating_add(value));
        }
    }
    (cpu.is_some() || memory.is_some()).then_some(ResourceUsageDto {
        cpu_tenths_percent: cpu,
        memory_bytes: memory,
    })
}

/// 从单条持久化记录恢复服务，失败时保留可诊断的注册项。
pub(crate) fn restore_service(stored: StoredService) -> ManagedService {
    match discover_path(&stored.config_path) {
        Ok(discovered) if discovered.compiled.spec.project != stored.name => ManagedService {
            name: stored.name.clone(),
            root: stored.root,
            config_path: stored.config_path,
            status: ServiceStatusDto::Failed,
            host: None,
            message: Some(format!(
                "配置中的服务名称已从 {} 变为 {}，需要显式迁移",
                stored.name, discovered.compiled.spec.project
            )),
            desired_running: stored.desired_running,
            pending_config: None,
            candidate_view: None,
            active_definition: None,
        },
        Ok(mut discovered) => {
            let definition = ActiveDefinition::from_compiled(&discovered.compiled);
            match super::project::prepare(&mut discovered) {
                Ok(_) => restore_valid(stored, discovered, definition),
                Err(error) => ManagedService {
                    name: stored.name,
                    root: stored.root,
                    config_path: stored.config_path,
                    status: ServiceStatusDto::Failed,
                    host: None,
                    message: Some(error.to_string()),
                    desired_running: stored.desired_running,
                    pending_config: None,
                    candidate_view: None,
                    active_definition: None,
                },
            }
        }
        Err(error) => ManagedService {
            name: stored.name,
            root: stored.root,
            config_path: stored.config_path,
            status: ServiceStatusDto::Failed,
            host: None,
            message: Some(error.to_string()),
            desired_running: stored.desired_running,
            pending_config: None,
            candidate_view: None,
            active_definition: None,
        },
    }
}

/// 恢复一个仍然有效的配置，并按持久化期望重新启动 Task。
fn restore_valid(
    stored: StoredService,
    discovered: crate::config::DiscoveredProject,
    active_definition: ActiveDefinition,
) -> ManagedService {
    let mut host = ServiceHost::from_compiled_at(discovered.compiled, &discovered.root);
    let start_error = stored
        .desired_running
        .then(|| host.start().err())
        .flatten()
        .map(|error| error.to_string());
    ManagedService {
        name: stored.name,
        root: discovered.root,
        config_path: discovered.config_path,
        status: if start_error.is_some() {
            ServiceStatusDto::Failed
        } else if stored.desired_running {
            ServiceStatusDto::Running
        } else {
            protocol_status(stored.status)
        },
        host: Some(host),
        message: start_error,
        desired_running: stored.desired_running,
        pending_config: None,
        candidate_view: None,
        active_definition: Some(active_definition),
    }
}

/// 把持久化状态映射为协议状态。
const fn protocol_status(status: StoredServiceStatus) -> ServiceStatusDto {
    match status {
        StoredServiceStatus::Running => ServiceStatusDto::Running,
        StoredServiceStatus::Stopped => ServiceStatusDto::Stopped,
        StoredServiceStatus::Failed => ServiceStatusDto::Failed,
    }
}
