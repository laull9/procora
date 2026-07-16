use std::{
    collections::{BTreeMap, VecDeque},
    fs,
    path::{Path, PathBuf},
};

use crate::config::{DiscoveryError, discover_path};
use crate::protocol::{
    CenterEventDto, CenterEventKindDto, CenterRequest, CenterResponse, EventBatchDto,
    ServiceActionDto, ServiceSelectorDto, ServiceStatusDto, ServiceStatusRecordDto, ServiceViewDto,
    SnapshotSourceDto,
};
use crate::source::SourceError;
use crate::storage::{SqliteCenterRepository, StorageError};
use thiserror::Error;
use uuid::Uuid;

use super::{
    ServiceHost,
    managed::ManagedService,
    project::{EVENT_CAPACITY, MAX_LOG_BATCH_BYTES},
    status::{protocol_status, status_text},
};

mod registry;

/// 中心服务器注册、恢复和服务解析错误。
#[derive(Debug, Error)]
pub enum CenterError {
    /// 服务配置发现或编译失败。
    #[error(transparent)]
    Discovery(#[from] DiscoveryError),
    /// 项目依赖下载、解包或版本验证失败。
    #[error(transparent)]
    Source(#[from] SourceError),
    /// 注册表持久化失败。
    #[error(transparent)]
    Storage(#[from] StorageError),
    /// 服务名称已经被其他目录使用。
    #[error("服务名称 `{name}` 已由目录 `{existing}` 使用")]
    DuplicateName {
        /// 冲突的稳定名称。
        name: String,
        /// 已注册服务目录。
        existing: PathBuf,
    },
    /// 服务目录已经用其他名称注册。
    #[error("服务目录 `{root}` 已注册为 `{existing}`，不能再注册为 `{requested}`")]
    DuplicateRoot {
        /// 冲突的服务目录。
        root: PathBuf,
        /// 已注册名称。
        existing: String,
        /// 新配置中的名称。
        requested: String,
    },
    /// 请求中的服务名称或路径没有匹配项。
    #[error("找不到服务 `{0}`")]
    NotFound(String),
    /// 服务路径无法规范化。
    #[error("无法访问服务路径 `{path}`: {source}")]
    InvalidSelectorPath {
        /// 请求中的服务路径。
        path: PathBuf,
        /// 文件系统错误。
        source: std::io::Error,
    },
    /// 目标服务当前没有可用的已编译配置。
    #[error("服务 `{0}` 当前不可用，请修复配置后执行 restart")]
    Unavailable(String),
}

/// 管理本机多个服务宿主的中心服务器状态。
#[derive(Debug)]
pub struct Center {
    services: BTreeMap<String, ManagedService>,
    repository: SqliteCenterRepository,
    instance_id: Uuid,
    event_sequence: u64,
    events: VecDeque<CenterEventDto>,
}

impl Center {
    /// 从持久化注册表恢复中心服务器。
    ///
    /// # Errors
    ///
    /// 当注册表无法读取时返回错误；单个服务配置损坏会恢复为失败状态。
    pub fn load(repository: SqliteCenterRepository) -> Result<Self, CenterError> {
        let stored_services = repository.load_services()?;
        let mut services = BTreeMap::new();
        for stored in stored_services {
            let service = super::managed::restore_service(stored);
            services.insert(service.name.clone(), service);
        }
        let center = Self {
            services,
            repository,
            instance_id: Uuid::new_v4(),
            event_sequence: 0,
            events: VecDeque::with_capacity(EVENT_CAPACITY),
        };
        center.persist_all()?;
        Ok(center)
    }

    /// 创建不复用任何已有状态的中心服务器，主要用于测试和嵌入模式。
    pub fn empty(repository: SqliteCenterRepository) -> Self {
        Self {
            services: BTreeMap::new(),
            repository,
            instance_id: Uuid::new_v4(),
            event_sequence: 0,
            events: VecDeque::with_capacity(EVENT_CAPACITY),
        }
    }

    /// 处理一条已经解码的中心服务器请求。
    pub fn handle(&mut self, request: CenterRequest) -> CenterResponse {
        let result = match request {
            CenterRequest::Hello(hello) => return self.hello(&hello),
            CenterRequest::Ping => return CenterResponse::Pong,
            CenterRequest::Open { path } => self.open(&path).map(CenterResponse::Service),
            CenterRequest::List => return CenterResponse::Services(self.list()),
            CenterRequest::Events { after_sequence } => {
                self.tick();
                return CenterResponse::Events(self.events_after(after_sequence));
            }
            CenterRequest::History { selector } => {
                self.history(&selector).map(CenterResponse::History)
            }
            CenterRequest::TaskLogs {
                selector,
                task_id,
                cursor,
                max_bytes,
            } => self
                .task_logs(&selector, &task_id, cursor, max_bytes)
                .map(CenterResponse::TaskLogs),
            CenterRequest::Snapshot { selector } => self.snapshot(&selector),
            CenterRequest::Manage { action, selector } => {
                self.manage(action, &selector).map(CenterResponse::Service)
            }
            CenterRequest::Remove { selector } => {
                self.remove(&selector).map(CenterResponse::Removed)
            }
            CenterRequest::Shutdown => {
                self.stop_all_hosts();
                self.push_event(CenterEventKindDto::CenterStopping, None);
                return CenterResponse::ShuttingDown;
            }
        };
        result.unwrap_or_else(|error| CenterResponse::Error {
            message: error.to_string(),
        })
    }

    /// 发现、注册并进入运行状态。
    fn open(&mut self, path: &Path) -> Result<ServiceViewDto, CenterError> {
        let mut discovered = discover_path(path)?;
        super::project::prepare(&mut discovered)?;
        let name = discovered.compiled.spec.project.clone();
        self.check_registration_conflicts(&name, &discovered.root)?;
        if let Some(host) = self
            .services
            .get_mut(&name)
            .and_then(|service| service.host.as_mut())
        {
            let _ = host.stop();
        }
        let root = discovered.root;
        let mut host = ServiceHost::from_compiled_at(discovered.compiled, &root);
        let start_error = host.start().err().map(|error| error.to_string());
        let service = ManagedService {
            name: name.clone(),
            root,
            config_path: discovered.config_path,
            status: if start_error.is_some() {
                ServiceStatusDto::Failed
            } else {
                ServiceStatusDto::Running
            },
            host: Some(host),
            message: start_error,
            desired_running: true,
        };
        let view = service.view();
        self.services.insert(name.clone(), service);
        self.persist_service(&name)?;
        self.write_status_log(&name);
        self.push_event(CenterEventKindDto::Opened, Some(view.clone()));
        Ok(view)
    }

    /// 返回按稳定名称排序的服务列表。
    fn list(&self) -> Vec<ServiceViewDto> {
        self.services.values().map(ManagedService::view).collect()
    }

    /// 返回指定服务的 TUI 快照。
    fn snapshot(&mut self, selector: &ServiceSelectorDto) -> Result<CenterResponse, CenterError> {
        let name = self.resolve_name(selector)?;
        let service = self.services.get_mut(&name).expect("名称已经解析");
        let running = service.status == ServiceStatusDto::Running;
        let host = service
            .host
            .as_mut()
            .ok_or_else(|| CenterError::Unavailable(service.name.clone()))?;
        Ok(CenterResponse::Snapshot(
            host.snapshot(SnapshotSourceDto::CenterLive, running),
        ))
    }

    /// 启动、重启或停止指定服务。
    fn manage(
        &mut self,
        action: ServiceActionDto,
        selector: &ServiceSelectorDto,
    ) -> Result<ServiceViewDto, CenterError> {
        let name = self.resolve_name(selector)?;
        match action {
            ServiceActionDto::Stop => {
                let service = self.services.get_mut(&name).expect("名称已经解析");
                let stop_error = service
                    .host
                    .as_mut()
                    .and_then(|host| host.stop().err())
                    .map(|error| error.to_string());
                service.status = if stop_error.is_some() {
                    ServiceStatusDto::Failed
                } else {
                    ServiceStatusDto::Stopped
                };
                service.message = stop_error;
                service.desired_running = false;
            }
            ServiceActionDto::Start | ServiceActionDto::Restart => {
                let config_path = self.services[&name].config_path.clone();
                self.services
                    .get_mut(&name)
                    .expect("名称已经解析")
                    .desired_running = true;
                match discover_path(&config_path) {
                    Ok(mut discovered) => {
                        if let Err(error) = super::project::prepare(&mut discovered) {
                            let service = self.services.get_mut(&name).expect("名称已经解析");
                            service.status = ServiceStatusDto::Failed;
                            service.message = Some(error.to_string());
                            return Err(error.into());
                        }
                        let new_name = discovered.compiled.spec.project.clone();
                        if new_name != name {
                            return Err(CenterError::DuplicateRoot {
                                root: discovered.root,
                                existing: name,
                                requested: new_name,
                            });
                        }
                        let root = discovered.root;
                        if let Some(host) = self
                            .services
                            .get_mut(&new_name)
                            .and_then(|service| service.host.as_mut())
                        {
                            let _ = host.stop();
                        }
                        let mut host = ServiceHost::from_compiled_at(discovered.compiled, &root);
                        let start_error = host.start().err().map(|error| error.to_string());
                        let service = self.services.get_mut(&new_name).expect("名称已经解析");
                        service.root = root;
                        service.config_path = discovered.config_path;
                        service.host = Some(host);
                        service.status = if start_error.is_some() {
                            ServiceStatusDto::Failed
                        } else {
                            ServiceStatusDto::Running
                        };
                        service.message = start_error;
                    }
                    Err(error) => {
                        let service = self.services.get_mut(&name).expect("名称已经解析");
                        service.status = ServiceStatusDto::Failed;
                        service.message = Some(error.to_string());
                        self.persist_service(&name)?;
                        self.write_status_log(&name);
                        return Err(error.into());
                    }
                }
            }
        }
        self.persist_service(&name)?;
        self.write_status_log(&name);
        let view = self.services[&name].view();
        self.push_event(CenterEventKindDto::StatusChanged, Some(view.clone()));
        Ok(view)
    }

    /// 返回指定服务的持久化状态历史。
    fn history(
        &self,
        selector: &ServiceSelectorDto,
    ) -> Result<Vec<ServiceStatusRecordDto>, CenterError> {
        let name = self.resolve_name(selector)?;
        self.repository
            .status_history(&name)?
            .into_iter()
            .map(|record| {
                Ok(ServiceStatusRecordDto {
                    service_name: record.service_name,
                    status: protocol_status(record.status),
                    message: record.message,
                    recorded_at_ms: record.recorded_at_ms,
                })
            })
            .collect()
    }

    /// 从指定 `ServiceHost` 自身目录续读 Task 文件日志。
    fn task_logs(
        &mut self,
        selector: &ServiceSelectorDto,
        task_id: &crate::core::TaskId,
        cursor: Option<crate::protocol::LogCursorDto>,
        max_bytes: u32,
    ) -> Result<crate::protocol::LogBatchDto, CenterError> {
        let name = self.resolve_name(selector)?;
        let service = self.services.get_mut(&name).expect("名称已经解析");
        let host = service
            .host
            .as_mut()
            .ok_or_else(|| CenterError::Unavailable(name))?;
        host.read_task_log(
            task_id,
            cursor,
            (max_bytes as usize).min(MAX_LOG_BATCH_BYTES),
        )
        .map_err(|error| CenterError::Unavailable(error.to_string()))
    }

    /// 读取指定游标之后仍然保留的中心事件。
    fn events_after(&self, after_sequence: u64) -> EventBatchDto {
        let oldest = self
            .events
            .front()
            .map_or(self.event_sequence + 1, |event| event.sequence);
        let resync_required =
            after_sequence > self.event_sequence || after_sequence.saturating_add(1) < oldest;
        let events = if resync_required {
            Vec::new()
        } else {
            self.events
                .iter()
                .filter(|event| event.sequence > after_sequence)
                .cloned()
                .collect()
        };
        EventBatchDto {
            events,
            next_sequence: self.event_sequence,
            resync_required,
        }
    }

    /// 追加一条有界中心事件并推进游标。
    fn push_event(&mut self, kind: CenterEventKindDto, service: Option<ServiceViewDto>) {
        self.event_sequence = self.event_sequence.saturating_add(1);
        if self.events.len() == EVENT_CAPACITY {
            self.events.pop_front();
        }
        self.events.push_back(CenterEventDto {
            sequence: self.event_sequence,
            kind,
            service,
        });
    }

    /// 检查稳定名称和目录的一一对应关系。
    fn check_registration_conflicts(&self, name: &str, root: &Path) -> Result<(), CenterError> {
        if let Some(existing) = self.services.get(name)
            && existing.root != root
        {
            return Err(CenterError::DuplicateName {
                name: name.to_owned(),
                existing: existing.root.clone(),
            });
        }
        if let Some(existing) = self.services.values().find(|service| service.root == root)
            && existing.name != name
        {
            return Err(CenterError::DuplicateRoot {
                root: root.to_path_buf(),
                existing: existing.name.clone(),
                requested: name.to_owned(),
            });
        }
        Ok(())
    }

    /// 把名称或路径选择器解析为稳定服务名称。
    fn resolve_name(&self, selector: &ServiceSelectorDto) -> Result<String, CenterError> {
        match selector {
            ServiceSelectorDto::Name(name) => self
                .services
                .contains_key(name)
                .then(|| name.clone())
                .ok_or_else(|| CenterError::NotFound(name.clone())),
            ServiceSelectorDto::Path(path) => {
                let canonical =
                    fs::canonicalize(path).map_err(|source| CenterError::InvalidSelectorPath {
                        path: path.clone(),
                        source,
                    })?;
                self.services
                    .values()
                    .find(|service| service.root == canonical || service.config_path == canonical)
                    .map(|service| service.name.clone())
                    .ok_or_else(|| CenterError::NotFound(path.display().to_string()))
            }
        }
    }

    /// 保存单个服务当前状态并按需追加状态历史。
    fn persist_service(&self, name: &str) -> Result<(), CenterError> {
        self.repository
            .save_service(&self.services[name].stored())?;
        Ok(())
    }

    /// 在中心服务器恢复后刷新全部服务当前状态。
    fn persist_all(&self) -> Result<(), CenterError> {
        for service in self.services.values() {
            self.repository.save_service(&service.stored())?;
        }
        Ok(())
    }

    /// 轮询全部宿主，并把 Task 状态变化提升为服务增量事件。
    pub(crate) fn tick(&mut self) {
        let changed = self
            .services
            .iter_mut()
            .filter_map(|(name, service)| {
                service
                    .host
                    .as_mut()
                    .is_some_and(ServiceHost::refresh)
                    .then(|| name.clone())
            })
            .collect::<Vec<_>>();
        for name in changed {
            let view = self.services[&name].view();
            self.push_event(CenterEventKindDto::StatusChanged, Some(view));
        }
    }

    /// 正常关闭 Center 时停止全部进程，但保留持久化运行期望供下次恢复。
    fn stop_all_hosts(&mut self) {
        for (name, service) in &mut self.services {
            if let Some(host) = service.host.as_mut()
                && let Err(error) = host.stop()
            {
                tracing::warn!(service = name, %error, "Center 关闭时停止服务失败");
            }
        }
    }

    /// 把服务状态变化写入该服务自己的日志目录。
    fn write_status_log(&self, name: &str) {
        let service = &self.services[name];
        let message = format!(
            "[procora] service_status={}{}\n",
            status_text(service.status),
            service
                .message
                .as_ref()
                .map_or_else(String::new, |message| format!(" message={message}"))
        );
        if let Some(host) = &service.host
            && let Err(error) = host.append_service_log(message.as_bytes())
        {
            tracing::warn!(service = name, %error, "服务状态日志写入失败");
        }
    }
}
