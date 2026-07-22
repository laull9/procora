# Procora

Procora 是一个以 TUI 为主要入口的本机任务服务管理器。每个用户有一个全局 Procora 服务器，可托管多个服务；每个服务由“目录 + 唯一配置文件”定义，并管理自己的 Task 依赖图。

```text
全局 Procora 服务器（每个本机用户一个）
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
- Center IPC 以 64 KiB 有界分片按游标续读日志；CLI 边接收边输出，日志总量不会膨胀成单个协议帧。

## 高频命令

| 命令 | 语义 |
| --- | --- |
| `procora init --config yaml/json/toml` | 创建不依赖 Cargo 的可运行示例并自动打开配置编辑页；脚本可加 `--no-edit`。 |
| `procora edit [path/config]` | 打开 TUI 配置编辑页；可维护 profile/继承、项目环境、项目级 `task_defaults`、Task `env_file`、生命周期/成功退出码，并用 `h` 单独编辑 exec/HTTP 健康检查。 |
| `procora clean [path/config]` | 清空服务目录中的 `.procora` 运行时文件、日志和管理依赖缓存。 |
| `procora push <local> [--target <service::name\|service::task::name>] [--ssh HOST]` | 通过 OpenSSH 把本机文件或目录完整替换到远端声明目标；唯一兼容目标会自动选择。 |
| `procora deps [path/config]` | 下载、智能解包、缓存并验证项目声明依赖；`--check` 只做离线验证。 |
| `procora up` | 启动全局 Procora 服务器。 |
| `procora down` | 停止全局 Procora 服务器；保留状态和日志。 |
| `procora status` | 查看全局服务器状态，不会启动它。 |
| `procora enable` | 注册并立即启动用户级开机自启动；Windows 会显式请求 UAC 提权。 |
| `procora disable` | 停止并移除开机自启动；Windows 会显式请求 UAC 提权，并保留状态和日志。 |
| `procora completions <shell>` | 输出 Bash、Zsh、Fish、PowerShell 或 Elvish 补全脚本。 |
| `procora mcp` | 通过 stdio 运行本地 MCP 服务，提供结构化工具和内嵌文档 Prompts。 |
| `procora` | 打开全部已注册服务的总览 TUI，可筛选、排序、管理服务；按 `n` 选择托管目录快速创建服务，随后直接进入编辑管理。 |
| `procora path/config` | 直接打开指定服务目录或配置文件；全局服务器未运行时用内联选择栏询问启动全局或临时服务。 |
| `procora temp-run [path/config]` | 显式启动只与本次 TUI 同生命周期的临时服务。 |
| `procora add <path>` | 必要时启动全局服务器，并注册、启动指定服务。 |
| `procora list` | 列出全局服务器中的服务；服务器未运行时不会启动它。 |
| `procora history <name/path>` | 从 SQLite 查询指定服务的状态变更历史。 |
| `procora show [name/path]` | 按名称、服务目录或配置文件打开 TUI；省略目标时使用当前目录，路径尚未注册时自动发现。 |
| `procora logs <name/path> <task> [--search TEXT\|--filter TEXT\|--clear]` | 输出 Task 日志；可按行搜索、过滤，或清空活动日志与轮转归档。 |
| `procora start <name/path>` | 重新加载已注册配置并启动服务宿主。 |
| `procora restart <name/path>` | 重新加载配置并重启服务宿主。 |
| `procora preview <name/path>` | 编译当前文件并输出 SHA-256 修订及新增、删除、重启、原地更新和无影响 Task。 |
| `procora apply <name/path> <revision>` | 仅在磁盘内容仍匹配已确认修订时应用候选。 |
| `procora stop <name/path>` | 停止服务宿主并保留注册信息。 |
| `procora remove <name/path>` | 停止并删除服务注册，不删除服务目录。 |
| `procora source git preview <repo>` | 获取 Git 引用、固定完整 commit 并校验候选；`--local` 显式允许本地仓库，不注册或启动服务。 |
| `procora source git confirm <repo> <revision>` | 重新获取同一来源并拒绝已变化的候选；仍不自动应用。 |

`validate`、`graph` 和 `doctor` 作为低频诊断命令继续保留。
`config <path>` 会输出应用默认值和路径规范化后的确定性 JSON，不启动任何 Task，适合审查实际运行输入。

命令支持唯一前缀推断，例如 `procora stat` 和 `procora li`。拼写错误会显示最相近命令，所有运行期错误都会附带 `procora --help` 入口。若路径名与命令相同，使用 `./<path>` 明确按路径打开。旧版 `procora server ...` 层级暂时保持兼容，但不再显示在帮助中。

全局和临时 TUI 都支持 `s` 启动、`x` 停止、`r` 重启，并实时刷新状态和日志。日志页解析常见 ANSI/SGR 彩色输出，默认跟随尾部且保留本次查看会话读取到的完整历史；使用 `PageUp/PageDown` 按页浏览、`Home/End` 跳到首尾，macOS 对应 `Fn+↑/↓` 和 `Fn+←/→`。`/` 输入搜索词，`n/N` 循环跳转匹配，`f` 切换只显示匹配行，连续按两次 `C` 清空当前 Task 的活动日志与轮转归档。鼠标滚轮滚动日志，触控板左右滚动与左右方向键一样横移文本，`↑/↓` 或 `j/k` 切换 Task。上翻后新日志不会打断阅读，连接中断时保留当前视图并显示错误。

全局单服务 TUI 按 `e` 直接进入内嵌配置管理。编辑期间仍在后台刷新服务快照；`Ctrl-S` 会先完成本地完整校验和写盘，再通过 Center 预览、核对精确修订并应用，成功后立即刷新 Task 视图，失败则保留编辑页和可重试诊断。总览的 `n` 向导可选择已有配置目录直接托管，也可在空目录中确认服务名称、创建最小 `procora.yaml`，注册后立即打开同一编辑管理页。

后台 Center 会对配置文件事件做 250ms 防抖并生成候选，但不会因为一次保存就自动重启服务。无效候选、项目改名、依赖准备失败和过期修订都会保留旧有效宿主；先运行 `procora preview` 审查影响，再把输出的完整修订交给 `procora apply`。退出码、重启退避和停止宽限等纯运行策略可原地提交；进程身份或依赖图变化只重启受影响的下游闭包，新增和删除按拓扑顺序对账，启动失败时恢复旧有效定义且不重启无影响 Task。

自动重启采用 30 秒封顶的指数退避。Task 可用 `max_restarts` 限制连续自动重启次数（0 表示无限），并以 `restart_reset_after` 指定稳定运行多久后清零连续计数（默认 `1m`，`0ms` 表示禁用）；耗尽后停止继续创建进程，原地放宽上限可恢复调度。

资源指标采用独立的 1 秒慢采样周期，TUI 的 500ms 状态刷新和 50ms 日志续读不会重复扫描系统进程。一个 Service 的全部活动 Task 在同一次系统刷新后批量聚合进程树；CPU 按当前进程可用的逻辑 CPU 总容量归一化到 0–100%，Task 启停会立即失效缓存，退出 Task 不触发空扫描。

TUI 只在输入、终端尺寸或数据发生变化时重绘，状态默认每 500ms 检查一次，日志页每 50ms 续读并在单轮中批量追赶积压内容。`PROCORA_TUI_PLAIN=1`、`NO_COLOR` 或 `TERM=dumb` 会启用 ASCII 无彩色模式。

配置编辑页支持 YAML、TOML 和 JSON。Task 编辑弹窗不再用 Enter 提交，统一按 `Ctrl-S` 校验、写入并退出弹窗；Esc 会探测本轮字段变化并弹出保存、放弃或取消选择。整个编辑页退出时也会探测未保存配置并显示同类选择弹窗。

生命周期和健康检查时长可直接写成 `restart_delay: 750ms`、`shutdown_timeout: 5s`、`period: 1m30s`。支持按 `h`、`m`、`s`、`ms` 降序组合；旧 `_ms` 整数字段继续兼容，TUI 则统一显示和保存可读写法。运行期与 `procora config` 的有效值仍使用毫秒整数，兼容现有 API 与差异判断。

配置顶层 `env` 可为全部 Task 提供默认环境，Task 本地 `env` 只声明覆盖值。`command` 可直接写成 `cargo run -- --port 8080` 命令文本，也兼容字符串加独立 `args`，并接受 `[program, arg1, ...]` 精确 argv 简写；三种写法都会规范化为不经过 shell 的程序和参数数组。命令文本支持单双引号、空参数和保留 Windows 反斜杠路径，TUI 命令字段也可直接输入同样写法，保存带参数 Task 时使用紧凑 argv。表单中的参数、环境变量和 HTTP 请求头优先使用 JSON 数组/对象显示，含空格、空字符串、逗号或等号的值不会在编辑往返中被拆分，同时仍接受旧版空格与 `KEY=VALUE` 输入。

`depends_on` 的常用 `started` 依赖可直接写成名称数组，例如 `depends_on: [database, cache]`；混合条件可写成紧凑 map，例如 `{database: started, cache: healthy}`。既有 `{database: {condition: started}}` 完整对象继续兼容，输入也接受 process-compose 的 `process_started`、`process_healthy` 和 `process_completed_successfully` 拼写，并统一规范化为 Procora 条件名。TUI 依赖字段接受相同别名，保存时全默认条件使用数组、混合条件使用标量 map。

顶层 `vars` 可集中声明可复用字符串，显式支持字段用 `${vars.NAME}` 引用；变量可链式引用，`$${vars.NAME}` 输出字面量 `${vars.NAME}`。解析不读取宿主环境、不执行 Go/Python 模板或 shell，普通 `$HOME` 和管理依赖的 `${dependency.tool}` 保持原样。变量可用于项目/profile/默认环境、默认工作目录，以及 Task/模板的命令、argv、工作目录、环境文件、环境和健康检查字符串；argv 数组内插值不会改变参数边界。`procora config` 同时输出声明值、解析值和字段到变量的直接引用，TUI 项目弹窗可编辑变量并立即刷新有效预览，保存仍保留原始表达式。

Service 和 Task 可声明只位于服务根目录内的 `uploads` 目标，本机无需知道服务器路径即可使用 `service::name` 或 `service::task::name` 上传。省略 `--target` 时，远端按来源类型和大小筛选当前活动目标：只有一个便自动选择，多个则在本机列出供选择。当前版本支持单文件或目录完整替换；内容先以 gzip tar 经 SSH 发送，远端验证类型、未压缩大小和 SHA-256，再用同目录备份与重命名提交。符号链接、特殊文件、父目录逃逸和 `.procora` 都会被拒绝。目标协商与传输共用一条 SSH 会话；SSH 先使用 BatchMode 自动认证，只有连接或认证失败且处于交互终端时才允许修改 SSH 目标，并由 OpenSSH 自己请求主机确认或密码，Procora 不读取或保存密码。CI 使用 `--batch` 禁止交互回退。完整配置与命令见[配置说明](docs/configuration.md#声明式远端上传目标)。

顶层 `task_defaults` 可集中声明所有 Task 共用的 `cwd`、`env`、成功退出码和生命周期策略。Task 标量或列表一旦显式声明便整体替换默认值，环境 map 则按键覆盖；`procora config` 会把来源报告为 `task_defaults`，TUI 项目弹窗可直接编辑默认层，保存和新建 Task 都不会把继承值复制进每个条目。Task 弹窗中的覆盖字段留空、或把重启策略切到 `inherit`，即可删除本地声明并恢复项目/内建默认。

顶层 `task_templates` 可为命令、参数、工作目录、环境文件、环境、健康检查、依赖和生命周期策略建立命名模板；模板支持单继承链，Task 用 `extends` 显式选择。map 按键合并，标量和列表整体替换，显式命令不会追加基模板 argv。`procora config` 会把来源报告为 `task_template` 并给出获胜模板名；TUI 项目卡片显示模板数量，Task 弹窗可选择模板、查看有效来源并只保存局部覆盖，模板定义本身通过 F2 高级文本编辑。

顶层 `profiles` 可把开发、测试或 CI 场景的项目环境、`task_defaults` 和运行 Task 白名单集中命名，`profile` 持久选择当前场景。profile 可用 `extends` 继承另一个 profile：map 按键组合，Task 白名单及默认标量/列表由子层显式值整体替换；未知、自继承和循环链都会精确指向 `.extends`。省略 `tasks` 表示继承基础 profile 的白名单，整条链都未声明时准入全部 Task；显式 `[]` 则明确不准入任何 Task。所有未准入 Task 和未使用 profile 仍完整校验，活动 Task 依赖被排除 Task 会直接报错。TUI 有独立 Profiles 区域，可新增、编辑、重命名和删除 profile；切换或修改后立即重编译预览，保存时保留未准入 Task 和 profile 原始声明。

入口配置可用 `include: [fragments/base.toml, fragments/local.json]` 组合同一服务根目录内的跨格式片段。列表后项覆盖前项、入口最终覆盖；Task、命名模板和管理依赖按完整条目覆盖。循环、父目录/符号链接逃逸、超过 16 层、64 个文档或 4 MiB 的闭包会被拒绝。

Task 可用 `env_file: config/api.env` 显式加载服务根目录内的 UTF-8 环境文件。环境优先级固定为基础项目 `env` < profile 项目 `env` < 基础 `task_defaults.env` < profile `task_defaults.env` < 模板 `env` < 有效 `env_file` < Task 内联 `env`。配置编译结果单独保留声明路径与内联层，因此单文件配置可在 TUI 中编辑 `env_file`，而不会把文件内容复制进配置；真正的 include 多文件入口仍保持文本模式。Procora 不会默认读取 `.env`，也不会在环境文件中执行变量替换；环境文件受大小与变量数限制，其内容参与候选修订并触发受影响 Task 的确认式热更新。

Task 的 `healthcheck` 支持不经过 shell 的 `command + args`，也支持带主机、端口、路径、请求头和精确状态码的 `http_get`。两种探针共享可读的首次延迟、周期、超时和连续成功/失败阈值，并只提供 readiness 语义：达到 unhealthy 不会隐式重启主 Task。

可信项目也可显式传入 `procora.py`，由受控 Python 3 辅助进程输出单个 JSON 配置。目录扫描不会自动执行它；CLI 会提示当前用户权限代码执行。辅助进程有 5 秒、脚本/stdout/stderr 大小、最小环境和整树回收边界，但不是安全沙箱。生成 JSON 仍经过完整配置校验并参与 preview/apply 修订确认。

`procora source git preview/confirm` 和库接口 `GitSource` 可从受限 HTTPS/SSH/SCP 或显式本地仓库获取定义，把引用固定为完整 commit，并在资源有界的无 hooks checkout 中复用同一配置校验。两条 CLI 命令都不注册或启动服务，confirm 会重新获取并拒绝过期修订。Center 尚不持久注册远端来源，也不提供私有仓库凭据代理。

## 完整配置示例

完整的配置写法、规则和一份可校验的综合配置集中在[配置示例](docs/example.md)。文档中的制品地址仅作结构展示，执行 `procora deps` 前需要替换为真实来源。

## 配置发现

`<path>` 可以是 YAML、TOML、JSON、精确命名的 `procora.py` 配置文件，也可以是服务目录：

- 显式文件始终只加载该文件，可用于消除目录歧义；只有精确文件路径才会执行 `procora.py`。
- 目录模式只扫描第一层的 `procora.yaml`、`procora.yml`、`procora.toml`、`procora.json`，不会读取 `package.json` 或其他业务配置。
- 候选必须通过完整 Procora 结构、语义和任务图校验，才算合法配置。
- 恰好一个合法配置时自动选择；多个合法配置时报错；没有合法配置时汇总候选失败原因。
- 配置顶层 `project` 是本机服务稳定名称，只允许 ASCII 字母、数字、点、短横线和下划线。

最小配置：

```yaml
version: 1
project: demo

dependencies:
  protoc:
    source: https://example.com/protoc-29.3.tar.gz
    version: "29.3"
    checksum: sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef
    kind: binary
    path: protoc/bin/protoc
    verify:
      args: ["--version"]

tasks:
  generate:
    command: "${dependency.protoc}"
    args: ["--version"]
```

最常见的依赖只需一行，例如 `tool: https://cdn.example.com/tool.tar.gz` 或 `tool: ssh://user@host/opt/tool`；版本、解包、类型、文件选择、重试、超时和缓存都有开箱即用默认值。需要进一步控制时再展开对象：`mirrors` 提供故障转移，`download` 控制重试、总超时、大小上限和带 `${env.NAME}` 的私有 HTTP 请求头，`ssh` 可显式指定私钥与 known_hosts。TUI 的 Enter 弹窗只显示常用字段，按 `a` 才进入高级下载与 SSH 策略。ZIP、tar、tar.gz/tgz 与 gzip 会自动识别。安装位于 `<service>/.procora/dependencies/<name>/<version>/`，跨进程锁、完整暂存和可回滚替换避免并发或下载失败破坏旧缓存；版本清单、路径类型、下载 SHA-256、最终内容指纹与可选 `verify` 命令会在每次任务启动前复核。远程来源强烈建议固定 `checksum`。

## 代码边界

- `procora::core`：服务名称、Task 标识、项目规范和依赖图。
- `procora::config`：格式解析、配置发现、完整校验和图编译。
- `procora::engine`：单服务内部的 Task 状态与调度规则。
- `procora::daemon`：中心服务器、多个 `ServiceHost`、名称/路径解析和本地 IPC。
- `procora::storage`：SQLite 中心注册表、服务当前状态和状态历史。
- `procora::log`：服务目录内的 Service/Task 文件日志、gzip 轮转和内存尾部缓存。
- `procora::source`：配置监听，以及项目依赖下载、SSH 获取、解包、缓存和版本验证。
- `procora::protocol`：CLI/TUI 与中心服务器之间的稳定 DTO。
- `procora::cli`：命令解析、中心进程拉起和 TUI 连接生命周期。
- `procora::mcp`：复用 CLI 程序化接口的 stdio 工具与内嵌文档 Prompts。
- `procora::tui`：服务及其 Task 的终端视图。

完整设计从[文档索引](docs/README.md)开始；近期实现事项见 [TODO](TODO.md)。

## 开发验证

```bash
cargo fmt --all --check
cargo test --all-features
cargo clippy --all-targets --all-features -- -D warnings
```

代码注释和设计文档统一使用中文。关键 trait、结构体、函数和静态变量需要简短注释，单个代码文件不超过 500 行，关键行为测试统一放在根目录的 `tests/`。
