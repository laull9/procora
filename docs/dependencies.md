# 外部依赖选型

## 1. 调研范围

本次选型于 2026-07-15 核对 crates.io、docs.rs、上游仓库和 systemd 官方接口文档，重点考察三平台支持、维护状态、Rust 版本、异步模型、依赖体积和平台降级方式。

骨架的最低 Rust 版本定为 1.95，原因是当前 `sysinfo 0.39` 的最低版本要求。仓库开发工具链可以更新，但提升最低版本需要在变更说明中明确记录。

## 2. 已接入依赖

| 领域 | 选型 | 接入位置 | 决策依据 |
| --- | --- | --- | --- |
| 异步运行时 | [`tokio`](https://docs.rs/tokio/latest/tokio/) | process、daemon | 进程、信号、网络与有界 channel 能力完整；根 manifest 只打开当前模块所需 feature |
| CLI | [`clap`](https://docs.rs/clap/latest/clap/) | cli | derive API、子命令、帮助和错误输出成熟 |
| TUI | [`ratatui`](https://docs.rs/ratatui/latest/ratatui/) + [`crossterm`](https://docs.rs/crossterm/latest/crossterm/) | tui | Ratatui 默认推荐 Crossterm，覆盖 Linux、macOS、Windows 终端 |
| 领域序列化 | [`serde`](https://docs.rs/serde/latest/serde/) | core、protocol 等 | 配置前端与协议 DTO 共用稳定数据模型接口，但不共用结构体 |
| YAML | [`serde-saphyr`](https://docs.rs/serde-saphyr/latest/serde_saphyr/) | config | 维护中、强类型反序列化、错误位置和解析预算能力较完整 |
| TOML / JSON | [`toml`](https://docs.rs/toml/latest/toml/) / [`serde_json`](https://docs.rs/serde_json/latest/serde_json/) | config、protocol | 官方生态实现，直接接入 Serde |
| 任务图 | [`petgraph`](https://docs.rs/petgraph/latest/petgraph/) | core | 提供 O(V+E) 拓扑排序和环检测；Procora 在外层补稳定 ID 与可读错误 |
| 进程组 | [`process-wrap`](https://docs.rs/process-wrap/latest/process_wrap/) | process | `command-group` 的后继；组合 POSIX 进程组/会话与 Windows Job Object，并支持 Tokio |
| 资源监测 | [`sysinfo`](https://docs.rs/sysinfo/latest/sysinfo/) | monitor | 统一获取三平台进程 CPU、内存和 I/O；平台精度差异仍由 Procora capability 表达 |
| 文件监听 | [`notify`](https://docs.rs/notify/latest/notify/) | source | 三平台文件变更通知；事件只作为“需要重读”的提示，不当作配置事实 |
| 本地 IPC | [`interprocess`](https://docs.rs/interprocess/latest/interprocess/) | daemon | 以统一 local socket API 覆盖 Unix domain socket 和 Windows Named Pipe，并支持 Tokio |
| 单实例锁 | [`fs2`](https://docs.rs/fs2/latest/fs2/) | daemon | 通过当前用户状态目录中的跨平台文件锁阻止两个 Center 争用同一 IPC 与 SQLite |
| 平台目录 | [`directories`](https://docs.rs/directories/latest/directories/) | platform | 按 XDG、macOS 标准目录和 Windows Known Folder 定位数据目录 |
| 状态数据库 | [`rusqlite`](https://docs.rs/rusqlite/latest/rusqlite/) + bundled SQLite | storage | 保存服务注册、当前状态和状态历史；WAL 与事务保证中心请求间的一致性 |
| 日志压缩 | [`flate2`](https://docs.rs/flate2/latest/flate2/) | log | 以 Rust 后端生成 gzip 轮转归档，不把日志正文写入 SQLite |
| 依赖下载 | [`ureq`](https://docs.rs/ureq/latest/ureq/) | source | 阻塞流式 HTTP(S) 下载；SSH 来源交给当前用户已配置认证的 OpenSSH `scp` |
| 归档解包 | [`zip`](https://docs.rs/zip/latest/zip/) / [`tar`](https://docs.rs/tar/latest/tar/) / `flate2` | source | 自动识别 ZIP、tar、tar.gz/tgz 和 gzip；只接受安装根目录内的安全条目 |
| 内容摘要 | [`sha2`](https://docs.rs/sha2/latest/sha2/) | source | 下载后验证可选 SHA-256，并把实际摘要写入版本清单 |
| 诊断日志 | [`tracing`](https://docs.rs/tracing/latest/tracing/) + [`tracing-subscriber`](https://docs.rs/tracing-subscriber/latest/tracing_subscriber/) | cli、daemon、process | 结构化内部诊断与异步 span；任务 stdout/stderr 仍由 `procora::log` 独立管理 |
| 错误 | [`thiserror`](https://docs.rs/thiserror/latest/thiserror/) / [`anyhow`](https://docs.rs/anyhow/latest/anyhow/) | library / binary | 库暴露稳定错误枚举，入口程序补充上下文并统一呈现 |
| 运行身份 | [`uuid`](https://docs.rs/uuid/latest/uuid/) | log、protocol、storage | 生成 daemon、run 和协议会话等不透明身份 |

## 3. systemd 选型

systemd 只存在于 Linux，因此放入 `procora::platform::systemd` feature：

- [`sd-notify`](https://docs.rs/sd-notify/latest/sd_notify/)：纯 Rust、轻量，用于 daemon 就绪、状态和 watchdog 通知。
- [`zbus_systemd`](https://docs.rs/zbus_systemd/latest/zbus_systemd/)：基于 zbus 的 systemd D-Bus 类型代理，用于查询和控制 unit。
- [systemd 官方 D-Bus API](https://www.freedesktop.org/software/systemd/man/latest/org.freedesktop.systemd1.html)：作为实际行为和兼容性的最终依据。

骨架已经提供 `notify_ready` 与 `list_unit_names`，但默认不启用 feature。`procora enable/disable` 不依赖这个可选 D-Bus 层，而是生成固定的 systemd user unit 并调用 `systemctl --user` 完成加载、启停和移除。更广泛的外部 unit 控制、系统总线选择、polkit 权限失败和 D-Bus 信号订阅仍必须先在 Linux CI 上建立集成测试。

Procora 自己管理的任务不默认转换成 systemd unit。systemd 适配器用于 daemon 宿主集成和用户显式选择的外部 unit 控制，避免出现两个监管器同时拥有同一进程树。

## 4. 明确不采用或暂缓

### 4.1 YAML 旧实现

不使用 `serde_yaml`：其[官方文档已标明项目不再维护](https://docs.rs/serde_yaml/latest/serde_yaml/)。也不使用 `serde_yml`：其[当前版本已经弃用并要求迁移](https://docs.rs/serde_yml/latest/serde_yml/)。

`serde-saphyr` 仍处于 `0.0.x`，因此配置层必须通过自己的窄封装使用它，并用跨格式契约测试锁住行为。若将来更换解析器，影响只能停留在 config crate 内。

### 4.2 Python 嵌入

暂不引入 PyO3。Python 配置按架构文档在受控子进程中输出规范 JSON，避免把 Python 运行时、GIL 和 ABI 带入 daemon。该方案不是安全沙箱，只运行可信配置。

### 4.3 数据库边界

结构化中心状态已经切换为 SQLite；数据库不保存 Service/Task 日志正文。日志继续使用所属 Service 目录内的活动文件和 gzip 归档，避免 Center 数据目录成为全部项目日志的集中存储点。

### 4.4 shell 解析

任务默认使用程序名与参数数组，不引入 shellwords/shlex 作为主执行路径。显式 shell 模式后续按平台实现，并在配置和 TUI 中突出安全风险。

## 5. 下一批候选

以下库适合后续里程碑，但当前没有直接消费者，暂不写入依赖图：

- `tokio-util`：`CancellationToken`、IPC codec 和可控关闭。
- `notify-debouncer-mini`：本地配置监听的去抖；需要先确定 include 闭包语义。
- `schemars`：生成配置 JSON Schema；需要先稳定 `RawProject`。
- `proptest`：状态机、图和配置合并的性质测试。
- `cargo-deny` 与 `cargo-audit`：许可证、来源、重复依赖和安全公告门禁。
- `zeroize` 或 `secrecy`：敏感配置值的内存与 Debug 防护。

新增依赖前需要回答：哪个 crate 是唯一消费者、能否保持 feature 最小化、三平台是否编译、是否引入 C/系统库、最低 Rust 版本是否变化，以及如何通过 `tests/` 契约测试限制其行为。
