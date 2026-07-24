//! 服务宿主的 Task 诊断写入与文件日志访问。

use uuid::Uuid;

use crate::{
    core::TaskId,
    engine::TaskRunIdentity,
    log::{LogFrame, LogStream},
    protocol::{LogBatchDto, LogCursorDto, TaskDiagnosticKindDto},
};

use super::{ServiceHost, diagnostics};

impl ServiceHost {
    /// 同时写入综合分析、内存尾部和持久 Task 日志。
    pub(super) fn record_task_diagnostic(
        &mut self,
        task_id: &TaskId,
        identity: Option<TaskRunIdentity>,
        kind: TaskDiagnosticKindDto,
        message: impl Into<String>,
        suggestion: Option<String>,
    ) {
        let recorded =
            diagnostics::record_shared(&self.diagnostics, task_id, kind, message, suggestion);
        if !recorded.emit_log {
            return;
        }
        let bytes = diagnostics::styled_log(&recorded.diagnostic);
        let frame = LogFrame {
            task_id: task_id.clone(),
            run_id: identity.map_or_else(Uuid::nil, |identity| identity.run_id),
            sequence: self.diagnostic_sequence,
            stream: LogStream::System,
            bytes: bytes.clone(),
        };
        self.diagnostic_sequence = self.diagnostic_sequence.saturating_add(1);
        if let Ok(mut tail) = self.logs.lock() {
            tail.push(frame);
        }
        self.flush_logs();
        if let Some(files) = &self.file_logs
            && let Err(error) = files.append_task(task_id, &bytes)
        {
            tracing::warn!(task = %task_id, %error, "Task 诊断日志写入失败");
        }
    }

    /// 向所属服务目录中的服务级日志追加内容。
    ///
    /// # Errors
    ///
    /// 当持久日志已配置但文件写入或压缩失败时返回错误。
    pub fn append_service_log(&self, bytes: &[u8]) -> Result<(), crate::log::FileLogError> {
        self.file_logs
            .as_ref()
            .map_or(Ok(()), |logs| logs.append_service(bytes))
    }

    /// 从 Service 本地文件续读指定 Task 日志。
    ///
    /// # Errors
    ///
    /// 当嵌入模式没有文件存储，或文件与索引无法读取时返回错误。
    pub fn read_task_log(
        &self,
        task_id: &TaskId,
        cursor: Option<LogCursorDto>,
        max_bytes: usize,
    ) -> Result<LogBatchDto, crate::log::FileLogError> {
        let files = self.file_logs.as_ref().ok_or_else(|| {
            crate::log::FileLogError::Io(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "嵌入模式没有持久文件日志",
            ))
        })?;
        let batch = files.read_task(
            task_id,
            cursor.map(|cursor| crate::log::FileLogCursor {
                generation: cursor.generation,
                offset: cursor.offset,
            }),
            max_bytes,
        )?;
        Ok(LogBatchDto {
            task_id: task_id.clone(),
            bytes: batch.bytes,
            next_cursor: LogCursorDto {
                generation: batch.next_cursor.generation,
                offset: batch.next_cursor.offset,
            },
            gap: batch.gap,
        })
    }

    /// 刷新待写内容并清空指定 Task 的文件日志。
    ///
    /// # Errors
    ///
    /// 当嵌入模式没有文件存储或日志文件无法更新时返回错误。
    pub fn clear_task_log(&self, task_id: &TaskId) -> Result<(), crate::log::FileLogError> {
        self.flush_logs();
        self.file_logs
            .as_ref()
            .ok_or_else(|| {
                crate::log::FileLogError::Io(std::io::Error::new(
                    std::io::ErrorKind::Unsupported,
                    "嵌入模式没有持久文件日志",
                ))
            })?
            .clear_task(task_id)
    }

    /// 在限定时间内刷新此前成功进入有界队列的文件日志。
    pub(super) fn flush_logs(&self) {
        if let Some(writer) = &self.log_writer {
            writer.flush();
        }
    }
}
