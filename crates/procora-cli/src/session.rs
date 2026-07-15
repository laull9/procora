use std::{collections::BTreeMap, io};

use procora_core::TaskId;
use procora_daemon::CenterClient;
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

/// CLI 为 TUI 适配的中心服务器实时会话。
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
