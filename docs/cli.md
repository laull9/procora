# CLI 与全局 Procora 服务器语义

## 全托管裸机部署

`deploy` 通过 SSH 上传一个完整 Service。远端只需安装 Procora 并允许当前 SSH 用户运行它；不需要预先创建服务目录、执行 `add` 或在配置中声明 `uploads` target：

```bash
procora deploy . --ssh prod --dry-run
procora deploy . --ssh prod
procora deploy
procora deploy ./service --ssh user@server --service demo
procora deploy . --ssh prod --timeout 45s --stable-for 5s --keep 5
procora deploy . --ssh prod --batch
procora deploy ./demo.pcpkg --ssh prod
```

第一次连接建议先执行普通 `ssh prod`，人工核对并保存主机指纹；确认 `ssh prod procora __ssh-probe` 能返回 JSON 后再部署。日常开发可省略 `--batch`，Procora 会在密钥不可用时交给 OpenSSH 从控制终端读取密码。CI 和 MCP 必须使用密钥、agent 或 SSH config 完成非交互认证，并启用 `--batch` 语义。

本机先发现并完整编译声明式配置，服务名取自 `project`。`--service` 只做身份一致性校验，不是远端路径或上传目标。`--dry-run` 仍会建立只读 SSH 连接、探测平台并在本机生成临时归档，但不会调用远端部署接收器；输出包含配置入口、二进制选择、内容大小、预期 release 和可供自动化确认的完整修订。

非 `--batch` 部署成功后，CLI 会在全局 Procora 数据目录的 `cli-memory/deploy.json` 中按规范化 Service 根目录和 `project` 记住 SSH 目标及远端 Procora 路径，不保存密码。之后在同一项目运行 `procora deploy` 会直接复用；显式参数始终优先，`PROCORA_SSH_TARGET` 优先于记忆。`--batch` 不读取或写入这份交互记忆，CI 应显式传入目标或环境变量。

远端在当前用户的 Procora 数据目录下保存：

```text
services/<project>/
  releases/<archive-sha256-prefix>/
  state.json
  deploy.lock
```

部署接收器会再次校验 SHA-256、展开大小、配置入口和 `project`，再把 release 注册到 Center。命令行会实时显示 `[校验]`、`[切换]`、`[验活]`、`[回滚]` 和 `[恢复]` 阶段。相同 release 已经注册且处于运行状态时返回 `changed=false`，不切换、不重启也不追加部署记录；若该 Service 已停止、失败或注册被移除，则相同内容仍会重新启动和验收。全部 Task 必须进入运行状态；配置健康检查的 Task 还必须达到 `healthy`，并持续通过 `--stable-for` 稳定窗口。Task 启动失败、健康检查失败或 `--timeout` 到期都会停止新 release，恢复上一 release，并再次执行相同验收。首次部署失败时会停止失败服务。若旧 release 也无法恢复，命令明确报告自动回滚失败。

部署和回滚是固定程序状态机，不调用 AI，也不根据日志文本猜测成功。release 切换由 Center 在同一服务身份内完成，并用旧根目录作为并发校验，避免删除注册记录和短暂的未注册窗口。切换前会把待部署 release 写入两阶段状态；若接收器或 SSH 会话在中途异常退出，下一次部署会先识别未完成事务并恢复最近一次已确认 release。没有配置健康检查的运行中 Task 以受管进程仍在运行为降级验收条件；需要强健康保证的服务应声明 exec 或 HTTP GET healthcheck。

同名服务只有在其根目录确实属于上述 Procora 托管 release 目录时才能被后续 `deploy` 更新。用户通过 `add` 注册的同名普通目录不会被接管。`--keep` 默认保留最近 3 个 release，范围为 1–32；部署记录保存在 `state.json`，最多保留最近 100 条。

本机与远端 Procora 都需要支持 `deploy`。远端版本过旧且缺少托管接收器时，客户端会直接提示升级远端 Procora。

`deploy` 的来源也可以是 `.pcpkg`。客户端先探测远端平台，再从包中物化唯一匹配变体，因此开发机与远端平台可以不同，未命中的二进制不会进入部署归档。包的构建、验证、本机安装和导出项见 [Procora Service 包](packages.md)。

### 部署后的远端管理

成功部署后，可在同一项目目录直接管理记住的 SSH 目标，也可随时用 `--ssh` 覆盖：

```bash
procora remote ps
procora remote status
procora remote logs demo api
procora remote history demo
procora remote start demo
procora remote restart demo
procora remote stop demo
procora remote rm demo

procora remote ps --ssh another-host
procora remote --ssh another-host logs demo api
```

`ps` 列出远端 Center 当前托管的 Service；`logs` 读取指定 Task 的当前活动日志；其他命令复用 Procora 已有的状态、历史和生命周期语义。`rm` 只停止并移除远端 Center 注册，不删除不可变 release 与 `state.json`，下一次部署可重新注册。Service 和 Task 参数先按领域标识规则校验，再作为固定参数交给 OpenSSH，不接受路径或任意 shell 文本。`--remote-bin`、`--batch`、主机确认、密钥/一次性内存密码回退和常见远端安装路径发现与 `deploy` 一致；`--batch` 不使用当前项目的部署目标记忆。

### 三平台预编译二进制

开发机与远端不需要使用相同系统或 CPU。先用项目自己的构建系统或 CI 生成各平台二进制；Procora 不参与编译，只消费已经存在的普通文件。例如构建目录可以是：

```text
dist/
  api-linux-amd64
  api-macos-arm64
  api-windows-amd64.exe
```

再把远端平台映射到这些文件。Windows 可在对象形式中覆盖 release target，使 Task 自动拿到 `.exe` 路径：

```yaml
version: 1
project: api

binaries:
  api:
    target: bin/api
    variants:
      linux-amd64: dist/api-linux-amd64
      macos-arm64: dist/api-macos-arm64
      windows-amd64:
        source: dist/api-windows-amd64.exe
        target: bin/api.exe

tasks:
  api:
    command: "${binary.api}"
```

这里的 `source` 是开发机上的构建输出，文件名没有约束；顶层 `target` 是默认 release 路径，变体对象中的 `target` 只覆盖该平台。`${binary.api}` 在远端启动前替换为当前 release 的绝对路径：Linux/macOS 得到 `bin/api`，Windows 得到 `bin/api.exe`。归档只包含命中的那一个文件，其他平台产物以及源码目录中的旧 target 都会被排除。

| 远端探测结果 | 命中键 | 写入release |
| --- | --- | --- |
| Linux x86-64 glibc | `linux-amd64` | `bin/api` |
| Linux x86-64 musl | `linux-x86_64-musl`，没有时回退 `linux-amd64` | `bin/api` |
| macOS Apple Silicon | `macos-arm64`，也可用 `macos-universal` | `bin/api` |
| macOS Intel | `macos-amd64`，也可用 `macos-universal` | `bin/api` |
| Windows x86-64 MSVC | `windows-amd64` | 变体声明的 `bin/api.exe` |
| Windows ARM64 MSVC | `windows-arm64` | 通常声明 `bin/api.exe` |

平台键使用 `os-arch` 或 `os-arch-environment`。支持 `amd64`/`x64` → `x86_64`、`arm64` → `aarch64`、`darwin`/`osx` → `macos`、`glibc` → `gnu` 等无歧义别名。`x86` 始终表示 32 位，不会被猜成 `x86_64`。Linux 精确 ABI 项优先于同 OS/架构的通用项。`macos-universal`/`macos-universal2` 只表示同时包含 Intel 与 Apple Silicon 代码的 macOS universal binary；它的优先级低于精确架构项，不能用于 Linux/Windows 或附带 ABI。

客户端会显示探测平台和每个命中的变体。缺少匹配项、选中源文件不存在、target 冲突或远端版本不支持平台探测时，命令在构造归档和切换服务前失败。归档固定二进制可执行权限；远端还会按自身平台重新选择配置、核对 target、大小和 SHA-256，平台及摘要同时写入 `state.json` release 记录。

### 参数怎么选

| 参数 | 默认值 | 建议 |
| --- | --- | --- |
| `--ssh` | 交互询问 | CI 中始终显式提供 SSH config 别名或`user@host` |
| `--service` | 不断言 | 发布脚本中填入预期 project，防止路径指错服务 |
| `--timeout` | `30s` | 应覆盖最慢 Task 启动和健康检查时间，上限10分钟 |
| `--stable-for` | `2s` | 无状态服务通常 2–10 秒；不要大于 timeout |
| `--keep` | `3` | 保留 3–5 个便于审计和快速恢复 |
| `--remote-bin` | `procora`并自动查找 | 远端非交互 PATH 特殊时显式填写 |
| `--batch` | 关闭 | CI 使用；禁止主机确认、密码询问和路径询问 |
| `--dry-run` | 关闭 | 发布前输出确定性计划，不上传或切换远端 |

建议至少给对外服务声明 HTTP 或 exec healthcheck。没有健康检查时，Procora 只能确认受管进程在稳定窗口内没有退出，无法判断端口、数据库连接或应用内部就绪状态。

单个二进制不能是空文件，完整归档的压缩大小和展开内容各有 8 GiB 上限。超限会在预检或远端接收正文前失败，不会留下半个 release。

### 失败时从哪里看

- “没有适用于远端平台的变体”：按错误中的规范化平台键补一项；不要把 `x86` 当成 x86-64。
- “预检后平台发生变化”：目标主机、远端 Procora 或 Linux libc 探测结果发生了变化，重新执行部署。
- “远端 Procora 不支持协议”：先升级远端 Procora；客户端不会降级成缺少摘要校验的部署。
- “选中产物不存在”：先运行构建矩阵；未命中的其他平台产物可以不存在。
- “SSH 自动登录失败”：交互终端先用普通 SSH 建立信任；CI 检查 key、agent、known_hosts 和 SSH config。
- “新 release 验活失败”：命令输出会继续报告回滚及旧 release 再验收结果；应用日志仍在对应 release 的 Service 日志目录中。

`deploy` 与 `push` 的边界不同：

| 命令 | 内容 | 远端是否声明 target |
| --- | --- | --- |
| `deploy` | 包含 Procora 配置的完整 Service | 不需要 |
| `push` | 已有 Service 的单个文件或目录 | 需要 |

## 远端声明式上传

服务端配置可在 Service 或 Task 下声明 `uploads`，客户端通过稳定选择器上传：

```bash
# 完整 CLI：直接执行，不打开 TUI
procora push ./assets --target demo::assets --ssh prod
procora push ./target/release/api --target demo::api::release --ssh user@server --restart
procora push demo.pcpkg --package-entry assets --ssh prod

# 参数不完整：交互终端打开内联引导
procora push
procora push ./assets
procora push ./assets --ssh prod

# 远端非交互 shell 找不到 procora 时显式给出安装路径
procora push ./assets --ssh prod --remote-bin ~/.local/bin/procora
procora push ./assets --ssh windows-prod --remote-bin C:/工具/Procora/procora.exe

# 检查本机或远端当前生效的上传项、类型、上限和 Service 内路径
procora uploads
procora uploads --ssh prod
procora uploads --ssh prod --json --batch
```

来源、`--target` 和 `--ssh` 都完整时不会打开 TUI。缺少来源时，小 TUI 可选择直接输入、终端路径浏览器或系统原生文件/文件夹选择器；缺少 SSH 目标时会列出上次使用值、`PROCORA_SSH_TARGET`、选择器推断值和 `~/.ssh/config` 中不含通配符的 Host；缺少上传选择器时，会在同一 SSH 会话中拉取与来源类型和大小兼容的活动上传项，并展示选择器、Service 内相对路径、类型、上限与远端默认重启策略。首次没有记忆时，运行行为默认遵循远端配置，不由客户端强制开启重启。

PATH 正常命中时，目标发现、选择和归档传输在同一条 SSH 会话内完成，不做额外预探测。Procora 先以 `BatchMode=yes` 和严格 known_hosts 自动认证；只有 OpenSSH 以 255 表示连接或认证失败，且标准输入/错误连接终端时，才提示用户确认或修改 SSH 目标并启动普通 OpenSSH。远端 Procora 命令不存在时，会用能力握手自动检查 Unix/macOS 的 `~/.local/bin`、`~/bin`、`/usr/local/bin`、Homebrew 与系统目录，以及 Windows 常见命令和用户安装位置；仍未找到时，交互终端允许用户直接输入远端路径，`--batch` 则要求显式传入 `--remote-bin <PATH>`。Center 离线或目标配置错误不会被误判为密码问题。密码完全由 OpenSSH 从控制终端读取，不支持密码命令行参数，也不会写入 Procora 配置、日志或引导记忆。

成功上传后，非敏感的来源方式、上次来源、SSH 目标、远端 Procora 路径、上传选择器和重启偏好会写入全局 Procora 数据目录的 `cli-memory/push.json`，只用作下一次交互默认值；现场目录不会生成记忆文件，显式 CLI 参数始终优先。`--restart` 可为本次上传显式开启自动重启，远端上传目标也可用 `restart: true` 声明默认策略；两者任一开启时，Procora 都只在目标原子提交成功后重启所属 Service。两处都未开启时默认不重启。`--batch` 禁止登录和目标选择交互，缺少本机来源时直接报错，适合 CI。

远端必须运行同一用户的 Center，上传目标从当前已经 apply 的有效配置解析。只保存在磁盘、尚未 apply 的候选声明不会提前生效。默认远端命令为 `procora`；自动查找仍无法定位时，可用 `--remote-bin ~/.local/bin/procora` 或 `--remote-bin C:/工具/Procora/procora.exe` 指定不含空格的 Unicode 命令路径。Windows 旧控制台或远端 shell 返回 GBK/GB18030 诊断时会自动恢复为中文；能力握手、目标清单和传输协议仍要求 UTF-8 JSON，编码损坏会明确失败而不会猜测协议。

SSH 互传不要求两端 Procora 包版本完全一致，而按传输协议范围和本次使用的能力校验。当前普通覆盖使用兼容协议 1，可与旧接收端互传；CLI 显式 `--restart` 使用协议 2，旧接收端不支持时会报告所需能力和双方协议边界，不会静默忽略重启。当前接收端兼容协议 1–2，因此旧客户端仍能上传，并可由远端配置的 `restart: true` 决定是否重启。`procora __ssh-probe` 返回机器可读的协议范围和能力列表，便于部署诊断。

## 自动更新

```bash
# 只检查，不下载或覆盖
procora update --check

# 下载、校验并安装当前平台的最新正式 Release
procora update

# 通过 GitHub 镜像下载
procora update --github-mirror https://mirror.example

# 交给自定义程序下载，程序依次接收 URL 和输出路径
procora update --download-command /path/to/procora-fetch
```

`procora update` 查询 GitHub 最新正式 Release，按语义版本比较当前版本，选择与安装脚本相同的六平台发布产物，并在 128 MiB 上限内流式下载。归档下载期间会在标准错误显示百分比、已下载量、总量和平均速度；Release 未提供大小时仍显示已下载量和速度。交互终端使用单行刷新，重定向或 CI 日志按秒节流，不污染标准输出中的版本结果。

只有同名 `.sha256` 校验通过、归档中恰好只有 `procora` 或 `procora.exe` 普通文件时才会安装。Linux/macOS 在当前可执行文件目录中暂存并原子替换；Windows 启动已验证的新版本助手，在旧进程退出后完成可恢复替换和暂存清理。安装目录不可写时会保留旧版本并提示修正权限。

如果更新前全局 Center 正在运行，新版本会在可执行文件替换后自动对账并正常重启 Center；原先离线时不会因为更新而隐式启动。`PROCORA_REPO=owner/repo` 可与安装脚本一样改为 fork 的 Release 来源。`--github-mirror URL` 覆盖 `PROCORA_GITHUB_MIRROR`，支持在完整 GitHub URL 前增加 HTTPS 前缀，或用 `{url}` 模板决定插入位置；非 GitHub 地址不会被意外改写。`--download-command PROGRAM` 覆盖 `PROCORA_DOWNLOAD_COMMAND`，程序固定接收 `URL OUTPUT` 两个参数，不经过 shell 解释。该程序负责写入目标文件，Procora 继续限制文件大小、显示进度并执行 SHA-256 校验。

## 1. 固定层级

Procora 的内部模型固定为 `Center → Service → Task`；界面和命令行将 Center 称为“全局 Procora 服务器”：

- Center 是当前用户级唯一后台协调进程，维护服务注册表和本地 IPC。
- Service 由规范化目录、被选中的配置文件和配置内 `project` 名称共同确定。
- Task 只在所属 Service 内唯一，由该服务的 `Engine` 调度；Task 不能脱离 Service 被中心服务器直接托管。

中心服务器负责多服务路由，`ServiceHost` 负责单服务的运行组合，`Engine` 负责单服务内部的 Task 规则。三者不能合并成一个全局任务表。

## 2. 默认入口

`procora` 默认打开当前用户全局中心的服务总览；`procora PATH` 继续直接打开指定服务目录或显式配置文件：

1. 无参数时确保当前用户的全局 Procora 服务器运行，并列出全部已注册 Service 的状态、Task 数量、目录和配置入口。
2. 总览使用上下方向键或 `j/k` 选择，`/` 按名称、路径、状态和说明实时筛选，`o` 在名称、状态、CPU、内存之间循环排序，`O` 切换升降序；列表、页头和详情同时展示 Service 聚合 CPU/内存，其中资源排序默认高占用优先。
3. `s/x/r` 启动、停止或重启，连续两次 `d` 移除注册；`Enter` 进入原有 Task 详情 TUI，详情中的 `q/Esc` 返回总览。
4. 显式 PATH 时优先按规范化目录连接已注册服务；目录尚未注册时，在全局服务器中发现配置并注册服务。
5. 显式 PATH 且全局服务器不存在时，仍询问启动全局服务或创建仅与本次 TUI 同生命周期的临时 `ServiceHost`。

总览需要交互终端；脚本应使用 `procora list`。显式 PATH 的非交互环境不会擅自选择运行模式，应使用 `procora up` 或 `procora temp-run [path]`。需要持久注册服务时仍使用 `procora add <path>`。

连接全局服务器后，TUI 通过协议控制服务并按游标读取事件和日志；临时模式直接连接当前进程中的服务宿主。两种模式提供相同的主要交互。Task、依赖和表单等字段型文本只有超出区域宽度时才响应左右方向键或 `h/l`；日志解析常见 ANSI/SGR 颜色并使用统一的大文本视口，任一日志行需要横移时全部行使用同一偏移。日志页持续捕获鼠标，纵向滚轮滚动正文，横向滚轮或 `Shift` 加纵向滚轮移动文本；使用 `/` 搜索、`n/N` 跳转匹配、`f` 过滤匹配行，尚无搜索词时按 `f` 可直接输入过滤词，`v` 切换“全部 / Procora / 子进程”来源，连续两次 `C` 确认清空当前 Task 日志。Task 运行错误会同时保留为特殊样式的 Procora 日志和详情页“综合分析”。`F3` 开关当前页面的自动横移，速度固定为每秒 4 个终端显示列；字段型文本在自动模式下仍只移动实际溢出的行。自动模式下手动横移会冻结当前高亮文本 10 秒，其他溢出文本继续滚动。

### 自适应终端布局

TUI 会在终端 Resize 后立即重新计算布局，不要求退出重进：

- 宽屏使用列表/详情主从布局；中等宽度改为上下排列。
- 宽度低于 48 列或高度低于 16 行时，总览、Task 页和包工作台进入单列紧凑视图，只显示当前选中对象、状态、主操作、`?` 帮助和退出路径。`j/k`、方向键、`Tab`、`Enter` 等交互继续有效，不会因为列表被折叠而失去导航能力。
- 包工作台与总览、Task 页共用 `←/→` 手动横移、触控板横向滚动和 `F3` 自动横移；长路径、摘要、错误与底栏反馈不再只能看到截断开头。包文件使用连续两次 `Delete` 或大写 `X` 删除，安装项使用 `U U` 解除、`D D` 永久删除，三种动作在底栏和帮助层分别显示。
- 配置表单低于 72 列或 18 行时一次只显示当前项目、Task、依赖或 Profile 区域；用 `Tab`/`Shift-Tab` 切换区域，当前区域详情和 `Enter`、`n/d`、`Ctrl-S` 操作固定显示在其下方。高级文本模式优先保留正文、光标、保存和退出。
- 帮助、运行方式选择、路径浏览、字段编辑和键值表弹层会在窄屏使用完整可用宽度，压缩为单行键位说明并截断次要描述；`Esc` 取消、`Enter` 确认和 `Ctrl-S` 保存始终优先显示。
- 极小终端不再尝试绘制破碎边框，而是显示“终端过小”及 `Ctrl-S`、`Esc` 或 `q` 恢复入口。放大窗口后完整页面会自动恢复。

窄屏中的省略号只表示显示被折叠，并不代表配置、路径或包清单被修改；支持横移的字段可用左右方向键查看，其余内容在放大窗口后恢复。任何危险操作的二次确认规则也不会因紧凑布局改变。

已知命令优先于同名路径；同名路径使用 `./status` 这类显式路径。命令和子命令支持唯一前缀推断，接近但不唯一或拼写错误的输入会显示最相近命令；运行期错误统一提示通过 `procora --help` 查看完整用法。

## 3. 项目初始化与中心进程

- `procora init --config yaml|json|toml`：在当前目录写入不依赖 Cargo 的可运行示例；默认 YAML。已有同名文件时拒绝覆盖，只有显式 `--force` 才覆盖。交互式终端会自动进入配置编辑页，脚本使用 `--no-edit` 跳过。
- `procora edit [path]`：发现唯一声明式配置并默认打开结构化 TUI 表单。项目弹窗可编辑项目变量、基础项目环境、`task_defaults`，并在已声明 profile 与基础配置之间循环切换；独立 Profiles 区域可新增、编辑、重命名和删除 profile，包括继承目标、可空/显式空 Task 白名单、环境及默认层覆盖。变量或 profile 改变后立即重编译活动 Task 和有效来源预览，保存时保留变量表达式、未准入 Task 及 profile 原始声明；重命名会同步活动选择和直接继承引用，仍被继承的 profile 不能删除。项目卡片显示变量解析数、活动 profile、准入 Task 数和模板数；Task 弹窗可填写 `extends` 选择模板、预览具体来源并只保存局部覆盖，依赖字段接受 `task:condition` 及 process-compose 条件别名，保存时规范化为数组或标量 map；生命周期与健康检查时长接受 `750ms`、`5s`、`1m30s` 并保存为带单位的新字段，健康检查弹窗根据 `none`、`exec`、`http` 类型只显示适用字段；模板定义使用 F2 高级文本编辑。Task 命令字段可直接输入带引号的完整命令文本，保存时规范化为精确 argv。新建和保存 Task 不会复制继承值；覆盖字段留空、或把重启策略设为 `inherit`，会删除本地覆盖并恢复模板/profile/项目/内建默认。通过 `Tab` 或 `Shift-Tab` 切换区域，上下方向键在列表边界自动跨区，左右方向键只横移当前高亮的溢出文本，`F3` 开关全局自动横移；`Enter` 打开编辑，Task 弹窗内 Enter 不再提交，统一用 `Ctrl-S` 校验、写盘并退出；`n` 新建、`d` 二次确认删除。Task 弹窗 Esc 会按本轮字段差异询问保存、放弃或取消，整个编辑页退出也会按全局脏状态弹出相同选择。`F1` 在配置有效时返回表单。内置编辑器不执行或改写 `procora.py`，Python 入口应使用可信的外部代码编辑器。
- `procora temp-run [path]`：显式创建与当前 TUI 同生命周期的临时服务，不连接或注册到全局服务器。
- `procora clean [path]`：删除服务目录下的 `.procora` 运行时目录，包括日志和管理依赖缓存；省略路径时使用当前目录。配置文件和其他项目文件不会删除，目录不存在时正常返回。
- `procora deps [path]`：同步项目声明依赖；`--check` 仅依据版本清单、目标类型和版本命令离线复核。
- `procora up`：确保当前用户的全局 Procora 服务器运行，并输出服务数量。
- `procora down`：发送正常关闭请求并等待端点退出；保留中心 SQLite 状态和每个 Service 自己的日志。
- `procora status`：只探测并显示状态，不隐式启动全局服务器。
- `procora logs <target> <task>`：以 64 KiB 有界分片按文件游标读取并立即输出指定 Task 的活动日志，不把全部内容聚合为单个 IPC 包或内存缓冲；保留原始 ANSI 颜色，`--search TEXT` 跨分片按完整行匹配并附原始行号，`--filter TEXT` 只输出匹配行，`--clear` 清空活动日志和该 Task 的全部 gzip 轮转归档。三个操作参数互斥，Center 未运行时不会隐式启动。
- `procora enable`：正常关闭已有的手动 Center，把内部前台 daemon 注册到当前平台的用户级原生托管器，并立即启动。
- `procora disable`：正常关闭 Center，停止并移除当前用户的自启动注册；不删除 SQLite 状态和 Service/Task 日志。
- `procora completions <shell>`：把 Bash、Zsh、Fish、PowerShell 或 Elvish 补全脚本写到标准输出，不启动 Center。用户可按 shell 约定保存或 `source` 该输出。
- `procora mcp`：通过 stdio 运行本地 MCP 服务，向可信客户端提供配置查询、服务生命周期工具和内嵌文档 Prompts；不监听网络端口。完整接口见 [MCP 本地服务](mcp.md)。
- `procora config <path>`：输出原始/解析变量及逐字段引用、活动 profile、可选 profile/模板列表、`profile_extends` 直接继承映射、项目环境、`task_defaults`、命令文本/argv 简写、其他默认值和目录规范化全部展开后的稳定有效配置 JSON，并在 `origins` 中说明各 Task 字段、依赖边及最终环境变量来自内建默认、项目 env、Task 默认层、profile、具体命名模板、env_file 还是 Task；不会下载依赖、注册服务或启动 Task。
- 结构化配置编辑页中，Task 的“工作目录覆盖”字段可按 F5 打开跨平台目录浏览器；选定结果优先保存为相对配置目录的 `cwd`。
- `procora source git preview <repository> [--reference REF] [--config PATH]`：受限获取 Git 引用，输出完整 commit、组合修订和配置校验结果；`--local` 才允许显式本地仓库。不会启动 Center、注册服务或运行 Task。
- `procora source git confirm <repository> <revision> [...]`：按相同来源参数重新获取，只有仓库、commit 与配置闭包修订仍匹配时成功；仍不自动应用。默认缓存位于当前用户 Procora 数据目录，也可用 `--cache` 覆盖。

自启动在 Linux 使用 `systemd --user`，在 macOS 使用 LaunchAgent，在 Windows 使用当前用户的登录触发计划任务。三者都直接监管内部前台 daemon，不通过会再次派生进程的 `procora up`。因此原生托管器能正确观察退出和失败，并在崩溃时按平台定义恢复。

这些注册都以“当前用户登录”为启动时机。Windows 的任务注册和移除会显式唤起 UAC，并在用户取消或权限不足时返回明确诊断；任务本身仍以当前用户受限权限运行。Linux 若要求用户尚未登录时也在系统启动阶段运行，需要由管理员单独配置该用户的 linger；`procora enable` 不会擅自修改这个用户级系统策略。升级后若可执行文件位置发生变化，需要在新二进制下重新执行 `procora enable`。

## 4. 服务注册与发现

`procora add <path>` 会确保全局服务器运行，然后把路径交给它处理。路径是文件时只编译该显式文件；文件名精确为 `procora.py` 时，CLI 会先提示可信代码执行，再由受控 Python 辅助进程生成 JSON。路径是目录时只扫描第一层的 `procora.yaml`、`procora.yml`、`procora.toml`、`procora.json`，绝不会自动执行 Python。其他 YAML、TOML、JSON 文件不会进入候选集合。

发现结果必须满足以下一种情况：

- 一个合法配置：注册并进入运行期望。
- 多个合法配置：拒绝猜测，要求用户传入显式文件路径。
- 没有合法配置：返回候选文件的失败摘要；没有候选时返回未发现错误。

服务名称与服务目录是一一对应关系。同名服务不能注册到两个仍存在的配置目录，同一目录也不能静默改成另一个名称。但旧注册的配置入口已经消失时，显式打开同名新目录会停止旧宿主并安全迁移该条记录。若配置入口仍存在且确需改名，应通过显式移除/迁移命令完成，而不是隐式覆盖。

## 5. 定位规则

`show` 和生命周期命令接受名称或路径；`show` 省略目标时使用当前目录：

- 已存在路径、绝对路径、`.`、`..` 或包含路径分隔符的输入按路径处理。
- 其他输入按配置中的 `project` 名称处理。
- 路径先规范化，符号链接和 macOS 路径别名因此会收敛到同一服务。
- 服务根目录内的子目录也会匹配该服务；存在嵌套服务时选择最接近的根目录。

按路径查看未注册项目时，`show` 会发现配置并注册服务。按名称查看失效旧记录时，只有当前目录项目的 `project` 与目标名称一致才会自动迁移，不会误注册其他项目。显式持久托管仍可使用 `add <path>`。

## 6. 生命周期命令

- `start`：先停止旧宿主中的真实 Task，重新加载已注册配置，再按依赖条件启动新的运行代次。
- `restart`：按反向依赖顺序停止真实 Task，重新加载配置并替换当前 `ServiceHost`，再按拓扑顺序启动。
- `preview`：原子重读配置入口，以内容 SHA-256 标识候选，并输出新增、删除、重启、原地更新和无影响 Task；不下载依赖、不停止进程。
- `apply <target> <revision>`：应用精确匹配已预览 SHA-256 的有效候选；磁盘内容变化、配置无效、项目改名或依赖准备失败时拒绝应用并保留旧宿主。
- `stop`：按反向依赖顺序优雅停止真实 Task；超过各 Task 宽限期后强制回收进程树，同时保留注册信息。
- `remove`：停止并从注册表删除服务，但不删除服务目录、配置和日志。
- `list`：稳定按服务名称排序，输出状态、Task 数量、目录和配置文件；全局服务器未运行时只报告离线状态。
- `history`：按名称或路径查询 SQLite 中的状态变更历史；不读取日志文件，也不隐式启动全局服务器。

旧版 `procora server ...` 入口作为隐藏兼容层继续解析相同动作，但新脚本和文档统一使用顶层命令。

中心服务器使用当前用户数据目录中的 `procora.sqlite3` 保存注册表、服务当前状态和状态历史。测试和隔离环境可通过 `PROCORA_HOME` 覆盖目录；本地 IPC 端点从该目录派生，以避免不同用户或隔离环境互相连接。

SQLite 不保存日志正文。Service 日志固定写入自身目录的 `.procora/logs/service.log`，Task 日志写入 `.procora/logs/tasks/<task>.log`；压缩归档也留在该 Service 目录，不汇总到 Center 数据目录。清空 Task 日志会推进文件代次，使现有读取游标收到 Gap 后从清空后的新内容安全恢复。

Center 使用跨平台独占文件锁保证同一 `PROCORA_HOME` 只有一个实例。本地协议先进行版本握手，服务变化进入容量为 256 的内存事件缓冲；慢客户端游标过期后必须重取快照，不能把不连续事件误当作完整状态。

Git 来源缓存使用独立跨进程锁。远端只接受无内嵌凭据的 HTTPS/SSH/SCP；Git 命令关闭交互、全局配置、hooks 和危险协议，并在 fetch 期间实施对象库上限。CLI 只完成 preview/confirm，不能把远端候选直接交给 `apply`；Center 持久来源与凭据边界落地前，这个限制是有意的安全准入。

Center 递归监听配置入口所在的服务根目录以兼容编辑器原子替换写入，容量为一的事件通道会合并突发通知，静默 250ms 后才重读完整文件。回调只接受当前闭包成员，监听只产生 `config_candidate_changed` 事件，不会绕过用户确认自动执行文件内容。

配置含 `include` 时，监听范围扩展到服务根目录，但只接受当前闭包成员和暂时缺失目标的事件；其他业务文件与 `.procora` 日志不会触发候选。候选修订覆盖闭包全部相对路径与原始字节，因此成员文件在 preview 与 apply 之间变化也会被拒绝。

Python 来源只监听 `procora.py`；每次 preview/apply 都重新执行脚本，并把脚本和生成 stdout 共同计入修订。脚本自行读取的其他文件不会触发监听事件，但它们导致的输出变化会使 apply 拒绝旧修订。超时、输出越界、非零退出或非法 JSON 只产生无效候选，不替换旧宿主。

## 7. 当前状态含义

服务状态是宿主级状态：

- `running`：配置已成功编译，`ServiceHost` 已装配并正在对账真实 Task，服务具有运行期望。
- `stopped`：服务仍在注册表中，但没有运行期望。
- `failed`：恢复或生命周期操作时无法加载配置。

每个 Task 的 `pending/blocked/running/stopped/failed`、日志和资源独立展示，不能用服务状态覆盖 Task 状态。嵌入模式下按 `q`、Esc 或 Ctrl-C 退出会先反向停止全部 Task，再恢复终端。
