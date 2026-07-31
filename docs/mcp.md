# MCP 本地服务

## 1. 入口与传输

`procora mcp` 通过当前进程的标准输入输出运行 MCP 服务，适合由编辑器、智能体或其他本地 MCP 客户端作为子进程启动。它只提供 stdio，不监听 TCP、HTTP 或 SSE，也不改变 Procora 现有的当前用户 IPC 与权限边界。

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

建议连接后先读取 `procora_mcp_reference` Prompt，再按“先读、后预检、最后写”的顺序调用工具。客户端应把工具的 `structuredContent` 当作事实来源，不需要解析人类终端文本。

## 2. 快速选择

| 想做什么 | 推荐工具链 |
| --- | --- |
| 检查一个本地配置能否加载 | `validate_project` |
| 看最终默认值和二进制矩阵 | `effective_config` |
| 修改正在运行的配置 | `preview_config → apply_config` |
| 把完整Service部署到裸机 | `preview_deploy → deploy_service` |
| 构建并验证 Service 包 | `build_package → verify_package` |
| 审计或恢复已安装包 | `list_installed_packages`、`rollback_package`、`recover_package` |
| 只启停本地已注册Service | `manage_service` |
| 排查历史状态 | `service_history` |

两个写工作流都使用 revision，但含义不同：配置 revision 固定 Center 读到的配置候选；部署 revision 固定本地归档、远端 SSH 目标、探测平台、二进制选择和验收参数。不要在两个工作流之间混用。

## 3. 工具

工具复用 `procora::cli::api`，不会通过捕获终端文本来模拟 CLI。成功结果同时提供 JSON 结构化内容和 JSON 文本；业务失败以 MCP 工具错误返回。

| 工具 | 行为 | 远端/本地副作用 |
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
| `build_package` | 构建确定性 `.pcpkg` | 是，写入本机包文件 |
| `inspect_package` | 读取包清单与逻辑摘要 | 否 |
| `verify_package` | 流式验证清单和全部 Blob | 否 |
| `extract_package` | 为具体平台物化到新目录 | 是，创建本机目录 |
| `install_package` | 安装不可变 release，验活失败自动回滚 | 是 |
| `list_installed_packages` | 列出包、release 与 pending 状态 | 否 |
| `rollback_package` | 切换到历史 release 并验活 | 是 |
| `recover_package` | 收敛中断的 pending 切换 | 可能 |
| `uninstall_package` | 解除注册；`purge` 可清理安装数据 | 是 |
| `preview_deploy` | 校验本地Service，SSH探测平台，选择二进制并构造归档摘要 | 只读连接远端；不上传、不切换Service |
| `deploy_service` | 复核预检revision，上传完整Service、验活并按需回滚 | 是，修改指定SSH远端的托管Service |

修改配置的推荐顺序是 `preview_config → 人工/智能体检查 revision 与 diff → apply_config`。`apply_config` 仍执行与 CLI 相同的 TOCTOU 修订校验，不能绕过预览确认。

### 3.1 裸机部署参数

`preview_deploy` 接受：

| 参数 | 必填 | 默认 | 说明 |
| --- | --- | --- | --- |
| `source` | 否 | `.` | 本机Service目录或显式声明式配置 |
| `ssh` | 是 | — | SSH config别名或`user@host` |
| `remote_bin` | 否 | `procora`并检查常见位置 | 远端Procora命令或无空格Unicode路径，例如`C:/工具/Procora/procora.exe` |
| `service` | 否 | — | 对配置`project`的身份断言 |
| `timeout_ms` | 否 | `30000` | 新release验收总时限，最大600000 |
| `stable_for_ms` | 否 | `2000` | 持续可用后才确认成功 |
| `keep` | 否 | `3` | 保留release数，范围1–32 |
| `allow_python` | 否 | `false` | 明确允许执行可信的 `procora.py` |

`allow_python` 同样适用于 `build_package` 和 `install_package`。部署或安装以 `procora.py` 为配置入口的 `.pcpkg` 时也必须显式授权，不能用预先打包绕过信任门。

`deploy_service` 接受相同参数，并额外要求 `revision`。两次调用的 `source`、`ssh`、`remote_bin`、`service`、`timeout_ms`、`stable_for_ms` 和 `keep` 必须保持一致。Procora 会重新构造预检；文件、平台、选择结果或参数有任何变化都会拒绝旧 revision，不会“尽量继续”。

Windows 中文 `source` 路径通过 JSON 的 UTF-8 文本还原为本机 UTF-16 路径，不依赖 MCP 客户端或部署机的活动代码页。SSH、`cmd.exe` 或计划任务返回的人工诊断若不是 UTF-8，会按 GB18030 兜底显示；MCP 消息、能力握手和部署协议仍严格要求 UTF-8，非 UTF-8 机器响应会明确报错。

### 3.2 两阶段调用示例

第一步只读预检：

```json
{
  "name": "preview_deploy",
  "arguments": {
    "source": "/workspace/api",
    "ssh": "prod",
    "service": "api",
    "timeout_ms": 45000,
    "stable_for_ms": 5000,
    "keep": 5
  }
}
```

重点检查结构化结果中的：

```json
{
  "project": "api",
  "target_platform": {
    "os": "linux",
    "arch": "x86_64",
    "environment": "gnu"
  },
  "binaries": [
    {
      "name": "api",
      "selector": "linux-x86_64",
      "source": "/workspace/api/dist/api-linux-amd64",
      "target": "bin/api",
      "bytes": 12345678,
      "sha256": "..."
    }
  ],
  "archive_sha256": "...",
  "revision": "..."
}
```

人工或调用方确认 project、SSH 目标、平台、每个 source→target、验收窗口和保留数量后，原样传回 revision：

```json
{
  "name": "deploy_service",
  "arguments": {
    "source": "/workspace/api",
    "ssh": "prod",
    "service": "api",
    "timeout_ms": 45000,
    "stable_for_ms": 5000,
    "keep": 5,
    "revision": "<preview_deploy返回的完整revision>"
  }
}
```

成功结果包含 `project`、新 `release`、可选 `previous_release`、完整 `preview` 以及结构化 `events`。`events[].phase` 包含本地 `preflight`/`binary`/`archive` 和远端校验、切换、验活、恢复、回滚阶段；客户端可直接展示它们，不应从 stderr 猜测部署状态。

### 3.3 MCP部署为什么只支持非交互SSH

MCP 客户端可能在没有控制终端的编辑器或后台进程中运行，因此部署固定使用 batch 语义：

- 不接受密码参数，也不把密码写入配置、日志或工具结果。
- 不弹出主机指纹确认；首次连接应由人在普通SSH/CLI中完成。
- 使用 SSH key、agent、known_hosts 和 SSH config。
- 找不到远端 Procora 时会检查常见位置；仍找不到则要求显式 `remote_bin`。
- 预检会建立只读SSH连接并在本机生成临时归档，但不会发送归档或修改远端Service。

## 4. 内嵌 Prompts

以下 Prompt 文本使用 `include_str!` 编译进 Procora 二进制，因此不依赖安装目录旁存在源码文档：

| Prompt | 内嵌来源 | 用途 |
| --- | --- | --- |
| `procora_cli_reference` | `docs/cli.md` | CLI、中心、服务定位和生命周期 |
| `procora_configuration_reference` | `docs/configuration.md` | 配置格式、合并、profile、模板和来源 |
| `procora_runtime_reference` | `docs/runtime.md` | Center、ServiceHost、Task 与状态模型 |
| `procora_mcp_reference` | `docs/mcp.md` | MCP参数、安全边界和两阶段裸机部署 |

客户端应优先获取与当前问题匹配的 Prompt，再选择工具。文档随二进制版本固定，避免 MCP 客户端按新文档误操作旧版本 Procora。

## 5. 安全边界

- MCP 服务只应交给可信的本地客户端启动；生命周期工具具有启动和停止本机任务的能力。
- `deploy_service` 还能修改SSH远端、启动远端Task并清理超出`keep`的旧release；调用前必须展示并确认预检结果。
- Python 入口会以当前用户权限执行可信代码。所有接收路径的配置、构建和部署工具默认拒绝显式入口及目录中的 `procora.py`；只有调用方展示风险并传入 `allow_python: true` 后才执行。
- `uninstall_package` 的 `purge: true` 会永久清理对应包托管目录；调用前应先用 `list_installed_packages` 核对 Service 与 release。
- `remove_service` 只删除中心注册，不删除配置或服务目录；MCP 不暴露 `clean`、`deps`、TUI 和自启动管理入口。

## 6. 常见错误与恢复

- `allow_python=true`：默认信任门拒绝了 Python 入口；检查脚本后显式授权，或改用声明式配置。
- `部署预检修订已经变化`：重新调用 `preview_deploy`，检查差异后使用新 revision。
- `SSH 自动登录失败`：在启动 MCP 客户端的同一用户环境检查 `ssh prod`、agent 和 known_hosts。
- `没有适用于远端平台的变体`：读取错误中的规范化平台键，并在 `binaries.*.variants` 补齐。
- `远端全托管部署协议不兼容`：升级远端 Procora；MCP 不做不安全降级。
- 新release启动或健康检查失败：同一次工具调用会完成自动回滚；查看返回的 `events` 判断新release失败和旧release恢复分别停在哪个阶段。
