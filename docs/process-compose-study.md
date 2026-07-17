# process-compose 对照研究

## 1. 研究基线

本轮研究基于 `F1bonacc1/process-compose` 的 `v1.120.0`，提交 `0f3a8e6868f592f044a2ecd9387ff3025825ad46`。对照对象是成熟项目积累的运行语义、故障案例和用户工作流，不是其 Go 代码结构或配置字段的逐项移植。

Procora 保持以下边界不变：

- 固定 `Center → ServiceHost → Task`，不把 Task 提升为跨服务全局对象。
- 默认只开放同用户本地 IPC，不因为上游提供 REST/MCP 就扩大网络攻击面。
- 命令文本、`command + args` 与 argv 数组统一规范化，默认不经过 shell，也不隐式执行模板。
- 配置、事件和进程结果先绑定 `generation/run_id`，再进入单写者引擎。
- 服务日志留在服务目录，Center 只保存结构化状态。

## 2. 能力差距矩阵

| 能力 | process-compose | Procora 状态 | 决策 |
| --- | --- | --- | --- |
| 依赖启动与完成条件 | 成熟 | 已实现 | 保持确定性 DAG 与反向停止 |
| 重启策略 | 多策略和 `max_restarts` | 已实现有界指数退避、尝试上限、稳定窗口重置和耗尽诊断 | 后续只补多服务同步故障抖动 |
| 健康检查 | exec/HTTP、readiness/liveness | 已实现 exec/HTTP GET readiness、连续阈值、取消与依赖门控 | 保持 readiness 单一语义；不把 unhealthy 隐式升级为重启 |
| 日志背压与续读 | 内存缓存、订阅、文件 | 已实现有界管线、持久游标、Gap 和高速并发压力门禁 | 继续增加长期运行与故障注入 |
| 资源采样节流 | 快慢刷新周期分离 | 已把 500ms 状态刷新与 1s 资源采样解耦，同一宿主一次刷新批量聚合全部 Task 树 | 后续按客户端可见性进一步降频 |
| 在线配置更新 | 已实现 | 已有安全 include 闭包、防抖候选、SHA-256、差异预览、显式确认、按影响 Task 对账和失败回退 | 不采用无确认自动重启 |
| 环境文件与替换 | 已实现 | 已实现显式 `env_file`，不默认读取 `.env`，不执行变量替换 | 保持确定性优先级、闭包修订与服务根边界 |
| 全局环境、变量与紧凑命令 | environment、Go template、shell 命令 | 已实现项目 `env`、显式变量链、命令文本与精确 argv，effective config 可解释 | 不读取宿主环境，不引入模板语言，默认不经过 shell |
| 成功退出码 | 可配置 | 已支持非负退出码集合并隐式包含 0 | 已统一接入重启与完成依赖语义 |
| 依赖条件 | `process_*` 条件对象 | 已支持名称数组、条件标量、旧版对象和上游条件别名 | 统一规范化为三种可执行语义，不错误映射 `process_completed` |
| 命名空间与副本 | 已实现 | Service 已提供天然隔离，Task 无副本 | 先设计 Service 内分组；不复制上游全局 namespace |
| 计划任务 | cron/interval | 未实现 | 需要先明确一次性任务与重启状态机关系 |
| 交互进程 | PTY、按键注入 | 未实现 | 独立于普通日志管线设计，避免后台 PTY 阻塞 |
| TUI 命令面板和主题 | 已实现 | 已可结构化维护和切换 profile，也可编辑项目默认、Task 模板选择/来源、`env_file` 及 exec/HTTP 健康检查 | 模板定义保留 F2 高级文本，主题不是阻塞项 |
| 远程 REST/MCP | 可选令牌认证 | 未实现 | 当前不引入；只有完成威胁模型后才考虑只读接口 |
| 配置继承/模板 | 多文件合并、process extends | 已实现受限 include、`task_defaults`、命名模板链与 profile 继承/持久准入 | profile 与模板的列表都整体替换，不追加 args |
| 动态配置代码 | 模板与扩展能力 | 已实现仅显式入口的受控 `procora.py` | 不嵌入 Python，不把故障隔离描述成安全沙箱 |
| Git 定义来源 | 主要面向本地配置 | 已实现固定 commit、资源有界、重新确认的候选 API | 远端不自动应用，不执行 Python，不先扩大网络控制面 |

## 3. 已吸收的故障经验

### 3.1 健康事件必须属于当前 run

上游曾修复更新或重启后陈旧 readiness 对象导致依赖级联失败的问题。Procora 的检查计划从创建起绑定 `generation/run_id`；Task 退出、停止或重启会先取消检查，旧结果无法改变新 run。

### 3.2 检查、日志和终端读取都不能无限阻塞

上游历史修复包含退出时管道阻塞、日志订阅背压死锁和后台 PTY 未排空。Procora 的检查同一 Task 最多一个活动进程，结果槽容量固定为一；超时和取消都会通过受管进程组或 Job Object 回收检查进程树。日志继续使用独立有界队列。

### 3.3 网络控制面不是免费能力

上游 `v1.120.0` 修复了 MCP SSE 的 DNS rebinding，并增加 Host、Origin 与令牌边界。Procora 当前本地 IPC 已做同用户校验、连接并发和帧大小限制，因此不会仅为功能对齐增加监听 TCP 端口。未来若加入网络入口，必须先定义绑定地址、来源校验、认证、重放防护、审计与默认关闭策略。

### 3.4 动态筛选必须持久保存准入意图

上游在 namespace 更新中修复了重新加载后被排除进程复活的问题。Procora 因此把活动 `profile` 和它的 Task 白名单持久写入配置，热更新仍从该选择重新编译任务图；文件变化不会绕过运行准入。所有未准入 Task 继续独立校验，避免切换场景后才发现损坏定义。

### 3.5 少写配置不能牺牲 argv 精确性

上游提供[全局环境变量](https://f1bonacc1.github.io/process-compose/configuration/#environment-variables)，并把命令交给可配置 shell。Procora 吸收少写配置的体验：顶层 `env` 按键合并到全部 Task；字符串命令可用空白、单双引号和反斜杠直接表达参数，也可使用非空字符串数组提供无歧义 argv。两者只做分词而不执行 shell，普通 Windows 路径反斜杠会保留；`"hello world"` 和空参数在三种格式与 TUI 往返中保持同一边界。管道、重定向或 `&&` 必须显式写成 `[sh, -c, ...]` 等 argv。

### 3.6 共享默认值必须保持覆盖可解释

上游 process `extends` 支持继承链，并对 `args` 等列表采用“基类在前、子类在后”的追加规则。Procora 用单层 `task_defaults` 处理真正适合全部 Task 的工作目录、环境和生命周期字段，再用显式 `task_templates` 承载命令、参数、依赖、环境文件和健康检查等身份字段。模板支持单继承链：map 按键合并，标量与列表整体替换，显式命令把 argv 作为一个执行单元替换；effective config 标出 `task_template` 和最终获胜模板名。这样吸收上游减少重复与循环诊断的经验，但不引入难以察觉的 argv 追加。参考[上游合并与 process inheritance](https://f1bonacc1.github.io/process-compose/merge/)。

### 3.7 变量表达力必须有明确边界

上游用 Go template 为命令、工作目录、环境和探针提供变量、条件与函数。Procora 只吸收减少重复字符串的核心需求：`${vars.NAME}` 支持确定性链式引用，`$${vars.NAME}` 显式转义；解析不读取宿主环境、不执行函数或 shell，也不能改变 Task/profile/依赖等结构身份。argv 数组内插值保持元素边界，命令文本则明确在插值后分词。effective config 同时报告原始值、解析值和字段引用，TUI 写回保留表达式。参考[上游 Variables](https://f1bonacc1.github.io/process-compose/configuration/#variables)。

### 3.8 场景继承也必须保留显式替换边界

上游 process inheritance 在全部配置合并后解析继承链，map 按键组合，但列表采用追加。Procora 把同样的“先合并声明、再解析继承”顺序用于 profile：开发、CI 等场景可通过 `extends` 复用环境和 Task 默认层；map 按键组合，Task 白名单与默认标量/列表由子层显式值整体替换。省略白名单与显式 `[]` 含义不同，未知、自继承和循环链都指向 `.extends`。`procora config` 输出直接继承映射，TUI 可结构化维护整条声明并在变更后立即重编译预览。这样减少场景重复，同时不引入上游列表追加造成的隐式 Task 准入。参考[上游合并与 process inheritance](https://f1bonacc1.github.io/process-compose/merge/)。

### 3.9 高频依赖声明应简短但不能模糊条件

上游用 `process_started`、`process_healthy`、`process_completed_successfully` 等条件对象表达调度边。Procora 保留完整对象兼容，同时让全默认 `started` 场景直接使用 Task 名称数组，混合条件使用名称到条件的标量 map；上游三种等价条件名只作为输入别名，effective config 和 TUI 统一输出 Procora 规范名。名称列表严格拒绝非字符串和重复项，模板合并与逐边来源保持不变。上游的 `process_completed` 表示不考虑退出码完成，而 Procora 当前只有成功完成语义，因此明确不做有损别名。参考[上游 Processes Lifetime](https://f1bonacc1.github.io/process-compose/launcher/)。

### 3.10 时间单位应在人类输入边界明确

上游在[健康检查](https://f1bonacc1.github.io/process-compose/health/)和[进程生命周期](https://f1bonacc1.github.io/process-compose/launcher/)中分别使用明确单位的时间配置。Procora 早期字段把 `_ms` 固定在名称中，适合领域 API，却迫使手写配置换算较长时间。现在声明层使用 `restart_delay: 750ms`、`period: 1m30s` 等紧凑时长，严格限定 `h`、`m`、`s`、`ms` 的降序组合；旧 `_ms` 整数保持兼容，别名冲突直接报错。解析后仍统一为毫秒做边界校验、差异比较与运行调度，TUI 也只在输入输出边界格式化，不把单位语义扩散进引擎。

## 4. Procora 落地顺序

1. 完成 exec 健康检查、连续阈值、超时整树回收、TUI 状态和跨平台真实测试。
2. 建立 cargo-deny、RustSec、许可证、MSRV 和兼容性治理。
3. 已实现配置事件去抖、内容哈希、候选修订、语义差异和按影响 Task 对账，失败时保留旧修订。
4. 显式环境文件、项目默认环境、`task_defaults`、命令文本/argv 简写、effective config、shell completion 与成功退出码已经落地。
5. 有效配置已能说明变量引用、内建默认、项目 env、Task 默认、活动 profile/继承映射、具体命名模板、env_file、依赖边与 Task 显式值来源；TUI 可编辑变量、结构化维护和切换 profile、选择模板、用紧凑依赖与可读时长语法、清除覆盖且保存不展开表达式/继承值或丢失未准入 Task。
6. 已增加高速日志和大图性质测试；继续补长运行和并发客户端故障注入。
7. 配置体验稳定后再评估计划任务、Service 内分组、副本和交互 PTY；本阶段不继续扩展安全功能面。

这个顺序把正确性、恢复和可诊断性放在功能数量之前。上游的新功能与修复记录会持续作为回归案例来源，但不会自动变成 Procora 的需求。
