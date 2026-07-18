# MCP 本地服务

## 1. 入口与传输

`procora mcp` 通过当前进程的标准输入输出运行 MCP 服务，适合由编辑器、智能体或其他本地 MCP 客户端作为子进程启动。第一版只提供 stdio，不监听 TCP、HTTP 或 SSE，也不改变 Procora 现有的当前用户 IPC 与权限边界。

通用客户端配置形态如下；实际配置文件位置和外层字段名以客户端文档为准：

```json
{
  "mcpServers": {
    "procora": {
      "command": "procora",
      "args": ["mcp"]
    }
  }
}
```

MCP 协议内容只写入 stdout，诊断写入 stderr。直接在终端运行时服务会持续等待输入，这属于正常行为。

## 2. 工具

工具复用 `procora::cli::api`，不会通过捕获终端文本来模拟 CLI。成功结果同时提供 JSON 结构化内容和 JSON 文本；业务失败以 MCP 工具错误返回。

| 工具 | 行为 | 是否可能改变状态 |
| --- | --- | --- |
| `validate_project` | 完整校验声明式配置 | 否 |
| `task_graph` | 返回确定性 Task 启动顺序 | 否 |
| `effective_config` | 返回默认值、来源和规范化路径展开后的配置 | 否 |
| `center_status` | 查询中心状态；离线时不启动 | 否 |
| `list_services` | 列出托管服务；离线时不启动 | 否 |
| `service_history` | 查询服务状态历史 | 否 |
| `add_service` | 注册并启动服务 | 是 |
| `manage_service` | 启动、重启或停止服务 | 是 |
| `preview_config` | 预览候选修订和 Task 影响 | 否 |
| `apply_config` | 应用已预览且仍精确匹配的修订 | 是 |
| `remove_service` | 停止并移除注册，不删除服务目录 | 是 |

修改配置的推荐顺序是 `preview_config → 人工/智能体检查 revision 与 diff → apply_config`。`apply_config` 仍执行与 CLI 相同的 TOCTOU 修订校验，不能绕过预览确认。

## 3. 内嵌 Prompts

以下 Prompt 文本使用 `include_str!` 编译进 Procora 二进制，因此不依赖安装目录旁存在源码文档：

| Prompt | 内嵌来源 | 用途 |
| --- | --- | --- |
| `procora_cli_reference` | `docs/cli.md` | CLI、中心、服务定位和生命周期 |
| `procora_configuration_reference` | `docs/configuration.md` | 配置格式、合并、profile、模板和来源 |
| `procora_runtime_reference` | `docs/runtime.md` | Center、ServiceHost、Task 与状态模型 |

客户端应优先获取与当前问题匹配的 Prompt，再选择工具。文档随二进制版本固定，避免 MCP 客户端按新文档误操作旧版本 Procora。

## 4. 安全边界

- MCP 服务只应交给可信的本地客户端启动；生命周期工具具有启动和停止本机任务的能力。
- MCP 工具拒绝显式 `procora.py`，因为加载它会以当前用户权限执行可信代码。确需使用时，应在交互式终端中运行 CLI 并确认警告。
- 目录发现仍只扫描声明式的 `procora.yaml`、`procora.yml`、`procora.toml` 和 `procora.json`，不会自动执行目录中的 Python 文件。
- `remove_service` 只删除中心注册，不删除配置或服务目录；MCP 不暴露 `clean`、`deps`、TUI 和自启动管理入口。
