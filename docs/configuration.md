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

include 合并和闭包内容修订已插入 Raw DTO 与领域规范之间；profile 与通用变量展开尚未实现，加入时同样不能绕过统一校验。

当前各阶段使用不同类型，避免未校验字段误入运行时：

- `RawProject`：保留可选字段与用户写法，用于产生精确错误位置。
- `ProjectSpec`：默认值已补全、路径已解析、标识符已规范化的领域对象。
- `TaskGraph`：完成依赖解析和环检测的可执行图。

`DefinitionRevision` 使用入口和全部 include 文件的相对路径、原始字节及无效诊断共同计算 SHA-256；统一承载不同类型远程来源的 `ConfigInput` 仍是后续抽象。

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

include 按列表从左到右加载，后一个文件覆盖前一个文件，入口最后覆盖全部片段。`tasks` 和 `dependencies` 以完整条目为覆盖单位，不把同名 Task 的命令与另一文件的参数隐式拼接；跨文件依赖边在全部合并完成后统一校验。片段可省略 `version` 和 `project`，但一旦声明就必须与入口一致；入口自身必须显式声明两者。

每个片段中的 `cwd`、健康检查 `cwd` 和本地管理依赖来源先按该片段目录重定位，再进入合并。include 路径必须是服务根目录内、不含 `.` 或 `..` 的相对普通路径；符号链接规范化后也不能逃逸根目录。单次闭包限制为 16 层、64 个文档、合计 4 MiB，循环、缺失文件、身份冲突和未知格式均使整个候选无效。

## 3. 规范模型草案

### 3.1 项目

当前 `ProjectSpec` 包含：

- `version`：配置模式主版本。
- `project`：项目稳定标识。
- `tasks`：以 `TaskId` 为键的任务定义。

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

当前 `TaskSpec` 包含：

- 执行：`command`、`args`、`cwd`、`env`。
- 就绪：可选 `healthcheck` 及其时间和连续结果阈值。
- 依赖：`depends_on` 及每条边的满足条件。
- 生命周期：`restart`、`restart_delay_ms`、`shutdown_timeout_ms`、`success_exit_codes`。

`restart` 可取 `never`、`on-failure`、`always`。`restart_delay_ms` 必须在 1–30000 毫秒之间，`shutdown_timeout_ms` 必须在 1–300000 毫秒之间，避免极端配置长期占用 Center 控制路径。相对 `cwd` 以配置文件所在目录解析并规范化，daemon 不修改自己的全局工作目录。

`success_exit_codes` 是非负整数数组，退出码 0 无论是否显式声明都始终视为成功。该结果同时用于 `on-failure` 重启判断和 `completed_successfully` 依赖，例如 `[0, 130]` 可把收到信号后返回 130 的程序视为正常结束。

命令始终以可执行文件加参数数组启动，不经过隐式 shell。确需 shell 语法时，用户必须把 `sh`、`bash` 或 `powershell` 作为显式 `command`，并自行提供参数。

健康检查同样不经过 shell，并默认继承 Task 的 `cwd` 和 `env`。检查程序退出码 0 表示成功，其他退出或超时表示失败：

```yaml
tasks:
  api:
    command: ./api
    healthcheck:
      command: ./api-health
      args: ["--ready"]
      initial_delay_ms: 500
      period_ms: 1000
      timeout_ms: 300
      success_threshold: 2
      failure_threshold: 3
```

`initial_delay_ms` 可为 0；`period_ms` 和 `timeout_ms` 必须在 1–300000 毫秒之间；连续成功和失败阈值必须在 1–100 之间。同一 Task 最多运行一个检查，下一次检查从上次完成后计时。超时或 Task 停止时会回收整个检查进程树。

## 4. 任务依赖图

任务图必须是有向无环图。每条依赖边包含条件：

- `started`：上游已成功创建受监管进程。
- `healthy`：上游当前 run 达到健康检查连续成功阈值；未配置检查器时仍以成功创建受监管进程作为兼容降级条件。
- `completed_successfully`：上游一次性任务以退出码 0 完成。

首版不支持任意布尔表达式。一个任务的全部依赖默认采用 AND 语义，从而保持调度结果可解释。可选依赖、OR 组等能力需要单独设计状态传播规则后再增加。

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
  < 未来项目 defaults
  < 选中的 profile
  < 环境变量映射
  < CLI 显式覆盖
```

include 当前采用完整 Task/依赖条目覆盖；未来 profile、环境和 CLI 覆盖仍必须按字段类型固定：

- 标量后者覆盖前者。
- map 按键递归合并，支持显式删除标记。
- list 默认整体替换，避免隐式追加产生意外命令参数。
- 任务不能因同名合并而静默改变执行类型。

加载结果应能输出“有效配置”和字段来源，便于用户理解某个值为何生效。有效配置输出必须对敏感字段脱敏。

## 6. 路径与计划变量

- 相对 `cwd` 已按声明它的配置文件所在目录解析，而不是以 daemon 当前工作目录解析。
- 逻辑路径来源、环境变量展开与 `SecretValue` 仍是计划能力；加入后必须支持明确的缺失变量行为，且不得在 `Debug` 或序列化中泄漏密钥。

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

生成结果不能声明 `include`，并且仍需通过与声明式格式相同的未知字段、语义、路径和任务图校验。脚本字节与本次生成的原始 stdout 一起进入候选 SHA-256，因此 preview 与 apply 会重新执行并拒绝生成结果已经变化的过期修订。脚本读取的其他业务文件不会自动加入监听集合；若它们改变，用户需要再次 preview，apply 阶段仍会通过重执行发现差异。

辅助进程的资源边界用于故障隔离，不是权限沙箱。脚本仍可按当前用户权限读取文件、访问网络或启动程序；CLI 在执行前给出警告，内置配置编辑器拒绝执行或改写 `procora.py`。只应对可信项目使用该入口。

## 8. 配置监听、拉取与应用

`DefinitionSource` 将本地文件、目录或未来远程来源统一为带版本输入：

- `LocalFileSource`：原子读取入口和完整 include 闭包，递归监听服务根目录并只接收闭包成员事件。
- `DirectorySource`：按明确顺序发现项目文件，不依赖操作系统目录遍历顺序。
- `GitSource`：获取分支、标签或提交后解析为完整不可变 commit，把受限 checkout 交给同一配置层；只返回候选，不注册或启动服务。
- `HttpSource`：计划能力，使用 ETag/内容哈希并限制大小与重定向。

监听事件需去抖动，并以重新读取完整配置为准，不能依赖文件系统事件本身表达最终内容。应用流程为：

1. 拉取候选输入并编译。
2. 与当前 `ProjectSpec` 做语义差异比较。
3. 输出新增、删除、重启、原地更新和无影响任务集合。
4. 当前本地来源等待 `server apply` 显式确认；未来来源策略不得绕过准入。
5. 提交新修订并由引擎对账。

`success_exit_codes`、`restart`、`restart_delay_ms` 和 `shutdown_timeout_ms` 可原地更新；命令、参数、环境、工作目录、健康检查或依赖边变化归入重启集合，并把影响传播到下游。应用只停止重启与删除集合，保留无影响 Task 的原运行身份，再按新图启动重启与新增集合；删除 Task 按旧图反向依赖顺序停止。

应用前会再次读取磁盘并核对完整修订，防止 preview 与 apply 之间的 TOCTOU 覆盖。配置编译和管理依赖准备都发生在停止旧 Task 之前；候选 Task 启动失败时会清理候选进程、恢复旧图及受影响 Task，无影响 Task 始终保留。文件事件使用容量为一的合并通道和 250ms 静默窗口，事件本身不被当作最终内容。

### 8.1 Git 定义源

`GitSource::remote` 只接受无内嵌凭据的 HTTPS、SSH URL 或 SCP 写法；本地仓库必须通过单独的 `GitSource::local` 显式授权。Git 以清空继承环境、关闭终端交互、禁用全局/系统配置、凭据 helper、hooks 和远端 file 协议的辅助进程运行；本地构造器只为该次可信来源开放 file 协议。引用字符先做白名单校验，禁止选项、refspec 和外部 remote-helper 注入。

每次获取使用 30 秒命令上限和 blobless 浅 fetch；对象库在执行期间限制为 128 MiB，`git archive` stdout 限制为 40 MiB，展开后限制 32 MiB、4096 个普通文件，持久 checkout 缓存限制 256 MiB。归档沿用依赖解包的路径逃逸防护，不创建符号链接或特殊文件；相同 commit 的缓存内容或可执行位变化会被完整性检查拒绝。远端配置只能是相对的 YAML/TOML/JSON 入口，不能执行 `procora.py`。

候选身份同时包含仓库、完整 commit 和配置闭包修订。`fetch_candidate` 只获取并编译；调用方展示差异后，应把用户确认的修订交给 `confirm_candidate`，由它重新获取并拒绝已经前移的引用或变化的配置。`procora source git preview/confirm` 已暴露同一条只读闭环；Center 的持久远端注册、实际应用和凭据代理仍需单独设计，不能用本地自动应用语义替代。

## 9. 校验与兼容性

- 顶层配置必须声明 `version`。
- 未知核心字段默认报错，避免拼写错误被忽略；扩展字段只允许出现在命名空间中。
- 同一模式版本的 YAML、TOML、JSON 输入应产生等价 `ProjectSpec`。
- 配置升级由显式迁移器完成，不能在运行时猜测旧字段含义。
- `procora validate` 应能在不启动 daemon、下载依赖或启动任务的情况下执行完整编译；`procora deps --check` 负责离线安装验证。
- `procora config effective` 应输出脱敏后的有效配置及来源说明。

具体测试矩阵见[测试策略](testing.md)。
