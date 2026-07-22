//! 中心服务、运行时、存储与依赖来源测试入口。

mod center;
#[path = "../support/command.rs"]
mod command_support;
mod dependency_manager;
mod event_loop;
mod exit_codes;
mod file_store;
mod git_source;
mod health_checks;
mod initial_state;
mod ipc;
mod log_stress;
mod managed_process;
mod process_tree;
mod remote_downloads;
mod soak;
mod sqlite_repository;
mod tail_buffer;
mod task_runtime;
mod uploads;
