use std::path::PathBuf;

use crate::config::discover_path;
use crate::protocol::{ServiceStatusDto, ServiceViewDto};
use crate::storage::{StoredService, StoredServiceStatus};

use super::{ServiceHost, status::stored_status};

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
            message: self.message.clone(),
        }
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
        },
        Ok(mut discovered) => match super::project::prepare(&mut discovered) {
            Ok(_) => restore_valid(stored, discovered),
            Err(error) => ManagedService {
                name: stored.name,
                root: stored.root,
                config_path: stored.config_path,
                status: ServiceStatusDto::Failed,
                host: None,
                message: Some(error.to_string()),
                desired_running: stored.desired_running,
            },
        },
        Err(error) => ManagedService {
            name: stored.name,
            root: stored.root,
            config_path: stored.config_path,
            status: ServiceStatusDto::Failed,
            host: None,
            message: Some(error.to_string()),
            desired_running: stored.desired_running,
        },
    }
}

/// 恢复一个仍然有效的配置，并按持久化期望重新启动 Task。
fn restore_valid(
    stored: StoredService,
    discovered: crate::config::DiscoveredProject,
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
