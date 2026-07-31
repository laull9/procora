//! MCP 工具、Prompts 与内嵌文档的端到端契约测试。

use std::path::PathBuf;

use procora::mcp::ProcoraMcpServer;
#[cfg(unix)]
use rmcp::transport::{ConfigureCommandExt, TokioChildProcess};
use rmcp::{
    ServiceExt,
    model::{CallToolRequestParams, GetPromptRequestParams},
};

/// 返回仓库根目录中的基础配置夹具。
fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/basic.yaml")
}

#[cfg(unix)]
/// 写入`MCP`部署测试使用的`Linux`二进制`Service`。
fn write_mcp_deploy_service(source: &std::path::Path) {
    std::fs::create_dir_all(source.join("dist")).unwrap();
    std::fs::write(
        source.join("procora.yaml"),
        r"version: 1
project: demo
binaries:
  api:
    target: bin/api
    variants:
      linux-amd64: dist/api-linux
tasks: {}
",
    )
    .unwrap();
    std::fs::write(source.join("dist/api-linux"), b"linux-binary").unwrap();
}

#[test]
#[cfg(unix)]
// MCP通过真实stdio子进程完成预检修订确认和无target部署且不污染协议stdout。
fn mcp_preview_and_deploy_share_the_managed_deployment_core() {
    use std::fs;

    use crate::{
        cli_uploads::{install_fake_ssh, temporary_directory},
        command_support::remove_directory_when_released,
    };

    let directory = temporary_directory("mcp-deploy");
    install_fake_ssh(&directory);
    let source = directory.join("service");
    write_mcp_deploy_service(&source);
    let path = format!(
        "{}:{}",
        directory.display(),
        std::env::var("PATH").unwrap_or_default()
    );

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let transport = TokioChildProcess::new(
                tokio::process::Command::new(env!("CARGO_BIN_EXE_procora")).configure(|command| {
                    command
                        .arg("mcp")
                        .env("PATH", path)
                        .env("FAKE_SSH_LOG", directory.join("ssh.log"))
                        .env("FAKE_SSH_HEADER_LOG", directory.join("ssh-header.log"));
                }),
            )
            .unwrap();
            let client = ().serve(transport).await.unwrap();
            let arguments = rmcp::object!({
                "source": source,
                "ssh": "mock-host",
                "timeout_ms": 1000,
                "stable_for_ms": 0,
                "keep": 2
            });
            let preview = client
                .call_tool(
                    CallToolRequestParams::new("preview_deploy").with_arguments(arguments.clone()),
                )
                .await
                .unwrap();
            assert_eq!(preview.is_error, Some(false));
            let preview = preview.structured_content.unwrap();
            assert_eq!(preview["target_platform"]["os"], "linux");
            assert_eq!(preview["binaries"][0]["target"], "bin/api");
            let revision = preview["revision"].as_str().unwrap();

            let deployed = client
                .call_tool(CallToolRequestParams::new("deploy_service").with_arguments(
                    rmcp::object!({
                        "source": source,
                        "ssh": "mock-host",
                        "revision": revision,
                        "timeout_ms": 1000,
                        "stable_for_ms": 0,
                        "keep": 2
                    }),
                ))
                .await
                .unwrap();
            assert_eq!(deployed.is_error, Some(false));
            let deployed = deployed.structured_content.unwrap();
            assert_eq!(deployed["project"], "demo");
            assert_eq!(deployed["release"], "0123456789abcdef");
            assert_eq!(deployed["preview"]["revision"], revision);
            assert!(
                deployed["events"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|event| event["phase"] == "binary")
            );

            fs::write(source.join("changed-after-preview.txt"), "new revision").unwrap();
            let stale = client
                .call_tool(CallToolRequestParams::new("deploy_service").with_arguments(
                    rmcp::object!({
                        "source": source,
                        "ssh": "mock-host",
                        "revision": revision,
                        "timeout_ms": 1000,
                        "stable_for_ms": 0,
                        "keep": 2
                    }),
                ))
                .await
                .unwrap();
            assert_eq!(stale.is_error, Some(true));
            assert!(
                stale.content[0]
                    .as_text()
                    .unwrap()
                    .text
                    .contains("部署预检修订已经变化")
            );
            client.cancel().await.unwrap();
        });

    let invocations = fs::read_to_string(directory.join("ssh.log")).unwrap();
    assert_eq!(invocations.matches("__receive-deploy").count(), 1);
    remove_directory_when_released(&directory);
}

#[test]
#[cfg(unix)]
// MCP包部署的重复预检完全一致，且预检revision可直接确认部署。
fn mcp_package_preview_revision_is_stable_for_deploy() {
    use std::fs;

    use crate::{
        cli_uploads::{install_fake_ssh, temporary_directory},
        command_support::remove_directory_when_released,
    };

    let directory = temporary_directory("mcp-package-deploy");
    install_fake_ssh(&directory);
    let source = directory.join("service");
    write_mcp_deploy_service(&source);
    let package = directory.join("demo.pcpkg");
    let built = std::process::Command::new(env!("CARGO_BIN_EXE_procora"))
        .args([
            "package",
            "build",
            source.to_str().unwrap(),
            "--output",
            package.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );
    let path = format!(
        "{}:{}",
        directory.display(),
        std::env::var("PATH").unwrap_or_default()
    );

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let transport = TokioChildProcess::new(
                tokio::process::Command::new(env!("CARGO_BIN_EXE_procora")).configure(|command| {
                    command
                        .arg("mcp")
                        .env("PATH", path)
                        .env("FAKE_SSH_LOG", directory.join("ssh.log"))
                        .env("FAKE_SSH_HEADER_LOG", directory.join("ssh-header.log"));
                }),
            )
            .unwrap();
            let client = ().serve(transport).await.unwrap();
            let arguments = rmcp::object!({
                "source": package,
                "ssh": "mock-host",
                "timeout_ms": 1000,
                "stable_for_ms": 0,
                "keep": 2
            });
            let first = client
                .call_tool(
                    CallToolRequestParams::new("preview_deploy").with_arguments(arguments.clone()),
                )
                .await
                .unwrap();
            assert_eq!(first.is_error, Some(false));
            let first = first.structured_content.unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(1_100)).await;
            let repeated = client
                .call_tool(CallToolRequestParams::new("preview_deploy").with_arguments(arguments))
                .await
                .unwrap();
            assert_eq!(repeated.structured_content.as_ref(), Some(&first));

            let revision = first["revision"].as_str().unwrap();
            let deployed = client
                .call_tool(CallToolRequestParams::new("deploy_service").with_arguments(
                    rmcp::object!({
                        "source": package,
                        "ssh": "mock-host",
                        "revision": revision,
                        "timeout_ms": 1000,
                        "stable_for_ms": 0,
                        "keep": 2
                    }),
                ))
                .await
                .unwrap();
            assert_eq!(deployed.is_error, Some(false));
            assert_eq!(
                deployed.structured_content.unwrap()["preview"]["revision"],
                revision
            );
            client.cancel().await.unwrap();
        });

    let invocations = fs::read_to_string(directory.join("ssh.log")).unwrap();
    assert_eq!(invocations.matches("__receive-deploy").count(), 1);
    remove_directory_when_released(&directory);
}

#[test]
// 服务通过真实MCP会话暴露工具和四份内嵌参考文档。
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
                "build_package",
                "inspect_package",
                "verify_package",
                "extract_package",
                "install_package",
                "list_installed_packages",
                "rollback_package",
                "recover_package",
                "uninstall_package",
                "preview_deploy",
                "deploy_service",
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
            assert!(error.text.contains("allow_python=true"));

            let prompts = client.list_all_prompts().await.unwrap();
            for name in [
                "procora_cli_reference",
                "procora_configuration_reference",
                "procora_runtime_reference",
                "procora_mcp_reference",
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

#[test]
// MCP包工具共享确定性构建、校验与安全物化实现。
fn mcp_package_tools_build_verify_and_extract() {
    use std::fs;

    use crate::{command_support::remove_directory_when_released, temporary_directory};

    let directory = temporary_directory("mcp-package-tools");
    let service = directory.join("service");
    fs::create_dir(&service).unwrap();
    fs::write(
        service.join("procora.yaml"),
        "version: 1\nproject: mcp-package\ntasks: {}\n",
    )
    .unwrap();
    fs::write(service.join("asset.txt"), "content").unwrap();
    let package = directory.join("mcp-package.pcpkg");
    let extracted = directory.join("extracted");

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

            let built = client
                .call_tool(CallToolRequestParams::new("build_package").with_arguments(
                    rmcp::object!({
                        "source": service,
                        "output": package,
                        "platform": "all"
                    }),
                ))
                .await
                .unwrap();
            assert_eq!(built.is_error, Some(false));
            assert_eq!(built.structured_content.unwrap()["project"], "mcp-package");

            for tool in ["inspect_package", "verify_package"] {
                let result = client
                    .call_tool(
                        CallToolRequestParams::new(tool)
                            .with_arguments(rmcp::object!({ "package": package })),
                    )
                    .await
                    .unwrap();
                assert_eq!(result.is_error, Some(false), "工具 {tool} 失败");
                assert_eq!(
                    result.structured_content.unwrap()["manifest"]["project"],
                    "mcp-package"
                );
            }

            let result = client
                .call_tool(
                    CallToolRequestParams::new("extract_package").with_arguments(rmcp::object!({
                        "package": package,
                        "output": extracted,
                        "platform": "current"
                    })),
                )
                .await
                .unwrap();
            assert_eq!(result.is_error, Some(false));
            assert!(extracted.join("asset.txt").is_file());

            client.cancel().await.unwrap();
            server.await.unwrap().unwrap();
        });

    remove_directory_when_released(&directory);
}
