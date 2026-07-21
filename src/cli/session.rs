use std::{
    collections::BTreeMap,
    io,
    time::{Duration, Instant},
};

use crate::core::TaskId;
use crate::daemon::{CenterClient, ServiceHost};
use crate::protocol::{
    CenterRequest, CenterResponse, EventBatchDto, LOG_STREAM_CHUNK_BYTES, LogCursorDto,
    ProjectSnapshot, ServiceActionDto, ServiceSelectorDto, ServiceViewDto,
};
use crate::tui::{LiveSession, LogUpdate, OverviewAction, OverviewExit, OverviewSession};

/// 运行全局中心的服务总览，并允许往返进入单服务详情。
pub(super) fn run_center_overview(
    client: &CenterClient,
    control_allowed: bool,
) -> anyhow::Result<()> {
    let services = request_services(client).map_err(anyhow::Error::from)?;
    let mut overview_session = CenterOverviewSession {
        client: client.clone(),
        last_services: services.clone(),
    };
    let mut app = crate::tui::OverviewApp::new(services);
    loop {
        match crate::tui::run_overview_live(&mut app, control_allowed, &mut overview_session)? {
            OverviewExit::Quit => return Ok(()),
            OverviewExit::OpenService(service_name) => {
                let selector = ServiceSelectorDto::Name(service_name.clone());
                let snapshot = request_snapshot(client, &selector).unwrap_or(ProjectSnapshot {
                    project: service_name,
                    source: crate::protocol::SnapshotSourceDto::CenterStale,
                    tasks: Vec::new(),
                });
                let hello = client.hello("procora-tui-detail")?;
                run_center_tui_mode(
                    client.clone(),
                    selector,
                    snapshot,
                    hello.event_sequence,
                    hello.control_allowed,
                    true,
                )?;
                overview_session.last_services =
                    request_services(client).map_err(anyhow::Error::from)?;
                app.replace_services(overview_session.last_services.clone());
            }
        }
    }
}

/// 中心会话主动拉取资源快照的最长间隔。
const RESOURCE_SNAPSHOT_INTERVAL: Duration = Duration::from_secs(1);

/// 使用中心事件游标运行可控制的实时 TUI 会话。
pub(super) fn run_center_tui(
    client: CenterClient,
    selector: ServiceSelectorDto,
    snapshot: ProjectSnapshot,
    event_sequence: u64,
    control_allowed: bool,
) -> anyhow::Result<()> {
    run_center_tui_mode(
        client,
        selector,
        snapshot,
        event_sequence,
        control_allowed,
        false,
    )
}

/// 按退出键语义运行中心单服务详情。
fn run_center_tui_mode(
    client: CenterClient,
    selector: ServiceSelectorDto,
    snapshot: ProjectSnapshot,
    event_sequence: u64,
    control_allowed: bool,
    back_navigation: bool,
) -> anyhow::Result<()> {
    let mut session = CenterTuiSession {
        client,
        selector,
        event_sequence,
        resource_schedule: ResourceSnapshotSchedule::new(Instant::now()),
        log_cursors: BTreeMap::new(),
    };
    if back_navigation {
        crate::tui::run_live_back(snapshot, control_allowed, &mut session)?;
    } else {
        crate::tui::run_live(snapshot, control_allowed, &mut session)?;
    }
    Ok(())
}

/// 全局中心服务总览的 IPC 会话。
struct CenterOverviewSession {
    client: CenterClient,
    last_services: Vec<ServiceViewDto>,
}

impl OverviewSession for CenterOverviewSession {
    fn poll_services(&mut self) -> io::Result<Option<Vec<ServiceViewDto>>> {
        let services = request_services(&self.client)?;
        if services == self.last_services {
            return Ok(None);
        }
        self.last_services.clone_from(&services);
        Ok(Some(services))
    }

    fn manage_overview(
        &mut self,
        service_name: &str,
        action: OverviewAction,
    ) -> io::Result<Vec<ServiceViewDto>> {
        let selector = ServiceSelectorDto::Name(service_name.to_owned());
        let response = match action {
            OverviewAction::Start => CenterRequest::Manage {
                action: ServiceActionDto::Start,
                selector,
            },
            OverviewAction::Stop => CenterRequest::Manage {
                action: ServiceActionDto::Stop,
                selector,
            },
            OverviewAction::Restart => CenterRequest::Manage {
                action: ServiceActionDto::Restart,
                selector,
            },
            OverviewAction::Remove => CenterRequest::Remove { selector },
        };
        match self.client.request(&response).map_err(io::Error::other)? {
            CenterResponse::Service(_) | CenterResponse::Removed(_) => {}
            CenterResponse::Error { message } => return Err(io::Error::other(message)),
            response => {
                return Err(io::Error::other(format!("意外服务管理响应: {response:?}")));
            }
        }
        let services = request_services(&self.client)?;
        self.last_services.clone_from(&services);
        Ok(services)
    }
}

/// 从中心读取完整服务摘要列表。
fn request_services(client: &CenterClient) -> io::Result<Vec<ServiceViewDto>> {
    match client
        .request(&CenterRequest::List)
        .map_err(io::Error::other)?
    {
        CenterResponse::Services(services) => Ok(services),
        CenterResponse::Error { message } => Err(io::Error::other(message)),
        response => Err(io::Error::other(format!("意外服务列表响应: {response:?}"))),
    }
}

/// 从中心读取指定服务详情快照。
fn request_snapshot(
    client: &CenterClient,
    selector: &ServiceSelectorDto,
) -> io::Result<ProjectSnapshot> {
    match client
        .request(&CenterRequest::Snapshot {
            selector: selector.clone(),
        })
        .map_err(io::Error::other)?
    {
        CenterResponse::Snapshot(snapshot) => Ok(snapshot),
        CenterResponse::Error { message } => Err(io::Error::other(message)),
        response => Err(io::Error::other(format!("意外快照响应: {response:?}"))),
    }
}

/// 运行与 TUI 同生命周期的临时服务会话。
///
/// # Errors
///
/// 当终端初始化、绘制、输入或服务会话操作失败时返回错误。
pub fn run_embedded_tui(host: &mut ServiceHost, snapshot: ProjectSnapshot) -> anyhow::Result<()> {
    let mut session = EmbeddedTuiSession::new(host);
    crate::tui::run_live(snapshot, true, &mut session)?;
    Ok(())
}

/// 为临时服务宿主提供状态、控制和日志能力。
pub struct EmbeddedTuiSession<'a> {
    host: &'a mut ServiceHost,
    running: bool,
    log_cursors: BTreeMap<TaskId, LogCursorDto>,
}

impl<'a> EmbeddedTuiSession<'a> {
    /// 创建一个已经启动的临时服务会话。
    pub fn new(host: &'a mut ServiceHost) -> Self {
        Self {
            host,
            running: true,
            log_cursors: BTreeMap::new(),
        }
    }

    /// 生成临时宿主的最新快照。
    fn snapshot(&mut self) -> ProjectSnapshot {
        self.host.snapshot(
            crate::protocol::SnapshotSourceDto::EmbeddedLive,
            self.running,
        )
    }
}

impl LiveSession for EmbeddedTuiSession<'_> {
    fn poll_snapshot(&mut self) -> io::Result<Option<ProjectSnapshot>> {
        Ok(Some(self.snapshot()))
    }

    fn manage(&mut self, action: ServiceActionDto) -> io::Result<ProjectSnapshot> {
        match action {
            ServiceActionDto::Start => {
                self.host.start().map_err(io::Error::other)?;
                self.running = true;
            }
            ServiceActionDto::Restart => {
                self.host.stop().map_err(io::Error::other)?;
                self.running = false;
                self.host.start().map_err(io::Error::other)?;
                self.running = true;
            }
            ServiceActionDto::Stop => {
                self.host.stop().map_err(io::Error::other)?;
                self.running = false;
            }
        }
        Ok(self.snapshot())
    }

    fn poll_log(&mut self, task_id: &TaskId) -> io::Result<Option<LogUpdate>> {
        let batch = self
            .host
            .read_task_log(
                task_id,
                self.log_cursors.get(task_id).copied(),
                LOG_STREAM_CHUNK_BYTES as usize,
            )
            .map_err(io::Error::other)?;
        self.log_cursors.insert(task_id.clone(), batch.next_cursor);
        Ok((batch.gap || !batch.bytes.is_empty()).then_some(LogUpdate {
            task_id: batch.task_id,
            bytes: batch.bytes,
            gap: batch.gap,
        }))
    }

    fn clear_log(&mut self, task_id: &TaskId) -> io::Result<()> {
        self.host
            .clear_task_log(task_id)
            .map_err(io::Error::other)?;
        self.log_cursors.remove(task_id);
        Ok(())
    }
}

/// CLI 为 TUI 适配的全局 Procora 服务器实时会话。
struct CenterTuiSession {
    client: CenterClient,
    selector: ServiceSelectorDto,
    event_sequence: u64,
    resource_schedule: ResourceSnapshotSchedule,
    log_cursors: BTreeMap<TaskId, LogCursorDto>,
}

/// 独立于状态事件维护资源快照拉取节奏。
#[derive(Debug)]
struct ResourceSnapshotSchedule {
    next_at: Instant,
}

impl ResourceSnapshotSchedule {
    /// 从最近一次完整快照开始计算下一次资源刷新时间。
    fn new(now: Instant) -> Self {
        Self {
            next_at: now + RESOURCE_SNAPSHOT_INTERVAL,
        }
    }

    /// 判断资源快照是否已经到期。
    fn is_due(&self, now: Instant) -> bool {
        now >= self.next_at
    }

    /// 在成功读取到期快照后安排下一次刷新。
    fn record_success(&mut self, now: Instant) {
        self.next_at = now + RESOURCE_SNAPSHOT_INTERVAL;
    }
}

impl CenterTuiSession {
    /// 从中心服务器重新读取当前服务快照。
    fn snapshot(&self) -> io::Result<ProjectSnapshot> {
        let response = self
            .client
            .request(&CenterRequest::Snapshot {
                selector: self.selector.clone(),
            })
            .map_err(io::Error::other)?;
        match response {
            CenterResponse::Snapshot(snapshot) => Ok(snapshot),
            CenterResponse::Error { message } => Err(io::Error::other(message)),
            response => Err(io::Error::other(format!("意外快照响应: {response:?}"))),
        }
    }
}

impl LiveSession for CenterTuiSession {
    fn poll_snapshot(&mut self) -> io::Result<Option<ProjectSnapshot>> {
        let response = self
            .client
            .request(&CenterRequest::Events {
                after_sequence: self.event_sequence,
            })
            .map_err(io::Error::other)?;
        let CenterResponse::Events(EventBatchDto {
            events,
            next_sequence,
            resync_required,
        }) = response
        else {
            return match response {
                CenterResponse::Error { message } => Err(io::Error::other(message)),
                response => Err(io::Error::other(format!("意外事件响应: {response:?}"))),
            };
        };
        self.event_sequence = next_sequence;
        let now = Instant::now();
        let resource_snapshot_due = self.resource_schedule.is_due(now);
        if resync_required || !events.is_empty() || resource_snapshot_due {
            let snapshot = self.snapshot()?;
            if resource_snapshot_due {
                self.resource_schedule.record_success(Instant::now());
            }
            return Ok(Some(snapshot));
        }
        Ok(None)
    }

    fn manage(&mut self, action: ServiceActionDto) -> io::Result<ProjectSnapshot> {
        let response = self
            .client
            .request(&CenterRequest::Manage {
                action,
                selector: self.selector.clone(),
            })
            .map_err(io::Error::other)?;
        match response {
            CenterResponse::Service(_) => self.snapshot(),
            CenterResponse::Error { message } => Err(io::Error::other(message)),
            response => Err(io::Error::other(format!("意外管理响应: {response:?}"))),
        }
    }

    fn poll_log(&mut self, task_id: &TaskId) -> io::Result<Option<LogUpdate>> {
        let batch = self
            .client
            .read_task_logs(
                &self.selector,
                task_id,
                self.log_cursors.get(task_id).copied(),
            )
            .map_err(io::Error::other)?;
        self.log_cursors.insert(task_id.clone(), batch.next_cursor);
        Ok((batch.gap || !batch.bytes.is_empty()).then_some(LogUpdate {
            task_id: batch.task_id,
            bytes: batch.bytes,
            gap: batch.gap,
        }))
    }

    fn clear_log(&mut self, task_id: &TaskId) -> io::Result<()> {
        let response = self
            .client
            .request(&CenterRequest::ClearTaskLogs {
                selector: self.selector.clone(),
                task_id: task_id.clone(),
            })
            .map_err(io::Error::other)?;
        match response {
            CenterResponse::TaskLogsCleared(cleared) if cleared == *task_id => {
                self.log_cursors.remove(task_id);
                Ok(())
            }
            CenterResponse::Error { message } => Err(io::Error::other(message)),
            response => Err(io::Error::other(format!("意外日志清空响应: {response:?}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    // 状态事件触发的额外快照不会推迟固定的一秒资源刷新。
    fn resource_snapshot_schedule_is_independent_from_events() {
        let started_at = Instant::now();
        let mut schedule = ResourceSnapshotSchedule::new(started_at);

        assert!(!schedule.is_due(started_at + Duration::from_millis(999)));
        assert!(schedule.is_due(started_at + RESOURCE_SNAPSHOT_INTERVAL));

        schedule.record_success(started_at + RESOURCE_SNAPSHOT_INTERVAL);
        assert!(!schedule.is_due(started_at + Duration::from_millis(1999)));
        assert!(schedule.is_due(started_at + Duration::from_secs(2)));
    }
}
