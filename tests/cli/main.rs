//! CLI、会话与 MCP 集成测试入口。

use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

#[path = "../support/command.rs"]
mod command_support;

mod center_update;
mod cli;
mod cli_commands;
mod cli_deploy;
mod cli_deploy_edges;
mod cli_git_source;
mod cli_package;
mod cli_package_management;
mod cli_package_workflow;
mod cli_reload;
mod cli_remote;
mod cli_update;
mod cli_uploads;
mod cli_uploads_list;
mod cli_usability;
mod embedded_session;
mod mcp;
mod new_service;
mod python_api;

/// 创建当前 CLI 测试独占的跨平台临时目录。
fn temporary_directory(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory = procora::platform::temp_dir().join(format!(
        "procora-cli-{label}-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(&directory).unwrap();
    directory
}
