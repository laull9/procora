# CLI 与中心服务器语义

## 1. 固定层级

Procora 的用户模型固定为 `Center → Service → Task`：

- Center 是当前用户级唯一后台协调进程，维护服务注册表和本地 IPC。
- Service 由规范化目录、被选中的配置文件和配置内 `project` 名称共同确定。
- Task 只在所属 Service 内唯一，由该服务的 `Engine` 调度；Task 不能脱离 Service 被中心服务器直接托管。

中心服务器负责多服务路由，`ServiceHost` 负责单服务的运行组合，`Engine` 负责单服务内部的 Task 规则。三者不能合并成一个全局任务表。

## 2. 默认入口

`procora` 不带参数时以当前目录为服务目标：

1. 探测当前用户的中心服务器。
2. 中心服务器存在时，优先按规范化目录连接已注册服务。
3. 目录尚未注册时，在中心服务器中发现配置并注册服务。
4. 获取一致性 Task 快照后打开 TUI。
5. 中心服务器不存在时，在当前进程创建嵌入式 `ServiceHost`；其生命周期与 TUI 相同，TUI 退出后不保留后台宿主。

这个默认入口不会仅因打开 TUI 而留下新的后台进程。需要持久托管时使用 `procora server <path>`。

连接 Center 后，TUI 进入“中心前端模式”，并非只读模式。客户端先协商协议版本、中心实例 ID、当前事件游标和 `control_allowed` 能力；允许控制时可在界面内启动、停止或重启服务。客户端按游标拉取增量事件，游标过期或中心重启时重新获取完整快照。

## 3. 项目初始化与中心进程

- `procora init --config yaml|json|toml`：在当前目录写入 `procora.yaml`、`procora.json` 或 `procora.toml` 示例；默认 YAML。已有同名文件时拒绝覆盖，只有显式 `--force` 才覆盖。
- `procora up`：确保当前用户唯一 Center 运行，完成协议握手并输出实例 ID、协议版本、服务数和控制能力。
- `procora down`：发送正常关闭请求并等待端点退出；保留中心 SQLite 状态和每个 Service 自己的日志。
- `procora status`：只探测并显示状态，不隐式启动 Center。
- `procora enable`：正常关闭已有的手动 Center，把内部前台 daemon 注册到当前平台的用户级原生托管器，并立即启动。
- `procora disable`：正常关闭 Center，停止并移除当前用户的自启动注册；不删除 SQLite 状态和 Service/Task 日志。

自启动在 Linux 使用 `systemd --user`，在 macOS 使用 LaunchAgent，在 Windows 使用当前用户的登录触发计划任务。三者都直接监管内部前台 daemon，不通过会再次派生进程的 `procora up`。因此原生托管器能正确观察退出和失败，并在崩溃时按平台定义恢复。

这些注册都以“当前用户登录”为启动时机，不申请管理员权限。Linux 若要求用户尚未登录时也在系统启动阶段运行，需要由管理员单独配置该用户的 linger；`procora enable` 不会擅自修改这个用户级系统策略。升级后若可执行文件位置发生变化，需要在新二进制下重新执行 `procora enable`。

## 4. 服务注册与发现

`procora server <path>` 会确保中心服务器运行，然后把路径交给中心服务器处理。路径是文件时只编译该显式文件；路径是目录时只扫描第一层的 `procora.yaml`、`procora.yml`、`procora.toml`、`procora.json`，并以完整配置编译结果判断合法性。其他 YAML、TOML、JSON 文件不会进入候选集合。

发现结果必须满足以下一种情况：

- 一个合法配置：注册并进入运行期望。
- 多个合法配置：拒绝猜测，要求用户传入显式文件路径。
- 没有合法配置：返回候选文件的失败摘要；没有候选时返回未发现错误。

服务名称与服务目录是一一对应关系。同名服务不能注册到两个目录，同一目录也不能静默改成另一个名称。若确需改名，后续应通过显式移除/迁移命令完成，而不是隐式覆盖。

## 5. 定位规则

`show` 和生命周期命令接受名称或路径：

- 已存在路径、绝对路径、`.`、`..` 或包含路径分隔符的输入按路径处理。
- 其他输入按配置中的 `project` 名称处理。
- 路径先规范化，再与已注册的服务目录或配置文件比较。

按路径查看只针对已注册服务；注册新服务使用 `server <path>`。这个区分保证 `show` 不会产生隐式持久化副作用。

## 6. 生命周期命令

- `server start`：先停止旧宿主中的真实 Task，重新加载已注册配置，再按依赖条件启动新的运行代次。
- `server restart`：按反向依赖顺序停止真实 Task，重新加载配置并替换当前 `ServiceHost`，再按拓扑顺序启动。
- `server stop`：按反向依赖顺序优雅停止真实 Task；超过各 Task 宽限期后强制回收进程树，同时保留注册信息。
- `server list`：稳定按服务名称排序，输出状态、Task 数量、目录和配置文件。
- `server history`：按名称或路径查询 SQLite 中的状态变更历史；不读取日志文件。

中心服务器使用当前用户数据目录中的 `procora.sqlite3` 保存注册表、服务当前状态和状态历史。测试和隔离环境可通过 `PROCORA_HOME` 覆盖目录；本地 IPC 端点从该目录派生，以避免不同用户或隔离环境互相连接。

SQLite 不保存日志正文。Service 日志固定写入自身目录的 `.procora/logs/service.log`，Task 日志写入 `.procora/logs/tasks/<task>.log`；压缩归档也留在该 Service 目录，不汇总到 Center 数据目录。

Center 使用跨平台独占文件锁保证同一 `PROCORA_HOME` 只有一个实例。本地协议先进行版本握手，服务变化进入容量为 256 的内存事件缓冲；慢客户端游标过期后必须重取快照，不能把不连续事件误当作完整状态。

## 7. 当前状态含义

服务状态是宿主级状态：

- `running`：配置已成功编译，`ServiceHost` 已装配并正在对账真实 Task，服务具有运行期望。
- `stopped`：服务仍在注册表中，但没有运行期望。
- `failed`：恢复或生命周期操作时无法加载配置。

每个 Task 的 `pending/blocked/running/stopped/failed`、日志和资源独立展示，不能用服务状态覆盖 Task 状态。嵌入模式下按 `q`、Esc 或 Ctrl-C 退出会先反向停止全部 Task，再恢复终端。
