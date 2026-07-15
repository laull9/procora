# TODO

本文记录近期可执行事项；长期里程碑和验收条件见 [`docs/roadmap.md`](docs/roadmap.md)。

## 已完成

- [x] 调研三平台进程、systemd、CLI、TUI、配置、监测、IPC 与文件监听依赖。
- [x] 建立分层 Cargo workspace 和统一依赖、lint、release 配置。
- [x] 建立 core、config、engine、process、monitor、log、platform 模块骨架。
- [x] 建立 source、storage、protocol、daemon、tui、cli 模块骨架。
- [x] 支持 YAML、TOML、JSON 到同一 `ProjectSpec` 的基础反序列化。
- [x] 实现任务标识校验、缺失依赖检查、DAG 环检测和启动/停止顺序。
- [x] 接入 POSIX 进程组与 Windows Job Object 的进程启动包装。
- [x] 接入 sysinfo 单进程资源快照和 notify 本地入口监听。
- [x] 提供 `validate`、`graph`、`doctor` 诊断命令及基础测试。
- [x] 建立 Linux-only systemd feature，提供 ready 通知和 unit 列表查询入口。
- [x] 将根包改为虚拟 workspace，并把 `procora` 二进制和 CLI 测试迁入 cli crate。
- [x] 以协议快照驱动 TUI，实现任务主从详情、直接依赖、日志未连接状态和紧凑布局。
- [x] 实现 TUI 的方向键/jk 选择、Tab/左右切页、数字直达和退出按键。
- [x] 固定 `Center → Service → Task` 三级模型，并实现合法服务名称校验。
- [x] 实现目录唯一合法配置发现、多个合法配置冲突和显式文件消歧。
- [x] 实现中心服务注册表、名称/路径定位、状态持久化与失败隔离恢复。
- [x] 使用本地 IPC 实现中心服务器单次请求闭环，并由 CLI 按需拉起后台中心进程。
- [x] 实现默认 `procora`、`server <path>`、`server list`、`show` 和服务启停/重启命令结构。
- [x] 实现无中心服务器时与 TUI 同生命周期的嵌入式 `ServiceHost`。
- [x] 使用 SQLite 保存服务注册、当前状态、错误信息、任务数和状态变更历史。
- [x] 将 Service/Task 日志保存在各自 Service 目录，实现按大小轮转、gzip 压缩和归档数量保留。
- [x] 实现 `init --config yaml/json/toml`、`up`、`down`、`status` 和 `server history`。
- [x] 将目录配置发现限制为 `procora.yaml/yml/toml/json`，忽略其他项目配置文件。
- [x] 实现 `enable/disable`，以 systemd user unit、LaunchAgent 或 Windows 登录任务托管 Center。

## P0：完成首个运行闭环

- [x] 将原始配置 DTO 与领域 `ProjectSpec` 分离，补默认值、路径解析和多错误聚合。
- [x] 为 YAML/TOML/JSON 补精确字段路径、行列号和无效配置夹具。
- [x] 为 `procora-process` 增加真实子进程、整树停止、迟到退出和输出排空测试。
- [x] 实现 Engine 命令/事件类型、单写者事件循环和带 `generation/run_id` 的身份校验。
- [x] 实现 `started`、`healthy`、`completed_successfully` 依赖条件调度。
- [x] 实现优雅停止、强制回收以及 `never/on-failure/always` 重启退避。
- [x] 把进程 stdout/stderr 接入有界日志管线和 TailBuffer。
- [x] 把日志游标和进程树资源指标映射到 TUI 协议页面。
- [x] 中心恢复时为 Task 创建新 `generation/run_id`，拒绝仅凭 PID 接管。
- [x] 把 `server start/restart/stop` 接入真实 Task 启停和 Ctrl-C 反向停止。
- [x] 为后台 Center 增加独立运行 tick，无观察客户端时仍推进退出、依赖与重启退避。
- [x] 修复顶层进程提前退出后的剩余进程树回收，并为输出管道排空设置有界等待。
- [x] 使用有界队列解耦 Task 管道读取与磁盘日志写入，慢磁盘不再直接反压任务。

## P1：后台模式与完整观察

P1 中可独立于真实 Task 进程运行时的基础设施已经完成；仍依赖 P0 的 Task 日志、资源和恢复身份接线统一列在 P0，避免把同一阻塞项重复记账。

- [x] 使用 interprocess 建立 Unix domain socket / Windows Named Pipe 的 JSON 行请求通道。
- [x] 实现协议握手、快照加有界增量事件、游标过期重同步和 TUI 自动重连。
- [x] 实现中心服务器显式单实例锁和正常关闭协议。
- [x] 为本地 IPC 增加 Unix 同用户校验、Windows 当前用户 DACL、帧大小与连接并发上限。
- [x] 关闭 Center 时先封闭新控制请求、等待在途请求，再释放单实例锁。
- [x] 将资源采样扩展为受管进程树聚合；根进程不存在时显式返回不可用。
- [x] 为现有 gzip 滚动文件补持久游标索引、按时间/总字节保留和轮转后 Gap 标记。
- [x] 将 Center/ServiceHost 实时状态接入 TUI；日志与资源接线由 P0 的真实 Task 运行时提供数据源。
- [x] 实现 TUI 服务启停/重启命令、完成反馈、失败提示和连接恢复。
- [x] 为 `show` 和默认 TUI 实现增量前端会话与控制权限协商。

## P2：热更新与平台集成

- [ ] 对 notify 事件去抖，重读 include 闭包并生成语义差异。
- [ ] 实现配置候选修订、预览、确认、按影响范围重启和失败保留旧修订。
- [ ] 通过受控 Python 子进程支持 Python 配置，不嵌入 Python 运行时。
- [ ] 设计并实现 Git 任务定义源，远程内容默认只产生待确认候选。
- [ ] 在 Linux CI 验证 `--features systemd`，补用户/系统总线和权限错误测试。
- [x] 生成 Center 的 systemd user unit，并统一接入 macOS LaunchAgent 与 Windows 登录任务。
- [x] 建立 3 平台 × 2 架构的 GitHub Releases 二进制流水线和 macOS/Linux shell、Windows PowerShell 安装脚本。

## 工程质量

- [ ] 加入 cargo-deny、cargo-audit 和依赖许可证策略。
- [ ] 加入配置/状态机性质测试与高速日志压力测试。
- [ ] 固定最低 Rust 版本 CI，升级 sysinfo 等依赖时检查 MSRV。
- [ ] 为公共配置模式、IPC 协议和持久化格式建立 ADR 与兼容性策略。
