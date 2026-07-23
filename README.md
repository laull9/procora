<div align="center">

# Procora: 一个带TUI、跨平台的进程与服务管理工具

面向本机开发环境的任务编排与服务管理器<br>
用一个跨平台 TUI 管理多个项目、任务依赖、进程树和日志

[![CI](https://github.com/laull9/procora/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/laull9/procora/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/laull9/procora?display_name=tag&sort=semver)](https://github.com/laull9/procora/releases/latest)
[![Downloads](https://img.shields.io/github/downloads/laull9/procora/total)](https://github.com/laull9/procora/releases)
[![Rust](https://img.shields.io/badge/Rust-1.95%2B-dea584?logo=rust)](https://www.rust-lang.org/)
[![Platforms](https://img.shields.io/badge/platform-Linux%20%7C%20macOS%20%7C%20Windows-blue)](#安装)

</div>

Procora 把每个项目视为一个 Service，把项目中的进程视为具有依赖关系的 Task。一个用户级 Center 可以同时托管多个 Service；CLI 与 TUI 共用同一套状态、日志和生命周期控制。


## 核心能力

- **声明式任务图**：使用 YAML、TOML 或 JSON 描述任务、依赖、环境变量、健康检查和重启策略。
- **真实进程托管**：Linux/macOS 使用进程组，Windows 使用 Job Object，停止时回收完整进程树。
- **终端优先**：在 TUI 中查看状态、资源、彩色日志，并启动、停止、重启或编辑服务。
- **可靠热更新**：先预览配置修订，再按受影响的下游闭包应用，失败时保留旧的有效定义。
- **多项目 Center**：统一注册本机服务，支持用户级开机自启动、历史状态和本地 IPC。
- **自动化接口**：提供脚本友好的 CLI、Shell 补全和 stdio MCP 服务。

```text
Procora Center
  ├── api-service
  │     ├── database
  │     └── backend ── depends_on: database
  └── worker-service
        └── queue
```

## 安装

Release 提供 Linux、macOS、Windows 的 x86_64 与 ARM64 二进制，安装脚本会校验归档的 SHA-256。

macOS / Linux：

```bash
curl --fail --location --proto '=https' --tlsv1.2 \
  https://raw.githubusercontent.com/laull9/procora/main/scripts/install.sh | sh
```

Windows PowerShell：

```powershell
irm https://raw.githubusercontent.com/laull9/procora/main/scripts/install.ps1 | iex
```

默认安装位置为：

| 平台 | 路径 |
| --- | --- |
| macOS / Linux | `$HOME/.local/bin/procora` |
| Windows | `%LOCALAPPDATA%\Procora\bin\procora.exe` |

脚本不会修改 `PATH`。如果安装后找不到命令，请按安装器提示把目录加入 `PATH`。

可通过环境变量自定义安装：

| 变量 | 用途 | 默认值 |
| --- | --- | --- |
| `PROCORA_VERSION` | 固定 Release 标签，如 `v0.5.1` | `latest` |
| `PROCORA_INSTALL_DIR` | 修改安装目录 | 见上表 |
| `PROCORA_REPO` | 指向 fork 的 `owner/repo` | `laull9/procora` |

例如安装指定版本：

```bash
curl --fail --location --proto '=https' --tlsv1.2 \
  https://raw.githubusercontent.com/laull9/procora/main/scripts/install.sh |
  PROCORA_VERSION=v0.5.1 sh
```

### 卸载

卸载器会先停用 Procora 开机自启动，再删除安装目录中的命令；数据库、运行状态和各 Service 日志默认保留。

macOS / Linux：

```bash
curl --fail --location --proto '=https' --tlsv1.2 \
  https://raw.githubusercontent.com/laull9/procora/main/scripts/uninstall.sh | sh
```

Windows PowerShell：

```powershell
irm https://raw.githubusercontent.com/laull9/procora/main/scripts/uninstall.ps1 | iex
```

自定义过安装目录时，卸载也需传入相同的 `PROCORA_INSTALL_DIR`。只有确认无需保留后台托管时，才使用 `PROCORA_FORCE_UNINSTALL=1` 跳过停用失败。

## 快速开始

```bash
mkdir procora-demo && cd procora-demo
procora init --config yaml --no-edit
procora add .
procora
```

`procora init` 创建可直接运行的最小配置；`procora add .` 注册并启动当前服务；不带参数运行 `procora` 会打开全局 TUI。

## 常用命令

| 命令 | 作用 |
| --- | --- |
| `procora init` | 创建最小配置 |
| `procora edit [path]` | 在 TUI 中编辑并校验配置 |
| `procora add <path>` | 注册并启动服务 |
| `procora` | 打开全部服务的总览 TUI |
| `procora show <name/path>` | 打开指定服务 |
| `procora list` | 列出已注册服务 |
| `procora start/stop/restart <name>` | 控制服务生命周期 |
| `procora logs <name> <task>` | 查看、搜索或清理 Task 日志 |
| `procora preview <name>` | 预览配置变更及影响范围 |
| `procora apply <name> <revision>` | 应用已确认的配置修订 |
| `procora enable/disable` | 启用或停用用户级开机自启动 |
| `procora completions <shell>` | 生成 Shell 补全 |
| `procora mcp` | 启动 stdio MCP 服务 |

运行 `procora --help` 查看全部命令。配置支持 profile、模板、include、`env_file`、管理依赖、远端上传和 Git 来源，详细语法见[配置说明](docs/configuration.md)与[完整示例](docs/example.md)。

## 数据与日志

- Center 状态保存在当前用户的 `PROCORA_HOME/procora.sqlite3`。
- Service 日志位于 `<service>/.procora/logs/service.log`。
- Task 日志位于 `<service>/.procora/logs/tasks/<task>.log`。
- 日志自动轮转为 gzip，并按数量、总大小和时间清理。
- `procora clean <path>` 清理指定 Service 的运行文件、日志和依赖缓存。

## 文档

| 文档 | 内容 |
| --- | --- |
| [文档索引](docs/README.md) | 架构、运行模型、平台适配与测试策略 |
| [CLI 与 Center](docs/cli.md) | 服务发现、定位和生命周期 |
| [配置说明](docs/configuration.md) | 配置格式、任务图与热更新 |
| [发布与安装](docs/release.md) | 支持平台、产物与发布流程 |
| [安全策略](SECURITY.md) | 信任边界与漏洞报告 |