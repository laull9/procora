//! Procora 的本地 MCP stdio 服务。

use std::path::PathBuf;

use rmcp::{
    ServerHandler, ServiceExt,
    handler::server::{
        router::{prompt::PromptRouter, tool::ToolRouter},
        wrapper::Parameters,
    },
    model::{
        CallToolResult, ContentBlock, GetPromptResult, Implementation, PromptMessage, Role,
        ServerCapabilities, ServerInfo,
    },
    prompt, prompt_handler, prompt_router,
    schemars::{self, JsonSchema},
    tool, tool_handler, tool_router,
    transport::stdio,
};
use serde::{Deserialize, Serialize};

use crate::{cli::api, config::is_python_config, protocol::ServiceActionDto};

/// 编译进二进制的 CLI 与中心服务器语义文档。
const CLI_GUIDE: &str = include_str!("../docs/cli.md");
/// 编译进二进制的配置参考文档。
const CONFIGURATION_GUIDE: &str = include_str!("../docs/configuration.md");
/// 编译进二进制的运行时语义文档。
const RUNTIME_GUIDE: &str = include_str!("../docs/runtime.md");

/// 只包含一个配置路径的工具参数。
#[derive(Debug, Deserialize, JsonSchema)]
struct PathParams {
    /// 服务目录或声明式配置文件路径。
    path: PathBuf,
}

/// 只包含一个服务目标的工具参数。
#[derive(Debug, Deserialize, JsonSchema)]
struct TargetParams {
    /// 配置中的服务名称、服务目录或显式配置文件。
    target: String,
}

/// 服务生命周期动作。
#[derive(Clone, Copy, Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum ManageAction {
    /// 启动已注册服务。
    Start,
    /// 重新加载配置并重启服务。
    Restart,
    /// 停止服务但保留注册。
    Stop,
}

impl From<ManageAction> for ServiceActionDto {
    fn from(action: ManageAction) -> Self {
        match action {
            ManageAction::Start => Self::Start,
            ManageAction::Restart => Self::Restart,
            ManageAction::Stop => Self::Stop,
        }
    }
}

/// 服务生命周期工具参数。
#[derive(Debug, Deserialize, JsonSchema)]
struct ManageParams {
    /// 配置中的服务名称、服务目录或显式配置文件。
    target: String,
    /// 要执行的生命周期动作。
    action: ManageAction,
}

/// 配置应用工具参数。
#[derive(Debug, Deserialize, JsonSchema)]
struct ApplyParams {
    /// 配置中的服务名称、服务目录或显式配置文件。
    target: String,
    /// `preview_config` 返回的完整 SHA-256 修订。
    revision: String,
}

/// Procora MCP 工具与内嵌文档 Prompts 服务。
#[derive(Clone, Debug)]
pub struct ProcoraMcpServer {
    tool_router: ToolRouter<Self>,
    prompt_router: PromptRouter<Self>,
}

impl Default for ProcoraMcpServer {
    fn default() -> Self {
        Self {
            tool_router: Self::tool_router(),
            prompt_router: Self::prompt_router(),
        }
    }
}

#[allow(
    clippy::unused_self,
    reason = "rmcp 工具路由宏要求工具作为服务实例方法注册"
)]
#[tool_router]
impl ProcoraMcpServer {
    /// 校验声明式配置且不下载依赖、注册服务或启动 Task。
    #[tool(description = "校验 Procora 声明式配置，不产生运行副作用")]
    fn validate_project(&self, Parameters(params): Parameters<PathParams>) -> CallToolResult {
        tool_result(declarative_path(params.path).and_then(|path| api::validate_project(&path)))
    }

    /// 返回声明式配置中确定性的 Task 启动顺序。
    #[tool(description = "返回 Procora Task 的确定性启动拓扑顺序")]
    fn task_graph(&self, Parameters(params): Parameters<PathParams>) -> CallToolResult {
        tool_result(declarative_path(params.path).and_then(|path| api::task_graph(&path)))
    }

    /// 返回带默认值、来源和规范化路径的有效配置。
    #[tool(description = "返回 Procora 规范化后的完整有效配置 JSON")]
    fn effective_config(&self, Parameters(params): Parameters<PathParams>) -> CallToolResult {
        tool_result(declarative_path(params.path).and_then(|path| api::effective_config(&path)))
    }

    /// 查询中心状态且不隐式启动中心。
    #[tool(description = "查询全局 Procora 中心状态，不会隐式启动它")]
    fn center_status(&self) -> CallToolResult {
        tool_result(api::center_status())
    }

    /// 列出持久托管服务且不隐式启动中心。
    #[tool(description = "列出全部持久托管服务，不会隐式启动中心")]
    fn list_services(&self) -> CallToolResult {
        tool_result(api::list_services())
    }

    /// 查询指定服务的持久化状态历史。
    #[tool(description = "查询指定 Procora 服务的状态历史")]
    fn service_history(&self, Parameters(params): Parameters<TargetParams>) -> CallToolResult {
        tool_result(api::service_history(&params.target))
    }

    /// 注册并启动一个声明式配置服务。
    #[tool(description = "注册并启动一个本地 Procora 声明式配置服务")]
    fn add_service(&self, Parameters(params): Parameters<PathParams>) -> CallToolResult {
        tool_result(declarative_path(params.path).and_then(api::add_service))
    }

    /// 对服务执行启动、重启或停止动作。
    #[tool(description = "启动、重启或停止指定 Procora 服务")]
    fn manage_service(&self, Parameters(params): Parameters<ManageParams>) -> CallToolResult {
        tool_result(api::manage_service(params.action.into(), &params.target))
    }

    /// 预览配置候选及其 Task 影响。
    #[tool(description = "只读预览配置候选修订及其 Task 影响")]
    fn preview_config(&self, Parameters(params): Parameters<TargetParams>) -> CallToolResult {
        tool_result(api::preview_config(&params.target))
    }

    /// 应用已经预览且内容没有变化的精确修订。
    #[tool(description = "应用先前预览且仍精确匹配的配置修订")]
    fn apply_config(&self, Parameters(params): Parameters<ApplyParams>) -> CallToolResult {
        tool_result(api::apply_config(&params.target, &params.revision))
    }

    /// 停止并移除服务注册，但不删除服务目录。
    #[tool(description = "停止并移除服务注册，不删除服务目录或配置")]
    fn remove_service(&self, Parameters(params): Parameters<TargetParams>) -> CallToolResult {
        tool_result(api::remove_service(&params.target))
    }
}

#[allow(
    missing_docs,
    reason = "rmcp prompt 宏生成的关联辅助函数无法单独添加文档"
)]
#[prompt_router]
impl ProcoraMcpServer {
    /// 返回完整 CLI 与中心服务器操作参考。
    #[prompt(
        name = "procora_cli_reference",
        description = "Procora CLI、中心服务器与服务生命周期的完整内嵌参考"
    )]
    async fn cli_reference(&self) -> GetPromptResult {
        documentation_prompt("CLI 与中心服务器参考", CLI_GUIDE)
    }

    /// 返回完整配置语义参考。
    #[prompt(
        name = "procora_configuration_reference",
        description = "Procora 配置格式、合并、profile、模板和来源语义的完整内嵌参考"
    )]
    async fn configuration_reference(&self) -> GetPromptResult {
        documentation_prompt("配置参考", CONFIGURATION_GUIDE)
    }

    /// 返回 Center、ServiceHost 与 Task 的运行时语义。
    #[prompt(
        name = "procora_runtime_reference",
        description = "Procora Center、ServiceHost、Task 和状态模型的完整内嵌参考"
    )]
    async fn runtime_reference(&self) -> GetPromptResult {
        documentation_prompt("运行时参考", RUNTIME_GUIDE)
    }
}

#[tool_handler(router = self.tool_router)]
#[prompt_handler(router = self.prompt_router)]
impl ServerHandler for ProcoraMcpServer {
    fn get_info(&self) -> ServerInfo {
        let capabilities = ServerCapabilities::builder()
            .enable_tools()
            .enable_prompts()
            .build();
        ServerInfo::new(capabilities)
            .with_server_info(
                Implementation::new("procora", env!("CARGO_PKG_VERSION"))
                    .with_title("Procora MCP")
                    .with_description("本地 Procora 配置与服务管理接口"),
            )
            .with_instructions(
                "优先读取匹配主题的 Prompt；修改服务前先调用只读工具确认目标和候选修订。",
            )
    }
}

/// 通过当前进程的标准输入输出运行 MCP 服务。
///
/// # Errors
///
/// 当 Tokio 运行时、MCP 握手、传输或服务关闭失败时返回错误。
pub fn run_stdio() -> anyhow::Result<()> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(anyhow::Error::from)?
        .block_on(serve_stdio())
}

/// 异步运行当前进程的 MCP stdio 服务。
///
/// # Errors
///
/// 当 MCP 握手、传输或服务关闭失败时返回错误。
pub async fn serve_stdio() -> anyhow::Result<()> {
    let service = ProcoraMcpServer::default()
        .serve(stdio())
        .await
        .map_err(anyhow::Error::from)?;
    service.waiting().await.map_err(anyhow::Error::from)?;
    Ok(())
}

/// 把程序化接口结果转换为 MCP 结构化成功或可见工具错误。
fn tool_result<T: Serialize>(result: anyhow::Result<T>) -> CallToolResult {
    match result.and_then(|value| serde_json::to_value(value).map_err(anyhow::Error::from)) {
        Ok(value) => CallToolResult::structured(value),
        Err(error) => CallToolResult::error(vec![ContentBlock::text(format!("{error:#}"))]),
    }
}

/// MCP 默认拒绝会执行可信代码的显式 Python 配置入口。
fn declarative_path(path: PathBuf) -> anyhow::Result<PathBuf> {
    if is_python_config(&path) {
        anyhow::bail!("MCP 不执行显式 procora.py；请在可信交互式终端中使用 Procora CLI");
    }
    Ok(path)
}

/// 把编译时内嵌文档包装成可直接注入会话的 Prompt。
fn documentation_prompt(title: &str, document: &str) -> GetPromptResult {
    GetPromptResult::new(vec![PromptMessage::new_text(
        Role::User,
        format!(
            "请以以下 Procora {title}为准回答和操作；若文档没有说明，请明确指出。\n\n{document}"
        ),
    )])
    .with_description(format!("Procora {title}"))
}
