# 配置示例

本文把常用规则、局部写法和一份完整配置集中在同一处。示例以 YAML 展示；TOML、JSON 入口与 include 片段在“跨格式 include”一节给出。每个代码块都可以独立复制到对应字段或文件中。

## 1. 最小入口

入口文件必须有 `version`、`project` 和 `tasks`。`project` 是服务稳定名称，只能使用 ASCII 字母、数字、点、短横线和下划线。

```yaml
version: 1
project: demo

tasks:
  api:
    command: [procora, doctor]
```

## 2. 命令与参数

Task 不经过隐式 shell 启动。推荐用 argv 数组保留参数边界；也可使用带引号的命令文本，或兼容的 `command` 加 `args` 写法。

```yaml
tasks:
  argv:
    command: [cargo, run, --release, --, "hello world"]
  command_text:
    command: 'cargo run --release -- "hello world"'
  explicit_args:
    command: cargo
    args: [run, --release]
```

```text
规则：`command` 是 argv 数组时不能同时声明非空 `args`；
      `$VAR`、管道、重定向和 `&&` 不会被执行，确需 shell 时显式把 shell 写成程序。
```

## 3. Task 依赖

全是 `started` 时可使用名称数组；需要不同条件时使用名称到条件的 map。可选条件为 `started`、`healthy`、`completed_successfully`。

```yaml
tasks:
  database:
    command: [procora, doctor]
  migrate:
    command: [procora, doctor]
    depends_on:
      database: healthy
  api:
    command: [procora, doctor]
    depends_on:
      database: healthy
      migrate: completed_successfully
  worker:
    command: [procora, doctor]
    depends_on: [database]
```

```text
规则：活动 Task 不能依赖被当前 profile 排除的 Task；依赖图不能形成环。
```

## 4. 环境、变量与环境文件

`vars` 只支持 `${vars.NAME}` 引用，解析不读取宿主环境。环境优先级为项目 `env` < profile `env` < `task_defaults.env` < 模板 `env` < `env_file` < Task 内联 `env`。

```yaml
vars:
  ROOT: .
  MODE: development

env:
  APP_MODE: ${vars.MODE}
  LOG_LEVEL: warn

task_defaults:
  cwd: ${vars.ROOT}
  env:
    SERVICE_REGION: local

tasks:
  api:
    command: [procora, doctor]
    env_file: config/api.env
    env:
      LOG_LEVEL: debug
```

```dotenv
# config/api.env
FROM_ENV_FILE=enabled
LOG_LEVEL=info
```

```text
规则：Procora 不会自动读取 `.env`；`env_file` 必须位于服务根目录内，Task 内联 env 覆盖同名文件值。
```

## 5. 生命周期与健康检查

时间字段使用 `ms`、`s`、`m`、`h`，并可组合。健康检查可选择 exec 或 HTTP GET，二者共享检查节奏和阈值。

```yaml
tasks:
  database:
    command: [procora, doctor]
    restart: always
    restart_delay: 750ms
    max_restarts: 5
    restart_reset_after: 1m
    shutdown_timeout: 5s
    success_exit_codes: [0, 130]
    healthcheck:
      command: procora
      args: [doctor]
      initial_delay: 500ms
      period: 2s
      timeout: 1s
      success_threshold: 1
      failure_threshold: 3
```

```yaml
tasks:
  api:
    command: [procora, doctor]
    healthcheck:
      http_get:
        host: 127.0.0.1
        port: 8080
        path: /ready
        headers:
          X-Probe: procora
        status_code: 204
      period: 2s
      timeout: 500ms
```

```text
规则：healthcheck 只表达 readiness；失败不会隐式重启 Task，重启由 `restart` 策略决定。
```

## 6. 默认值、模板与 profile

`task_defaults` 为所有 Task 提供默认层；`task_templates` 可被 Task 用 `extends` 继承；profile 可继承并控制当前运行的 Task 白名单。

```yaml
task_defaults:
  restart: on-failure
  shutdown_timeout: 5s

task_templates:
  service:
    env:
      SERVICE_FAMILY: backend
  http-service:
    extends: service
    healthcheck:
      http_get:
        host: 127.0.0.1
        port: 8080
        path: /ready

profiles:
  common:
    env:
      LOG_FORMAT: pretty
  dev:
    extends: common
    tasks: [database, api]
    env:
      HOT_RELOAD: "true"
  ci:
    extends: common
    tasks: [lint, test]
    env:
      CI: "true"

tasks:
  api:
    extends: http-service
    command: [procora, doctor]
```

```text
规则：map 按键合并，标量和列表由更高层整体替换；Task 覆盖字段留空或在 TUI 选择 `inherit` 可恢复默认/模板值。
```

## 7. 管理依赖

简单来源只需一行。需要固定版本、镜像、摘要、下载策略或私有请求头时再展开对象。`${dependency.NAME}` 在 Task 启动前解析为验证后的绝对路径。

```yaml
dependencies:
  helper: https://downloads.example.com/helper.tar.gz
  schema:
    source: https://artifacts.example.com/schema-v2.json
    mirrors:
      - https://backup.example.com/schema-v2.json
    version: v2
    checksum: sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef
    unpack: never
    kind: file
    path: schema-v2.json
    download:
      retries: 3
      timeout: 90s
      max_bytes: 10485760
      headers:
        Authorization: Bearer ${env.ARTIFACT_TOKEN}

tasks:
  api:
    command: [procora, doctor]
    env:
      SCHEMA_PATH: ${dependency.schema}
```

```text
规则：远程制品应固定 `checksum`；`${env.NAME}` 只在下载时读取 Procora 进程环境，秘密不会写入安装清单。
```

## 8. 跨格式 include

入口可组合服务根目录内的 YAML、TOML、JSON 片段，按 `include` 顺序加载，入口最后覆盖。片段可省略 `version` 和 `project`。

```toml
# fragments/base.toml
[task_defaults]
restart = "on-failure"

[tasks.database]
command = ["procora", "doctor"]

[tasks.api]
command = ["procora", "doctor"]

[tasks.api.depends_on]
database = "healthy"
```

```json
// fragments/development.json
{
  "profiles": {
    "dev": {
      "tasks": ["database", "api", "gateway"],
      "env": {"APP_MODE": "development"}
    }
  }
}
```

```yaml
# procora.yaml
include:
  - fragments/base.toml
  - fragments/development.json
version: 1
project: include-example
profile: dev

tasks:
  gateway:
    command: [procora, doctor]
    depends_on:
      api: healthy
```

```text
规则：include 必须留在服务根目录内；循环、父目录逃逸、符号链接逃逸、超过 16 层或过大闭包都会被拒绝。
```

## 9. 📄 综合配置

以下入口把前述能力放在一个可校验的文件中。将两个代码块分别保存为同目录的 `procora.yaml` 和 `comprehensive.env` 后，可运行 `procora validate procora.yaml`；远程依赖地址仅用于展示结构，执行 `procora deps` 前请替换为真实制品。

<!-- 配置块：comprehensive.env -->
```dotenv
# comprehensive.env
FROM_ENV_FILE=enabled
LOG_LEVEL=info
```

<!-- 配置块：comprehensive.yaml -->
```yaml
version: 1
project: comprehensive-example
profile: dev

vars:
  ROOT: .
  MODE: development

env:
  APP_MODE: ${vars.MODE}
  LOG_LEVEL: warn

task_defaults:
  cwd: ${vars.ROOT}
  restart: on-failure
  restart_delay: 750ms
  max_restarts: 5
  restart_reset_after: 1m
  shutdown_timeout: 5s

task_templates:
  service:
    env:
      SERVICE_FAMILY: backend
    success_exit_codes: [0, 130]
  http-service:
    extends: service
    healthcheck:
      http_get:
        host: 127.0.0.1
        port: 8080
        path: /ready
        headers:
          X-Probe: procora
        status_code: 204
      period: 2s
      timeout: 500ms

profiles:
  common:
    env:
      LOG_FORMAT: pretty
  dev:
    extends: common
    tasks: [database, migrate, api, worker]
    env:
      HOT_RELOAD: "true"
  ci:
    extends: common
    tasks: [lint, test]
    env:
      CI: "true"
    task_defaults:
      max_restarts: 1

dependencies:
  helper: https://downloads.example.com/helper.tar.gz
  schema:
    source: https://artifacts.example.com/schema-v2.json
    mirrors:
      - https://backup.example.com/schema-v2.json
    version: v2
    checksum: sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef
    unpack: never
    kind: file
    path: schema-v2.json
    download:
      retries: 3
      timeout: 90s
      max_bytes: 10485760
      headers:
        Authorization: Bearer ${env.ARTIFACT_TOKEN}

tasks:
  database:
    command: [procora, doctor]
    restart: always
    healthcheck:
      command: procora
      args: [doctor]
      initial_delay: 500ms
      period: 2s
      timeout: 1s
      success_threshold: 1
      failure_threshold: 3
  migrate:
    command: [procora, doctor]
    depends_on:
      database: healthy
  api:
    extends: http-service
    command: [procora, doctor]
    env_file: comprehensive.env
    env:
      LOG_LEVEL: debug
      SCHEMA_PATH: ${dependency.schema}
    depends_on:
      database: healthy
      migrate: completed_successfully
  worker:
    extends: service
    command: [procora, doctor]
    env:
      HELPER_PATH: ${dependency.helper}
    depends_on: [database]
  lint:
    command: [procora, doctor]
  test:
    command: [procora, doctor]
    depends_on:
      lint: completed_successfully
```
