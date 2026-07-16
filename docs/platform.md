# 三平台适配策略

## 1. 支持范围

Procora 的三个目标平台为：

- Linux：主流 glibc 发行版作为首要开发目标，musl 兼容后续验证。
- macOS：当前受支持的 Apple Silicon 与 Intel 系统版本。
- Windows：Windows 10/11 与对应 Windows Server，原生控制台环境。

Release 固定构建六个目标 triple：`x86_64-unknown-linux-gnu`、`aarch64-unknown-linux-gnu`、`x86_64-apple-darwin`、`aarch64-apple-darwin`、`x86_64-pc-windows-msvc`、`aarch64-pc-windows-msvc`。项目不维护平台原生安装包；WSL 视为 Linux 环境，不与原生 Windows 进程混合管理。实际最低操作系统版本需要在发布流水线持续验证后固定。

## 2. 平台层边界

`procora::platform` 只提供操作系统原语和能力探测，上层 `procora::process`、`procora::monitor`、`procora::daemon` 负责组合业务语义。建议内部结构：

```text
procora::platform/src/
├── lib.rs              # 公共类型与能力集合
├── process.rs          # 跨平台进程身份和控制接口
├── ipc.rs              # 本地端点抽象
├── dirs.rs             # 配置、数据、日志和运行目录
├── linux/              # Linux 实现
├── macos/              # macOS 实现
└── windows/            # Windows 实现
```

平台模块通过 `cfg` 编译，公共接口不暴露平台句柄。确实需要高级能力时使用带命名的平台扩展类型，不能把 Windows 句柄伪装成 Unix 文件描述符。

## 3. 进程树控制

### Linux

- 使用进程组或会话隔离受管任务，停止时面向整个进程组。
- 优雅停止通常发送 SIGTERM，宽限期后发送 SIGKILL。
- 进程身份结合 PID 与 `/proc` 可用的启动信息校验。
- cgroup v2 可用于更强的树追踪和资源限制，但属于可选能力，首版不能依赖其存在。

### macOS

- 使用进程组组织任务并发送 POSIX 信号。
- 进程与资源信息通过系统支持的进程查询接口获取，不能假设存在 `/proc`。
- 某些后代脱离进程组后无法保证完全回收，必须通过能力和诊断信息说明。

### Windows

- 使用 Job Object 关联任务进程树，并配置关闭时回收语义。
- 优雅停止需要区分控制台控制事件、窗口消息和强制终止；并非所有程序都响应同一种方式。
- 进程身份结合 PID、创建时间和持有的系统句柄校验。
- 命令行参数使用 Windows 参数编码规则，不能套用 POSIX shell 转义。

所有平台的进程启动结果都应返回不透明 `ProcessIdentity`。停止、监测和恢复操作以该身份为输入，避免上层只传裸 PID。

## 4. 全局 Procora 服务器开机自启动

`procora enable` 和 `procora disable` 管理当前用户的全局 Procora 服务器，不把每个 Service/Task 分别交给操作系统，从而避免 Procora 与原生托管器同时监管同一任务树：

| 平台 | 当前用户级后端 | 注册位置或名称 | 启动时机 |
| --- | --- | --- | --- |
| Linux | systemd user unit | `$XDG_CONFIG_HOME/systemd/user/procora.service` | 用户 systemd 会话启动；通常为登录时 |
| macOS | LaunchAgent | `~/Library/LaunchAgents/dev.procora.center.plist` | 用户登录时 |
| Windows | 任务计划程序 | `Procora Center` | 当前用户交互登录时 |

原生托管器的主进程始终是 `procora __daemon` 前台进程。Linux 使用 `Restart=on-failure`、启动速率限制、10 秒停止期限和控制组兜底回收；`ExecStop` 先执行 `procora down` 完成正常关闭。systemd 单元通过同目录临时文件原子替换，注册后检查 `is-active`，禁用时即使 unit 文件缺失也会清理 enabled/failed 状态。macOS 的 KeepAlive 只恢复非成功退出；Windows 计划任务使用当前用户的受限权限并绑定交互登录会话。

`enable` 会先正常关闭已有手动全局服务器，再完成注册并等待新进程就绪；`disable` 会先正常关闭全局服务器，再停止和移除注册。两个命令都保留 SQLite 注册表与各 Service 日志。

托管定义固定记录执行 `enable` 时的可执行文件、端点和数据库绝对路径。它不要求管理员权限，也不自动启用 Linux linger；需要“无人登录也启动”时应由管理员显式执行 `loginctl enable-linger <user>`。

## 5. 本地 IPC

- Linux/macOS：Unix domain socket，服务端用内核提供的 peer credentials 校验连接者有效 UID 与 Center 相同。
- Windows：Named Pipe，使用只允许对象所有者、系统和管理员访问的受保护 DACL。

`procora::protocol` 负责帧和 DTO，平台层负责可靠字节流、端点生命周期和 peer 身份。单条请求限制为 1 MiB、响应限制为 8 MiB，连接具有读写超时和并发上限；daemon 启动时处理陈旧端点，但删除前必须确认没有活跃实例。锁文件、peer 身份、实例 ID 和握手共同构成本地边界。

首版协议只接受本机连接。TCP 即使绑定 loopback 也不作为本地 IPC 的默认替代，因为认证、端口冲突和防火墙语义更复杂。

## 6. 文件系统与目录

Center 的 SQLite 数据库、缓存和运行时端点使用平台标准用户目录。Service/Task 日志是明确例外：它们固定保存在所属 Service 的 `.procora/logs` 目录，不集中复制到用户数据目录。路径适配器返回强类型目录，不让调用者自行猜测 `$HOME`、`XDG_*`、`~/Library` 或 `%APPDATA%`。

文件操作需要考虑：

- Windows 文件可能因正在读取而无法重命名或删除。
- macOS 与 Windows 的默认文件系统通常大小写不敏感，任务 ID 到路径映射不能只靠大小写区分。
- 原子替换能力和目录同步语义存在差异，持久化层需要平台测试。
- 文件监听可能产生重复、乱序或合并事件，配置层必须去抖后重新读取完整内容。

## 7. 终端能力

TUI 应通过终端库抽象输入、备用屏幕、颜色和尺寸变化。至少处理：

- Unix TTY 与 Windows Terminal。
- 终端过小时显示降级页面，而不是布局 panic。
- 非交互输出自动退化为纯文本或 JSON。
- `PROCORA_TUI_PLAIN=1`、`NO_COLOR` 或 `TERM=dumb` 使用 ASCII 无彩色模式。
- Ctrl-C、Ctrl-Break、关闭终端等事件通过入口层转换为统一命令。

TUI 不依赖平台进程接口；任何平台差异都通过协议能力标志和可选指标进入视图模型。

## 8. 资源指标差异

跨平台公共指标只承诺统一单位与含义，不承诺所有平台都有值：

| 能力 | Linux | macOS | Windows |
| --- | --- | --- | --- |
| 进程树 CPU 时间 | 计划支持 | 计划支持 | 计划支持 |
| 常驻内存聚合 | 计划支持 | 计划支持 | 计划支持 |
| 进程 I/O 字节 | 视内核接口 | 视系统接口 | 计划支持 |
| 网络按进程统计 | 首版不支持 | 首版不支持 | 首版不支持 |
| 强资源隔离 | 可选 cgroup | 首版不支持 | 可选 Job Object 限制 |

表格描述的是设计目标，不代表当前仓库已经实现。每个采样值都附带 `native`、`estimated` 或 `unavailable` 状态，TUI 对不可用项显示 `—`。

## 9. 平台测试要求

- 公共契约测试在三平台运行相同场景。
- 信号、Job Object、文件滚动、端点权限和 PID 身份必须有平台专属集成测试。
- 不能只使用 mock 验证进程树回收；CI 需要启动会派生子进程的测试夹具。
- 时间和资源数字使用范围断言，避免依赖调度器的精确时序。
- 平台缺失能力测试断言显式降级，而不是跳过整个用例。

更完整的矩阵见[测试策略](testing.md)。
