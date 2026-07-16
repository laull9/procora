use std::{collections::BTreeMap, io};

use procora_core::TaskId;
use procora_daemon::{CenterClient, ServiceHost};
use procora_protocol::{
    CenterRequest, CenterResponse, EventBatchDto, LogCursorDto, ProjectSnapshot, ServiceActionDto,
    ServiceSelectorDto,
};
use procora_tui::{LiveSession, LogUpdate};

/// 使用中心事件游标运行可控制的实时 TUI 会话。
pub(super) fn run_center_tui(
    client: CenterClient,
    selector: ServiceSelectorDto,
    snapshot: ProjectSnapshot,
    event_sequence: u64,
    control_allowed: bool,
) -> anyhow::Result<()> {
    let mut session = CenterTuiSession {
        client,
        selector,
        event_sequence,
        log_cursors: BTreeMap::new(),
    };
    procora_tui::run_live(snapshot, control_allowed, &mut session)?;
    Ok(())
}

/// 运行与 TUI 同生命周期的临时服务会话。
///
/// # Errors
///
/// 当终端初始化、绘制、输入或服务会话操作失败时返回错误。
pub fn run_embedded_tui(host: &mut ServiceHost, snapshot: ProjectSnapshot) -> anyhow::Result<()> {
    let mut session = EmbeddedTuiSession::new(host);
    procora_tui::run_live(snapshot, true, &mut session)?;
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
            procora_protocol::SnapshotSourceDto::EmbeddedLive,
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
            .read_task_log(task_id, self.log_cursors.get(task_id).copied(), 16 * 1024)
            .map_err(io::Error::other)?;
        self.log_cursors.insert(task_id.clone(), batch.next_cursor);
        Ok((batch.gap || !batch.bytes.is_empty()).then_some(LogUpdate {
            task_id: batch.task_id,
            bytes: batch.bytes,
            gap: batch.gap,
        }))
    }
}

/// CLI 为 TUI 适配的全局 Procora 服务器实时会话。
struct CenterTuiSession {
    client: CenterClient,
    selector: ServiceSelectorDto,
    event_sequence: u64,
    log_cursors: BTreeMap<TaskId, LogCursorDto>,
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
        if resync_required || !events.is_empty() {
            return self.snapshot().map(Some);
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
        let response = self
            .client
            .request(&CenterRequest::TaskLogs {
                selector: self.selector.clone(),
                task_id: task_id.clone(),
                cursor: self.log_cursors.get(task_id).copied(),
                max_bytes: 16 * 1024,
            })
            .map_err(io::Error::other)?;
        match response {
            CenterResponse::TaskLogs(batch) => {
                self.log_cursors.insert(task_id.clone(), batch.next_cursor);
                Ok((batch.gap || !batch.bytes.is_empty()).then_some(LogUpdate {
                    task_id: batch.task_id,
                    bytes: batch.bytes,
                    gap: batch.gap,
                }))
            }
            CenterResponse::Error { message } => Err(io::Error::other(message)),
            response => Err(io::Error::other(format!("意外日志响应: {response:?}"))),
        }
    }
}
