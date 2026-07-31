# Python API 与脚本化包构建

Procora 随二进制内嵌一份支持 Python 3.9+、零第三方依赖的 Python 包。官方安装脚本在检测到 Python 3 时自动运行 `procora python install`，把包物化到 Procora 用户数据目录，并在当前解释器的用户 `site-packages` 写入 `procora.pth`。因此不需要 pip、虚拟环境或额外网络下载；Python API 与承载它的 Procora 二进制始终来自同一版本。

如果安装时尚无 Python，安装器只给出提示，不影响 CLI。之后可执行：

```bash
procora python install
procora python path
python -c "import procora; print(procora.version())"
```

指定解释器时使用 `procora python install --interpreter /path/to/python`。普通解释器写入用户 `site-packages`，虚拟环境写入该环境自身的 `site-packages`。卸载器会自动移除 `.pth`；也可显式运行 `procora python uninstall`。

## 1. 约定式配置

在 Service 根目录创建 `procora.py`：

```python
from procora import Project

app = Project(
    "api",
    env={"APP_ENV": "development"},
    task_defaults={"shutdown_timeout": "5s"},
)

app.task("database", ["docker", "compose", "up", "postgres"])
app.task(
    "api",
    ["python", "-m", "my_api"],
    cwd=".",
    env={"PORT": 8080},
    depends_on={"database": "started"},
    healthcheck={"http_get": {"port": 8080, "path": "/ready"}},
    restart="on-failure",
)
```

不需要 `print()` 或 `main`：Procora 设置受控配置模式后，唯一 `Project` 会在脚本正常结束时自动输出严格 JSON。`Project` 还提供：

- `task()`：命令、环境、依赖、健康检查与重启策略；命令既可写字符串，也可写 argv 数组。
- `template()` 与 `profile()`：Task 模板和 profile 覆盖。
- `dependency()`：管理依赖原始声明。
- `binary()`：逻辑二进制及多平台变体。
- `upload()`：项目级包导出与上传目标。
- `to_dict()` / `emit()`：检查或显式输出生成配置。

所有结果仍进入与 YAML/TOML/JSON 完全相同的 Rust 严格解析、默认值、路径规范化、任务图和循环依赖校验。Python API 不复制运行时语义，也不会绕过未知字段检查。

复杂配置可拆到同目录普通模块并由 `procora.py` 导入。隔离运行器固定让官方 `procora` 包优先于脚本目录，因此项目文件不会用同名模块遮蔽官方 API；用户全局 `site-packages` 不参与配置执行。

目录发现会同时识别 `procora.py`、`procora.yaml`、`procora.yml`、`procora.toml` 和 `procora.json`。若多个入口都合法，Procora 与声明式配置一样拒绝猜测，要求显式传入文件路径。

`procora.py` 是可信代码，会以当前用户权限执行。辅助进程会清空继承环境、关闭 stdin、限制执行时间和输出大小，并在超时后回收进程树，但它不是安全沙箱。

## 2. 从 Python 构建包

最短入口使用模块命令；默认输出 `dist/<service>.pcpkg`：

```bash
python -m procora .
python -m procora . --platform current
python -m procora . --prepare "python scripts/generate.py" --force
```

自动化脚本可直接使用结构化结果：

```python
from procora import build

result = build(
    source=".",
    platform="all",
    prepare=["python scripts/generate.py"],
)

print(result.path)
print(result.package_digest)
print(result.files, result.binary_variants, result.package_bytes)
```

`build()` 只负责组合稳定参数并调用同版本 `procora package build --json`；文件收集、`.procoraignore`、跨平台路径校验、内容寻址、确定性归档和构建后自校验仍由 Rust 实现。可用 `PROCORA_BIN` 或 `procora_bin=` 指定 CLI；默认从 `PATH` 定位。

也可把构建放在独立 `build.py` 中。不要直接用 `python procora.py` 构建，因为入口文件名会与安装的 `procora` 包同名；应使用 `python -m procora`、独立脚本或 `procora package build`。

## 3. Python MCP 客户端

标准库同步客户端适合不想引入 MCP SDK 的构建脚本：

```python
from procora import McpClient

with McpClient() as client:
    print([tool["name"] for tool in client.tools()])
    result = client.call("validate_project", path=".", allow_python=True)
    print(result["project"])
```

`McpClient.call()` 返回 `structuredContent`，工具错误转换为 `McpError`。客户端通过 stdio 启动 `procora mcp`，不监听网络端口。

MCP 对 Python 入口默认拒绝执行。只有调用方明确确认脚本可信并传入 `allow_python: true` 后，配置校验、服务注册、包构建或部署工具才会执行 `procora.py`。
