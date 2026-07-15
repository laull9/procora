# Procora

Procora 是一个以 TUI 为主要入口的本机任务服务管理器。它采用固定的三级模型：一个当前用户级中心服务器托管多个服务，每个服务由“目录 + 唯一配置文件”定义，每个服务再管理自己的 Task 依赖图。

```text
Procora Center（每个本机用户一个）
  ├── Service: api（目录 + 配置）
  │     ├── Task: database
  │     └── Task: backend
  └── Service: worker（目录 + 配置）
        └── Task: queue
```

项目当前处于早期开发阶段，但首个运行闭环已经完成：配置会被规范化为任务图，真实 Task 由进程组或 Job Object 托管，依赖调度、重启退避、反向停止、日志续读和进程树资源快照已接入 CLI/TUI。Service 的 `running` 仍表示宿主已加载并具有运行期望；单个 Task 的实际状态在 TUI 中独立展示。

## 安装

发布版本提供 Linux、macOS、Windows 各自的 x86_64 与 ARM64 二进制，不维护平台原生安装包。

macOS/Linux：

```bash
curl --fail --location --proto '=https' --tlsv1.2 https://raw.githubusercontent.com/laull/procora/main/scripts/install.sh | sh
```

Windows PowerShell：

```powershell
irm https://raw.githubusercontent.com/laull/procora/main/scripts/install.ps1 | iex
```

安装器会校验 Release 归档的 SHA-256。可通过 `PROCORA_VERSION` 固定标签，通过 `PROCORA_INSTALL_DIR` 修改安装目录；完整发布矩阵见[发布说明](docs/release.md)。

## 状态与日志存储

- 当前用户的结构化状态保存在 `PROCORA_HOME/procora.sqlite3`，包括服务注册、当前状态、运行期望、错误信息、任务数和状态变更历史。
- 日志正文不进入 SQLite，也不由 Center 集中保存。
- 每个 Service 的日志保存在自己的 `<service>/.procora/logs/service.log`。
- 每个 Task 的日志保存在 `<service>/.procora/logs/tasks/<task>.log`。
- 活动日志达到大小阈值后自动轮转为 `.gz`，并按归档数量、归档总字节和时间策略清理旧文件。
- 每个活动日志旁保存文件代次与字节游标；客户端落后到跨越轮转边界时会收到 Gap 标记，并从当前可用尾部恢复。

## 高频命令

| 命令 | 语义 |
| --- | --- |
| `procora init --config yaml/json/toml` | 在当前目录创建对应格式的示例服务配置；默认不覆盖已有文件。 |
| `procora up` | 显式启动当前用户唯一的中心后台服务器，并显示实例与协议信息。 |
| `procora down` | 通过正常关闭协议停止中心后台服务器；不会删除 SQLite 状态或各服务日志。 |
| `procora status` | 查看中心服务器状态，不会因查询而自动启动它。 |
| `procora enable` | 使用当前平台的用户级原生托管器注册 Center，并立即启动；以后在用户登录时自动运行。 |
| `procora disable` | 正常停止 Center，并移除用户级原生自启动注册；保留服务状态和日志。 |
| `procora` | 在当前目录打开 TUI。若中心服务器存在则连接或注册当前服务；否则创建与 TUI 同生命周期的嵌入式宿主。 |
| `procora server <path>` | 发现目标目录中的唯一合法配置，或加载显式配置文件；必要时启动中心服务器，并注册、启动该服务宿主。 |
| `procora server list` | 列出中心服务器登记的服务名称、运行状态、任务数、目录和配置文件。 |
| `procora server history <name/path>` | 从 SQLite 查询指定服务的状态变更历史。 |
| `procora show <name/path>` | 按名称、服务目录或配置文件打开已注册服务的 TUI。 |
| `procora server start <name/path>` | 重新加载已注册配置并启动服务宿主。 |
| `procora server restart <name/path>` | 重新加载配置并重启服务宿主。 |
| `procora server stop <name/path>` | 停止服务宿主并保留注册信息。 |

`validate`、`graph` 和 `doctor` 作为低频诊断命令继续保留。

连接中心服务器后的 TUI 是可交互前端，而不是只读观察器：`s` 启动服务、`x` 停止服务、`r` 重新加载并重启服务；界面使用事件游标自动刷新，并在操作失败或连接中断时保留当前视图和显示提示。

## 配置发现

`<path>` 可以是 YAML、TOML、JSON 配置文件，也可以是服务目录：

- 显式文件始终只加载该文件，可用于消除目录歧义。
- 目录模式只扫描第一层的 `procora.yaml`、`procora.yml`、`procora.toml`、`procora.json`，不会读取 `package.json` 或其他业务配置。
- 候选必须通过完整 Procora 结构、语义和任务图校验，才算合法配置。
- 恰好一个合法配置时自动选择；多个合法配置时报错；没有合法配置时汇总候选失败原因。
- 配置顶层 `project` 是本机服务稳定名称，只允许 ASCII 字母、数字、点、短横线和下划线。

最小配置：

```yaml
version: 1
project: demo

tasks:
  database:
    command: postgres
  api:
    command: cargo
    args: ["run", "-p", "api"]
    depends_on:
      database:
        condition: started
    restart: on-failure
    restart_delay_ms: 500
    shutdown_timeout_ms: 5000
```

## 代码边界

- `procora-core`：服务名称、Task 标识、项目规范和依赖图。
- `procora-config`：格式解析、配置发现、完整校验和图编译。
- `procora-engine`：单服务内部的 Task 状态与调度规则。
- `procora-daemon`：中心服务器、多个 `ServiceHost`、名称/路径解析和本地 IPC。
- `procora-storage`：SQLite 中心注册表、服务当前状态和状态历史。
- `procora-log`：服务目录内的 Service/Task 文件日志、gzip 轮转和内存尾部缓存。
- `procora-protocol`：CLI/TUI 与中心服务器之间的稳定 DTO。
- `procora-cli`：命令解析、中心进程拉起和 TUI 连接生命周期。
- `procora-tui`：服务及其 Task 的终端视图。

完整设计从[文档索引](docs/README.md)开始；近期实现事项见 [TODO](TODO.md)。

## 开发验证

```bash
cargo fmt --all --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

代码注释和设计文档统一使用中文。关键 trait、结构体、函数和静态变量需要简短注释，单个代码文件不超过 500 行，关键行为测试统一放在各 crate 的 `tests/` 目录。
