//! Procora 单仓库任务服务管理器。

/// 命令行解析、运行期命令和会话管理。
pub mod cli;
/// 多格式配置读取、校验和任务图编译。
pub mod config;
/// 领域类型、任务规范与任务图。
pub mod core;
/// 中心服务器、IPC 与服务宿主。
pub mod daemon;
/// 任务状态机、调度计划与运行期对账。
pub mod engine;
/// 任务日志帧、文件存储与尾部缓冲。
pub mod log;
/// 面向本地智能体的 MCP stdio 服务。
pub mod mcp;
/// 受管任务进程的跨平台资源采样。
pub mod monitor;
/// 操作系统能力、标准目录与自启动集成。
pub mod platform;
/// 受管子进程的创建、输出与回收。
pub mod process;
/// 本地 IPC 的版本化传输对象。
pub mod protocol;
/// 配置来源、依赖下载和变更监听。
pub mod source;
/// SQLite 状态持久化。
pub mod storage;
/// 远端声明式文件传输实现。
mod transfer;
/// 终端用户界面。
pub mod tui;
