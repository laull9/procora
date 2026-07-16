//! CLI/TUI 与中心服务器之间的版本化传输对象。

use std::path::PathBuf;

use crate::core::TaskId;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// 当前本地 IPC 协议主版本。
pub const PROTOCOL_VERSION: u16 = 2;

/// 客户端连接中心服务器时发送的握手请求。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ClientHello {
    /// 客户端支持的协议主版本。
    pub protocol_version: u16,
    /// 客户端实现名称。
    pub client_name: String,
}

/// 中心服务器对握手请求返回的身份与能力摘要。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CenterHello {
    /// 中心服务器采用的协议主版本。
    pub protocol_version: u16,
    /// 中心服务器实例标识。
    pub instance_id: Uuid,
    /// 当前注册的服务数量。
    pub service_count: usize,
    /// 当前中心事件序列，客户端从这里开始订阅增量。
    pub event_sequence: u64,
    /// 当前会话是否允许执行服务生命周期操作。
    pub control_allowed: bool,
}

/// 中心服务器增量事件的稳定类型。
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CenterEventKindDto {
    /// 服务被注册或重新打开。
    Opened,
    /// 服务生命周期状态发生变化。
    StatusChanged,
    /// 中心服务器准备正常退出。
    CenterStopping,
}

/// 中心服务器为前端保留的一条有序事件。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CenterEventDto {
    /// 在当前中心实例中单调递增的序列号。
    pub sequence: u64,
    /// 事件类型。
    pub kind: CenterEventKindDto,
    /// 服务相关事件的最新服务摘要。
    pub service: Option<ServiceViewDto>,
}

/// 从一个游标开始读取的有界增量事件批次。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EventBatchDto {
    /// 游标之后仍在缓冲区中的事件。
    pub events: Vec<CenterEventDto>,
    /// 客户端下一次请求应携带的游标。
    pub next_sequence: u64,
    /// 游标已过期或属于其他中心实例，客户端需要重新获取快照。
    pub resync_required: bool,
}

/// 服务状态历史的一条跨进程记录。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ServiceStatusRecordDto {
    /// 服务稳定名称。
    pub service_name: String,
    /// 当时的服务状态。
    pub status: ServiceStatusDto,
    /// 当时的错误或降级说明。
    pub message: Option<String>,
    /// Unix 纪元后的毫秒时间戳。
    pub recorded_at_ms: i64,
}

/// Service 本地活动日志的代次与字节游标。
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct LogCursorDto {
    /// 活动日志每次轮转后递增的代次。
    pub generation: u64,
    /// 当前代已经消费的字节偏移。
    pub offset: u64,
}

/// 从 Service 本地日志文件读取的一批有界内容。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LogBatchDto {
    /// 本批日志所属 Task。
    pub task_id: TaskId,
    /// 不假设字符编码的原始日志字节。
    pub bytes: Vec<u8>,
    /// 下一次续读使用的游标。
    pub next_cursor: LogCursorDto,
    /// 原游标跨越了轮转或截断边界。
    pub gap: bool,
}

/// 客户端当前看到的项目数据来源。
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotSourceDto {
    /// 数据由本地配置生成，尚未连接服务宿主。
    ConfigPreview,
    /// 数据来自与当前 TUI 同生命周期的嵌入式服务宿主。
    EmbeddedLive,
    /// 数据来自实时中心服务器会话。
    CenterLive,
    /// 中心服务器连接中断，当前数据可能已经过期。
    CenterStale,
}

/// 中心服务器对托管服务采用的稳定运行状态。
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceStatusDto {
    /// 服务宿主已经加载并处于运行期望状态。
    Running,
    /// 服务已经注册，但当前处于停止状态。
    Stopped,
    /// 服务配置加载或生命周期操作失败。
    Failed,
}

/// CLI 管理命令支持的服务生命周期动作。
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceActionDto {
    /// 启动已经注册的服务。
    Start,
    /// 重新加载配置并重启服务。
    Restart,
    /// 停止服务。
    Stop,
}

/// 服务可以通过稳定名称或服务目录定位。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceSelectorDto {
    /// 按配置中的项目名称定位。
    Name(String),
    /// 按服务目录或配置文件路径定位。
    Path(PathBuf),
}

/// 中心服务器公开的托管服务摘要。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ServiceViewDto {
    /// 配置中的稳定服务名称。
    pub name: String,
    /// 服务所在的规范化目录。
    pub root: PathBuf,
    /// 当前注册的规范化配置文件路径。
    pub config_path: PathBuf,
    /// 当前服务运行状态。
    pub status: ServiceStatusDto,
    /// 最近一次成功加载的任务数量。
    pub task_count: usize,
    /// 失败或降级状态的可读说明。
    pub message: Option<String>,
}

/// CLI 发给中心服务器的单次请求。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CenterRequest {
    /// 建立版本化会话并读取中心身份与能力。
    Hello(ClientHello),
    /// 探测中心服务器是否可用。
    Ping,
    /// 从目录或显式配置文件注册并启动服务。
    Open {
        /// 要扫描的服务目录或显式配置文件。
        path: PathBuf,
    },
    /// 列出所有已经注册的服务。
    List,
    /// 从指定序列号之后读取中心增量事件。
    Events {
        /// 客户端最后处理完成的事件序列号。
        after_sequence: u64,
    },
    /// 查询指定服务的持久化状态历史。
    History {
        /// 要查询的服务。
        selector: ServiceSelectorDto,
    },
    /// 从指定 Task 的 Service 本地日志文件续读。
    TaskLogs {
        /// 要读取的服务。
        selector: ServiceSelectorDto,
        /// 服务内的 Task。
        task_id: TaskId,
        /// 上次已经消费的文件游标。
        cursor: Option<LogCursorDto>,
        /// 单次响应允许返回的最大字节数。
        max_bytes: u32,
    },
    /// 读取指定服务的任务快照。
    Snapshot {
        /// 要读取的服务。
        selector: ServiceSelectorDto,
    },
    /// 对指定服务执行生命周期动作。
    Manage {
        /// 要执行的动作。
        action: ServiceActionDto,
        /// 要管理的服务。
        selector: ServiceSelectorDto,
    },
    /// 请求中心服务器完成当前响应后正常退出。
    Shutdown,
}

/// 中心服务器对单次请求返回的响应。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CenterResponse {
    /// 返回中心服务器身份、版本和会话能力。
    Hello(CenterHello),
    /// 中心服务器已响应探测。
    Pong,
    /// 返回服务列表。
    Services(Vec<ServiceViewDto>),
    /// 返回有界增量事件批次。
    Events(EventBatchDto),
    /// 返回按写入顺序排列的服务状态历史。
    History(Vec<ServiceStatusRecordDto>),
    /// 返回一批 Task 文件日志。
    TaskLogs(LogBatchDto),
    /// 返回单个服务摘要。
    Service(ServiceViewDto),
    /// 返回供 TUI 使用的一致性任务快照。
    Snapshot(ProjectSnapshot),
    /// 中心服务器已经接受正常退出请求。
    ShuttingDown,
    /// 请求未能完成。
    Error {
        /// 可直接展示给本机用户的错误说明。
        message: String,
    },
}

/// 面向客户端展示的稳定任务状态值。
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatusDto {
    /// 任务等待调度。
    Pending,
    /// 任务被依赖或策略阻断。
    Blocked,
    /// 任务正在运行。
    Running,
    /// 任务已经停止。
    Stopped,
    /// 任务执行失败。
    Failed,
}

/// 任务资源使用的跨平台传输值。
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceUsageDto {
    /// CPU 百分比的十分之一，例如 123 表示 12.3%。
    pub cpu_tenths_percent: Option<u16>,
    /// 常驻内存字节数。
    pub memory_bytes: Option<u64>,
}

/// 状态快照中的单任务视图。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TaskView {
    /// 任务稳定标识。
    pub task_id: TaskId,
    /// 用于观察界面展示的命令摘要。
    pub command: String,
    /// 传输层稳定状态。
    pub status: TaskStatusDto,
    /// 该任务直接依赖的任务标识。
    pub dependencies: Vec<TaskId>,
    /// 服务宿主可用时返回的资源快照。
    pub resources: Option<ResourceUsageDto>,
    /// 对失败、阻塞或过期状态的简短解释。
    pub message: Option<String>,
}

/// TUI 首次渲染使用的一致性项目快照。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProjectSnapshot {
    /// 当前项目稳定标识。
    pub project: String,
    /// 快照的数据来源和连接新鲜度。
    pub source: SnapshotSourceDto,
    /// 按服务端或配置编译器确定顺序排列的任务视图。
    pub tasks: Vec<TaskView>,
}
