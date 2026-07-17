//! 中心服务器、IPC 与前台/后台共享的服务宿主。

mod center;
mod health;
mod host;
mod host_logs;
mod host_view;
mod ipc;
mod managed;
mod project;
mod resources;
mod status;

pub use center::{Center, CenterError};
pub use host::{ServiceHost, ServiceHostError};
pub use ipc::{CenterClient, IpcError, run_center_server};
