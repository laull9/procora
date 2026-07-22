//! CLI、会话与 MCP 集成测试入口。

#[path = "../support/command.rs"]
mod command_support;

mod center_update;
mod cli;
mod cli_commands;
mod cli_git_source;
mod cli_reload;
mod cli_uploads;
mod cli_usability;
mod embedded_session;
mod mcp;
mod new_service;
