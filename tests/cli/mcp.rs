//! MCP 工具、Prompts 与内嵌文档的端到端契约测试。

use std::path::PathBuf;

use procora::mcp::ProcoraMcpServer;
use rmcp::{
    ServiceExt,
    model::{CallToolRequestParams, GetPromptRequestParams},
};

/// 返回仓库根目录中的基础配置夹具。
fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/basic.yaml")
}

#[test]
// 服务通过真实MCP会话暴露工具和三份内嵌参考文档。
fn server_exposes_tools_and_embedded_documentation() {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let (server_transport, client_transport) = tokio::io::duplex(1024 * 1024);
            let server = tokio::spawn(async move {
                let running = ProcoraMcpServer::default().serve(server_transport).await?;
                running.waiting().await?;
                anyhow::Ok(())
            });
            let client = ().serve(client_transport).await.unwrap();

            let tools = client.list_all_tools().await.unwrap();
            for name in [
                "validate_project",
                "task_graph",
                "effective_config",
                "center_status",
                "list_services",
                "service_history",
                "add_service",
                "manage_service",
                "preview_config",
                "apply_config",
                "remove_service",
            ] {
                assert!(
                    tools.iter().any(|tool| tool.name == name),
                    "缺少工具 {name}"
                );
            }

            let validated = client
                .call_tool(
                    CallToolRequestParams::new("validate_project")
                        .with_arguments(rmcp::object!({ "path": fixture() })),
                )
                .await
                .unwrap();
            assert_eq!(validated.is_error, Some(false));
            let structured = validated.structured_content.unwrap();
            assert_eq!(structured["project"], "demo");
            assert_eq!(structured["task_count"], 2);

            let rejected = client
                .call_tool(
                    CallToolRequestParams::new("validate_project")
                        .with_arguments(rmcp::object!({ "path": "procora.py" })),
                )
                .await
                .unwrap();
            assert_eq!(rejected.is_error, Some(true));
            let error = rejected.content[0].as_text().unwrap();
            assert!(error.text.contains("MCP 不执行显式 procora.py"));

            let prompts = client.list_all_prompts().await.unwrap();
            for name in [
                "procora_cli_reference",
                "procora_configuration_reference",
                "procora_runtime_reference",
            ] {
                assert!(
                    prompts.iter().any(|prompt| prompt.name == name),
                    "缺少 Prompt {name}"
                );
            }
            let guide = client
                .get_prompt(GetPromptRequestParams::new("procora_cli_reference"))
                .await
                .unwrap();
            let text = guide.messages[0].content.as_text().unwrap();
            assert!(text.text.contains("# CLI 与全局 Procora 服务器语义"));
            assert!(text.text.contains("procora add"));

            client.cancel().await.unwrap();
            server.await.unwrap().unwrap();
        });
}
