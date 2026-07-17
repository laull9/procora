# 配置模型与任务图

## 1. 设计范围

配置层负责把 YAML、TOML、JSON，以及受控 Python 辅助进程生成的 JSON 转换为稳定的领域规范。引擎只接收经过校验的 `ProjectSpec` 和 `TaskGraph`，不感知输入文件格式。

当前三种格式已经通过同一份 `RawProject`、字段级诊断、规范化规则和任务图编译。结构解析错误包含字段路径与行列信息；语义校验会一次聚合多个独立错误。

## 2. 加载管线

```text
定位输入
  → 读取并记录来源元数据
  → 按格式反序列化为 RawProject
  → 结构与语义校验
  → 规范化为 ProjectSpec
  → 编译 TaskGraph
```

include 合并、项目变量解析、命名 profile、项目级 `env`/`task_defaults`、命令文本/argv 规范化和闭包内容修订已插入 Raw DTO 与领域规范之间；这些前端能力共享统一校验，不会绕过 `ProjectSpec` 与任务图编译。

当前各阶段使用不同类型，避免未校验字段误入运行时：

- `RawProject`：保留可选字段与用户写法，用于产生精确错误位置。
- `ProjectSpec`：默认值已补全、路径已解析、标识符已规范化的领域对象。
- `TaskGraph`：完成依赖解析和环检测的可执行图。
- `TaskConfigOrigins`：与有效 Task 并行保存字段和最终环境变量的来源，不把“省略”与“显式写默认值”混为一谈。

`DefinitionRevision` 使用入口、全部 include 和显式 `env_file` 的相对路径、原始字节及无效诊断共同计算 SHA-256；统一承载不同类型远程来源的 `ConfigInput` 仍是后续抽象。

错误信息至少包含阶段、来源、字段路径、可读原因和可选修复建议。解析错误保留行列信息；多个独立校验错误应尽量一次返回。

### 2.1 服务目录发现

CLI 的服务路径既可指向显式文件，也可指向目录。显式文件按扩展名加载，不参与其他候选比较；目录只扫描第一层的 `procora.yaml`、`procora.yml`、`procora.toml`、`procora.json`，并对每个候选执行完整解析、语义校验和任务图编译。`package.json`、`compose.yaml` 等其他项目文件即使扩展名受支持也会被忽略。

只有完整编译成功的候选才称为“合法配置”。一个合法候选自动选中，多个合法候选返回歧义错误并要求显式文件路径，没有合法候选时返回所有候选的失败摘要。目录遍历和错误输出必须排序，不能依赖文件系统返回顺序。

### 2.2 include 闭包

入口文件可用路径数组组合 YAML、TOML 或 JSON 片段：

```yaml
include:
  - fragments/base.toml
  - fragments/local.json
version: 1
project: demo
tasks:
  api:
    command: ./api
```

include 按列表从左到右加载，后一个文件覆盖前一个文件，入口最后覆盖全部片段。项目级 `env` 与 `task_defaults.env` 按键合并且后者优先；`task_defaults` 的其他标量和列表仅在高优先级文档显式声明时整体替换。`tasks`、`task_templates` 和 `dependencies` 以完整条目为覆盖单位，不把同名 Task 或模板的命令与另一文件的参数隐式拼接。模板引用与跨文件依赖边在全部合并完成后统一校验。片段可省略 `version` 和 `project`，但一旦声明就必须与入口一致；入口自身必须显式声明两者。

每个片段中的 Task/`task_defaults` 工作目录、健康检查 `cwd`、`env_file` 和本地管理依赖来源先按该片段目录重定位，再进入合并。include 路径必须是服务根目录内、不含 `.` 或 `..` 的相对普通路径；符号链接规范化后也不能逃逸根目录。单次闭包限制为 16 层、64 个文档、合计 4 MiB，循环、缺失文件、身份冲突和未知格式均使整个候选无效。

## 3. 规范模型草案

### 3.1 项目

当前 `ProjectSpec` 包含：

- `version`：配置模式主版本。
- `project`：项目稳定标识。
- `tasks`：以 `TaskId` 为键的任务定义。

配置编译结果另保留项目级 `env`，它作为所有 Task 的低优先级默认环境并在 effective config 中同时显示原始默认层和每个 Task 的展开结果。Task 本地 `env` 只需声明差异：

```yaml
version: 1
project: demo
env:
  RUST_LOG: info
  REGION: local
tasks:
  api:
    command: ./api
  worker:
    command: ./worker
    env:
      RUST_LOG: debug
```

高频共享字段可集中写入 `task_defaults`：

```yaml
version: 1
project: demo
task_defaults:
  cwd: ./app
  env:
    RUST_LOG: info
  restart: on-failure
  restart_delay: 500ms
  max_restarts: 5
  restart_reset_after: 1m
  shutdown_timeout: 5s
tasks:
  api:
    command: [./server, --api]
  worker:
    command: [./server, --worker]
    restart: never
```

当前默认层只接受 `cwd`、`env`、`success_exit_codes` 和五个生命周期字段。Task 显式标量或列表整体替换默认值，`env` 按键覆盖。命令、参数、依赖、环境文件和健康检查刻意不进入全局默认层；需要共享这些任务身份字段时使用带名称和来源的 `task_templates`，避免无意改变所有 Task。默认层即使暂时没有 Task 也会独立校验。

生命周期与健康检查的时间字段优先使用 `restart_delay`、`restart_reset_after`、`shutdown_timeout`、`initial_delay`、`period` 和 `timeout`。值由整数与 `h`、`m`、`s`、`ms` 组成，可组合为 `1m30s` 或 `1s500ms`；组合单位必须从大到小、不能重复，且不接受空白或小数。旧版 `_ms` 字段和整数毫秒继续兼容，机器生成配置也可给新字段写整数毫秒；同一对象不能同时声明一对新旧别名。结构化 TUI 始终显示并保存带单位的新写法，effective config 与运行领域值仍稳定输出 `_ms` 整数，避免改变 API 和热更新比较语义。时长属于结构数值，不接受变量插值。

顶层 `task_templates` 声明不会直接运行的命名模板，Task 通过 `extends` 显式选择。模板本身也可继承另一个模板，形成从基础到专用的单继承链：

```yaml
task_templates:
  service:
    cwd: ./app
    env: {RUST_LOG: info}
    restart: on-failure
    healthcheck:
      http_get: {port: 8080, path: /ready}
  api:
    extends: service
    command: 'cargo run --bin api'
    env: {ROLE: api}

tasks:
  api:
    extends: api
    env: {PORT: "8080"}
```

模板链在全部 include 合并完成后、环境文件读取和 `task_defaults` 应用前解析。标量与列表由更高层整体替换，`env` 和 `depends_on` 按键合并；显式 `command` 会把程序与参数作为一个执行单元整体替换，不继承或追加基模板 argv。未知模板、自继承和循环链都会指向精确的 `.extends` 字段；未使用模板也独立校验。当前模式没有模板 map 键或标量的删除标记，需要移除的字段不应放在更基础的模板中。

顶层 `profiles` 用于命名不同运行场景，`profile` 把当前准入意图持久写在配置中：

```yaml
version: 1
project: demo
profile: dev

profiles:
  common:
    env: {LOG_FORMAT: pretty}
    task_defaults:
      restart: on-failure
  dev:
    extends: common
    tasks: [api, frontend]
    env: {APP_MODE: development}
  ci:
    extends: common
    tasks: [lint, test]
    env: {CI: "true"}
    task_defaults:
      max_restarts: 1

tasks:
  api: {command: [cargo, run, --bin, api]}
  frontend: {command: [npm, run, dev]}
  lint: {command: [cargo, clippy, --all-targets]}
  test: {command: [cargo, test, --all-features]}
```

profile 可用 `extends` 继承另一个命名 profile。继承链中的 `env` 和 `task_defaults.env` 按键组合，子层冲突键获胜；Task 白名单及其余默认标量/列表只在子层显式声明时整体替换。省略 `tasks` 表示继承基础 profile 的白名单，整条链都未声明时准入全部 Task；显式 `tasks: []` 则精确表示不准入任何 Task。Task 本地声明和命名模板不由 profile 改写，因此不会发生隐式 command/argv 拼接。未准入 Task 仍完成结构、模板、环境文件和运行字段校验，活动 Task 依赖未准入 Task 会在任务图编译时报错。未知 profile/基础 profile、自继承、循环链、重复/未知 Task 和未使用 profile 中的非法默认值都返回精确字段路径。

profile 继承在全部 include 合并后解析并在模板前应用；同名 profile 跨文件组合时，环境 map 按键合并，`extends`、Task 白名单和默认标量/列表由更高优先级文档显式值整体替换。profile 内的相对 `task_defaults.cwd` 仍以声明它的文件目录为基准。`procora config` 同时输出 `active_profile`、可选 profile 名称、`profile_extends` 直接继承映射和 `profile` 字段来源。

单文件 TUI 项目弹窗可循环选择 profile，独立 Profiles 区域可结构化新增、编辑、重命名和删除 profile 的继承目标、白名单、环境及默认层覆盖。重命名会同步活动选择和直接继承引用；仍被其他 profile 继承的基础项不能删除。任一变更确认后都会通过完整加载管线刷新活动 Task 与有效值预览，结构化保存不会展开继承值或丢失未准入 Task。包含真正 include 文档的入口继续使用 F2 高级文本模式，以保留每个声明的来源目录。

顶层 `vars` 提供显式、确定性的项目字符串变量：

```yaml
vars:
  ROOT: ./workspace
  BIN: cargo
  MODE: development
  API_DIR: ${vars.ROOT}/api

task_defaults:
  cwd: ${vars.API_DIR}
  env:
    APP_MODE: ${vars.MODE}

tasks:
  api:
    command: ["${vars.BIN}", run, --bin, api]
    healthcheck:
      http_get:
        host: localhost
        path: /${vars.MODE}/ready
```

引用只识别 `${vars.NAME}`；变量值可继续引用其他变量，解析顺序不依赖 map 声明顺序。未知变量、非法名称、未闭合引用和循环链都会指向定义或使用字段。`$${vars.NAME}` 删除一个 `$` 并保留字面量 `${vars.NAME}`；普通 `$VAR`、`${dependency.name}` 和其他花括号文本完全不处理。变量不读取 Procora 进程环境，不提供条件、函数或命令执行。

支持插值的字段限定为：项目/profile/`task_defaults` 的环境值和默认工作目录；Task 与模板的字符串命令、argv/`args` 元素、`cwd`、`env_file`、环境值；exec 健康检查的命令、参数和工作目录；HTTP 健康检查的 host、path 与请求头值。稳定身份和结构字段（`project`、profile/Task/模板/依赖名称、`extends`、`depends_on`、include 路径、数值与枚举）不接受变量，避免配置结构随字符串展开而改变。命令文本在插值后分词，因此变量需要保持单一参数边界时应使用 argv 数组。

变量解析发生在 include 合并之后、profile 与模板解析之前；同名变量按 include/入口优先级整体替换。路径表达式仍相对声明它的文件目录解析。变量不是新的覆盖来源：例如 profile 环境中的引用仍以 `profile` 为值来源，另由 `variable_references` 记录声明字段直接引用了哪些变量。`procora config` 输出原始 `vars`、链式解析后的 `resolved_vars` 和该引用映射。未使用 profile、模板和未准入 Task 中的变量引用同样校验。TUI 编辑变量后立即重编译预览，YAML/TOML/JSON 保存均保留原始引用，而不是把有效值写死。

配置编译结果还包含项目级 `dependencies`，由 `procora::source` 在 Task 启动前解析，不进入任务调度图。每项必须声明稳定名称、`source` 和 `version`，可选字段如下：

- `checksum`：SHA-256，接受纯 64 位十六进制或 `sha256:` 前缀。
- `unpack`：`auto`（默认）或 `never`。
- `kind`：`auto`、`binary`、`file` 或 `directory`。
- `path`：归档内要管理的相对路径。
- `verify.command`：安装根目录内的验证程序；省略时使用最终管理路径。
- `verify.args`：验证参数数组。
- `verify.contains`：标准输出与标准错误必须包含的文本；省略时使用 `version`。

来源支持 HTTP(S)、SSH URL、SCP 地址、本地 `file://` 和相对服务目录路径。Task 的 `command`、`args`、`env` 与 `cwd` 可用 `${dependency.<name>}` 引用已验证的绝对路径。版本目录固定为 `.procora/dependencies/<name>/<version>`；下载和解包先在临时目录完成，成功后再切换为正式安装。版本清单同时保存最终文件或目录的确定性内容指纹，因此本地损坏即使没有 `verify` 命令也会在下次同步时触发重新安装。

在中心服务器模型中，`project` 同时是本机服务稳定名称，必须满足 `ServiceName` 字符约束。服务目录和配置文件路径单独保存在中心注册表中，不写回领域配置。

### 3.2 任务

当前 Task 配置包含：

- 执行：`command`、`args`、`cwd`、`env_file`、`env`；`env_file` 在编译期合并进规范化 `TaskSpec.env`。
- 就绪：可选 `healthcheck` 及其时间和连续结果阈值。
- 依赖：`depends_on` 及每条边的满足条件。
- 生命周期：`restart`、`restart_delay`、`max_restarts`、`restart_reset_after`、`shutdown_timeout`、`success_exit_codes`。

`restart` 可取 `never`、`on-failure`、`always`。`restart_delay` 必须在 1ms–30s 之间，并按 30 秒封顶的指数退避应用。`max_restarts` 限制当前 generation 内的连续自动重启次数，默认 0 表示无限；达到上限后 Task 保持最终 `failed` 或 `exited` 状态，手动启动会清零计数。`restart_reset_after` 默认 1m，单次真实运行达到该稳定窗口后，下一次退出重新从首次退避和计数开始；`0ms` 表示永不自动重置，最大为 24h。`shutdown_timeout` 必须在 1ms–5m 之间，避免极端配置长期占用 Center 控制路径。相对 `cwd` 以配置文件所在目录解析并规范化，daemon 不修改自己的全局工作目录。

`success_exit_codes` 是非负整数数组，退出码 0 无论是否显式声明都始终视为成功。该结果同时用于 `on-failure` 重启判断和 `completed_successfully` 依赖，例如 `[0, 130]` 可把收到信号后返回 130 的程序视为正常结束。

全部使用默认 `started` 条件时，`depends_on` 可直接写成 Task 名称数组；需要不同条件时可使用名称到条件的紧凑 map：

```yaml
tasks:
  api:
    command: api
    depends_on: [database, cache]
  tests:
    command: tests
    depends_on:
      database: healthy
      migrate: completed_successfully
```

既有 `database: {condition: started}` 对象继续兼容。条件标量和对象中的 `condition` 都额外接受 process-compose 的 `process_started`、`process_healthy`、`process_completed_successfully` 输入别名，effective config 与 TUI 保存统一使用 `started`、`healthy`、`completed_successfully`。Procora 没有“任意退出码完成”条件，因此不会把上游 `process_completed` 错误映射为成功完成。名称数组只接受字符串并拒绝重复项，非法条件和旧版对象中的未知字段仍保留依赖字段路径诊断。

命令始终以可执行文件加参数数组启动，不经过隐式 shell。未声明 `args` 时，字符串 `command` 可直接包含参数；空白负责分词，单双引号保留含空格或空字符串的参数，反斜杠可转义空白和引号，普通 Windows 路径反斜杠保持原样：

```yaml
tasks:
  api:
    command: 'cargo run --release -- "hello world" "" C:\tools\api.exe'
```

这会规范化为程序 `cargo` 与参数 `run`、`--release`、`--`、`hello world`、空字符串、`C:\tools\api.exe`。`$VAR`、管道、重定向、`&&` 等内容不会被解释，只会成为普通参数。确需 shell 语法时，必须显式把 shell 作为程序，例如 `command: [sh, -c, "producer | consumer"]`。

既有字符串 `command` 加独立 `args` 写法继续保持兼容；一旦显式声明 `args`，整个字符串仍被视为程序名称，这也为含空格的旧版可执行路径提供稳定退出口。常用场景还可把完整 argv 写进一个字段，第一个元素是程序，其余元素原样成为参数：

```yaml
tasks:
  api:
    command: [cargo, run, --release, --, "hello world"]
```

argv 数组必须非空且只能包含字符串，不能再同时声明非空 `args`。它是无歧义的精确输入；命令文本、兼容写法和 argv 数组在 effective config 中始终展开为字符串 `command` 和数组 `args`。结构化 TUI 的命令字段也接受命令文本，独立参数字段继续优先使用 JSON 数组。

Task 可显式声明一个环境文件；Procora 不会自动读取服务目录中的 `.env`：

```yaml
tasks:
  api:
    command: ./api
    env_file: config/api.env
    env:
      LOG_LEVEL: debug
```

`env_file` 相对声明它的入口或 include 文件解析，必须留在服务根目录内，符号链接也不能越界。文件必须是 UTF-8 普通文件，支持空行、`#` 注释、可选 `export`、单/双引号和双引号内的 `\\`、`\"`、`\n`、`\r`、`\t` 转义；不执行变量替换。文件内重复键以后者为准，Task 的内联 `env` 最终覆盖文件值。单文件限制 1 MiB、4096 个变量，一次闭包中不同环境文件总量限制 4 MiB。

环境文件与 Task 共享编译、候选和应用语义：缺失、非法语法或越界会使候选无效；内容参与修订哈希并由本地监听器跟踪，变化会被归类为进程身份变化。Task 和健康检查会收到同一份合并结果。

编译结果同时保留 `env_file` 声明路径和 Task 内联环境层，不把它们混同为运行环境。结构化 TUI 因而可以在 YAML、TOML、JSON 单文件配置中直接编辑环境文件路径，保存时只写声明和内联覆盖，不会复制环境文件内容；包含真正 include 文档的入口继续使用高级文本模式以保留文件来源。

编译结果还逐 Task 保存来源矩阵。普通字段区分 `built_in`、`task_defaults`、`task_template` 与 `task`；最终环境变量和依赖边逐键区分项目默认、Task 默认、具体模板、`env_file` 与 Task 本地声明，覆盖后只记录真正生效的层，模板来源另附最终获胜名称。`procora config` 在输出完全展开值的同时输出这份 `origins`，因此显式 `restart: never`、模板/项目默认的 `restart: never` 与完全省略虽然运行语义一致，仍可被诊断和编辑器区分。结构化 TUI 保存时据此只写默认层和模板引用一次、不会向 Task 展开继承值，并保留用户明确写出的默认值；Task 弹窗把覆盖字段留空或将重启策略设为 `inherit`，会删除本地覆盖并恢复模板、项目或内建默认层。

`healthcheck` 是 readiness 语义，可在旧版兼容的 exec 探针与 `http_get` 中选择一种。exec 不经过 shell，默认继承 Task 的 `cwd` 和 `env`；退出码 0 表示成功，其他退出或超时表示失败：

```yaml
tasks:
  api:
    command: ./api
    healthcheck:
      command: ./api-health
      args: ["--ready"]
      initial_delay: 500ms
      period: 1s
      timeout: 300ms
      success_threshold: 2
      failure_threshold: 3
```

HTTP GET 探针使用相同的时间和连续阈值字段，并精确匹配声明的状态码：

```yaml
tasks:
  api:
    command: ./api
    healthcheck:
      http_get:
        scheme: http
        host: 127.0.0.1
        port: 8080
        path: /ready
        headers:
          X-Probe: procora
        status_code: 204
      period: 1s
      timeout: 300ms
      success_threshold: 2
      failure_threshold: 3
```

`http_get` 默认 `scheme: http`、`host: 127.0.0.1`、`path: /`、`status_code: 200`，端口可省略以使用协议默认值。HTTP 与 HTTPS 均受单次总超时约束，不跟随重定向；状态码必须在 100–399，主机、路径和请求头会在启动前统一校验。`command` 与 `http_get` 互斥，`args` 和 `cwd` 仅适用于 exec。

`initial_delay` 可为 `0ms`；`period` 和 `timeout` 必须在 1ms–5m 之间；连续成功和失败阈值必须在 1–100 之间。同一 Task 最多运行一个检查，下一次检查从上次完成后计时。exec 超时或 Task 停止时会回收整个检查进程树；阻塞中的 HTTP 请求不会拖住 Task 停止，后台请求仍受原超时和全局并发上限约束。

## 4. 任务依赖图

任务图必须是有向无环图。每条依赖边包含条件：

- `started`：上游已成功创建受监管进程。
- `healthy`：上游当前 run 达到健康检查连续成功阈值；未配置检查器时仍以成功创建受监管进程作为兼容降级条件。
- `completed_successfully`：上游一次性任务以退出码 0 完成。

首版不支持任意布尔表达式。一个任务的全部依赖默认采用 AND 语义，从而保持调度结果可解释。可选依赖、OR 组等能力需要单独设计状态传播规则后再增加。

结构化 TUI 的依赖字段使用 `task:condition` 逗号列表并接受上述条件别名。保存时，如果全部边都是 `started`，YAML/TOML/JSON 都写成名称数组；只要存在其他条件，就写成名称到规范条件的标量 map。模板和 Task 的依赖仍按名称合并，逐边来源不会因输入简写改变。

图编译阶段执行：

1. 校验任务 ID 唯一且符合规范。
2. 确认所有依赖目标存在。
3. 检测自依赖和环，并输出最短可读环路径。
4. 预计算拓扑序、反向依赖和停止顺序。
5. 校验依赖条件与上游任务类型是否合理。

依赖条件失效时，下游默认不会被立即强杀，而是进入策略控制的 `blocked` 或保持运行。首版采用保守规则：未启动的下游保持阻塞；已运行的下游记录依赖退化事件。是否级联停止必须由后续显式配置开启。

## 5. 默认值、合并与后续覆盖优先级

当前 include 与未来覆盖层统一采用以下从低到高的优先级：

```text
内建默认值
  < include 文件（从左到右）
  < 入口文件
  < 选中的 profile
  < 环境变量映射
  < CLI 显式覆盖
```

单个 Task 的有效字段优先级是内建默认值低于基础 `task_defaults`，再低于 profile 的 `task_defaults` 覆盖，之后是从基础到专用解析的命名模板链，最后是 Task 显式声明。进程环境另有固定优先级：Procora 进程的继承环境低于基础项目 `env`，再依次低于 profile 项目 `env`、基础 `task_defaults.env`、profile `task_defaults.env`、模板 `env`、有效 `env_file` 和 Task 内联 `env`。Procora 不存在自动 `.env` 层。

include 当前采用完整 Task、模板和依赖条目覆盖；profile 共享层已经按以下字段类型固定，未来 CLI 覆盖也必须遵循相同原则：

- 标量后者覆盖前者。
- map 按键合并；当前尚不提供删除标记。
- list 默认整体替换，避免隐式追加产生意外命令参数。
- 任务不能因同名合并而静默改变执行类型。

加载结果通过 `procora config` 输出“有效配置”和字段来源，便于用户理解某个值为何生效。当前来源粒度覆盖 profile、Task 字段、依赖边及最终环境变量；模板来源同时给出最终获胜的模板名称，变量引用单独保留声明字段路径和直接变量集合。未来 CLI 覆盖必须扩展同一通道，而不是另建无法组合的说明结构。敏感字段加入后，有效配置输出必须先脱敏。

## 6. 路径与变量边界

- 相对 `cwd` 已按声明它的配置文件所在目录解析，而不是以 daemon 当前工作目录解析。
- `${vars.NAME}` 只展开项目显式声明值；宿主环境变量展开和 `SecretValue` 仍是计划能力，加入后必须支持明确的缺失行为，且不得在 `Debug` 或序列化中泄漏密钥。

项目应尽量传递独立的 `cwd` 给进程，不修改 daemon 的全局当前目录。

## 7. Python 配置前端

Python 配置不嵌入核心引擎。只有用户显式传入文件名精确为 `procora.py` 的路径时才执行；目录发现永远只扫描四种声明式 `procora.*` 文件，不会自动执行脚本：

```text
procora.py
  → Python 3 隔离辅助进程
  → 仅 stdout 的单个 JSON 文档
  → RawProject
  → 与数据格式相同的校验和规范化管线
```

默认解释器为 Unix 的 `python3` 或 Windows 的 `python`，也可由嵌入方通过 `PythonConfigRunner` 显式注入。Procora 不经过 shell，以 `-I -S -X utf8` 启动解释器，清空继承环境，只传入固定上下文，关闭 stdin，并用进程组或 Job Object 托管整个进程树。脚本限制 1 MiB、执行限制 5 秒、stdout 限制 1 MiB、stderr 限制 256 KiB；超时、状态查询失败和退出后遗留后代都会触发整树回收。非零退出保留有界 stderr 诊断，stdout 必须是且只能是一个 JSON 文档。

生成结果不能声明 `include`，并且仍需通过与声明式格式相同的未知字段、语义、路径和任务图校验。脚本字节、本次生成的原始 stdout 和生成结果显式声明的 `env_file` 一起进入候选 SHA-256，因此 preview 与 apply 会重新执行并拒绝生成结果已经变化的过期修订。脚本自行读取但未声明为配置输入的其他业务文件不会自动加入监听集合；若它们改变，用户需要再次 preview，apply 阶段仍会通过重执行发现差异。

辅助进程的资源边界用于故障隔离，不是权限沙箱。脚本仍可按当前用户权限读取文件、访问网络或启动程序；CLI 在执行前给出警告，内置配置编辑器拒绝执行或改写 `procora.py`。只应对可信项目使用该入口。

## 8. 配置监听、拉取与应用

`DefinitionSource` 将本地文件、目录或未来远程来源统一为带版本输入：

- `LocalFileSource`：原子读取入口、完整 include 与显式环境文件闭包，递归监听服务根目录并只接收闭包成员事件。
- `DirectorySource`：按明确顺序发现项目文件，不依赖操作系统目录遍历顺序。
- `GitSource`：获取分支、标签或提交后解析为完整不可变 commit，把受限 checkout 交给同一配置层；只返回候选，不注册或启动服务。
- `HttpSource`：计划能力，使用 ETag/内容哈希并限制大小与重定向。

监听事件需去抖动，并以重新读取完整配置为准，不能依赖文件系统事件本身表达最终内容。应用流程为：

1. 拉取候选输入并编译。
2. 与当前 `ProjectSpec` 做语义差异比较。
3. 输出新增、删除、重启、原地更新和无影响任务集合。
4. 当前本地来源等待 `procora apply` 显式确认；未来来源策略不得绕过准入。
5. 提交新修订并由引擎对账。

`success_exit_codes`、`restart`、`restart_delay_ms`、`max_restarts`、`restart_reset_after_ms` 和 `shutdown_timeout_ms` 可原地更新；放宽已耗尽 Task 的重启上限会从当前连续计数继续调度。命令、参数、环境、工作目录、健康检查或依赖边变化归入重启集合，并把影响传播到下游。应用只停止重启与删除集合，保留无影响 Task 的原运行身份，再按新图启动重启与新增集合；删除 Task 按旧图反向依赖顺序停止。

应用前会再次读取磁盘并核对完整修订，防止 preview 与 apply 之间的 TOCTOU 覆盖。配置编译和管理依赖准备都发生在停止旧 Task 之前；候选 Task 启动失败时会清理候选进程、恢复旧图及受影响 Task，无影响 Task 始终保留。文件事件使用容量为一的合并通道和 250ms 静默窗口，事件本身不被当作最终内容。

### 8.1 Git 定义源

`GitSource::remote` 只接受无内嵌凭据的 HTTPS、SSH URL 或 SCP 写法；本地仓库必须通过单独的 `GitSource::local` 显式授权。Git 以清空继承环境、关闭终端交互、禁用全局/系统配置、凭据 helper、hooks 和远端 file 协议的辅助进程运行；本地构造器只为该次可信来源开放 file 协议。引用字符先做白名单校验，禁止选项、refspec 和外部 remote-helper 注入。

每次获取使用 30 秒命令上限和 blobless 浅 fetch；对象库在执行期间限制为 128 MiB，`git archive` stdout 限制为 40 MiB，展开后限制 32 MiB、4096 个普通文件，持久 checkout 缓存限制 256 MiB。归档沿用依赖解包的路径逃逸防护，不创建符号链接或特殊文件；相同 commit 的缓存内容或可执行位变化会被完整性检查拒绝。远端配置只能是相对的 YAML/TOML/JSON 入口，不能执行 `procora.py`。

候选身份同时包含仓库、完整 commit 和配置闭包修订。`fetch_candidate` 只获取并编译；调用方展示差异后，应把用户确认的修订交给 `confirm_candidate`，由它重新获取并拒绝已经前移的引用或变化的配置。`procora source git preview/confirm` 已暴露同一条只读闭环；Center 的持久远端注册、实际应用和凭据代理仍需单独设计，不能用本地自动应用语义替代。

## 9. 校验与兼容性

- 顶层配置必须声明 `version`。
- 未知核心字段默认报错，避免拼写错误被忽略；扩展字段只允许出现在命名空间中。
- 同一模式版本的 YAML、TOML、JSON 与 Python 生成 JSON 应产生等价领域语义；命令文本、字符串加 `args` 旧写法、argv 数组、依赖数组/标量/对象、模板和 profile 继承表示必须共享规范化结果。
- `tests/fixtures/config/equivalent/` 固定保存旧版重复声明与新版 `task_defaults`、命令文本、argv、依赖简写、可读时长、命名模板、profile/继承、变量的 YAML、TOML、JSON、Python 输出；任何模式演进都必须继续通过同一 `ProjectSpec` 与任务图断言。
- 配置升级由显式迁移器完成，不能在运行时猜测旧字段含义。
- `procora validate` 应能在不启动 daemon、下载依赖或启动任务的情况下执行完整编译；`procora deps --check` 负责离线安装验证。
- `procora config effective` 应输出脱敏后的有效配置及来源说明。

具体测试矩阵见[测试策略](testing.md)。
