use crate::protocol::ServiceStatusDto;
use crate::storage::StoredServiceStatus;

/// 把协议服务状态映射为 `SQLite` 稳定状态。
pub(crate) const fn stored_status(status: ServiceStatusDto) -> StoredServiceStatus {
    match status {
        ServiceStatusDto::Running => StoredServiceStatus::Running,
        ServiceStatusDto::Stopped => StoredServiceStatus::Stopped,
        ServiceStatusDto::Failed => StoredServiceStatus::Failed,
    }
}

/// 把 `SQLite` 稳定状态映射为协议状态。
pub(crate) const fn protocol_status(status: StoredServiceStatus) -> ServiceStatusDto {
    match status {
        StoredServiceStatus::Running => ServiceStatusDto::Running,
        StoredServiceStatus::Stopped => ServiceStatusDto::Stopped,
        StoredServiceStatus::Failed => ServiceStatusDto::Failed,
    }
}

/// 返回服务状态日志使用的稳定文本。
pub(crate) const fn status_text(status: ServiceStatusDto) -> &'static str {
    match status {
        ServiceStatusDto::Running => "running",
        ServiceStatusDto::Stopped => "stopped",
        ServiceStatusDto::Failed => "failed",
    }
}
