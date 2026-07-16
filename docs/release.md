# 发布与安装

## 发布目标

Procora 不制作 deb、rpm、pkg、dmg、msi 等平台原生安装包。每个 `v*` 标签只触发 `.github/workflows/release.yml`；格式、Clippy、测试、文档、MSRV、依赖安全和平台测试全部通过后，流水线才会为下列六个目标构建并发布 `procora`：

| 系统 | x86_64 | ARM64 |
| --- | --- | --- |
| Linux | `x86_64-unknown-linux-gnu` | `aarch64-unknown-linux-gnu` |
| macOS | `x86_64-apple-darwin` | `aarch64-apple-darwin` |
| Windows | `x86_64-pc-windows-msvc` | `aarch64-pc-windows-msvc` |

Unix 目标发布 `procora-<target>.tar.gz`，Windows 目标发布 `procora-<target>.zip`。每个归档旁都包含同名 `.sha256` 文件。流水线也可手动运行，但必须显式输入要创建或更新的 Release 标签。

工作流中的 JavaScript Actions 统一使用 Node 24 版本并固定完整提交 SHA。构建产物首次上传失败时会等待 10 秒后覆盖重试一次；重试仍失败则终止发布，不会带着缺失平台的产物创建 Release。

## 一键安装

macOS/Linux 安装脚本检测系统和架构，下载对应 tar.gz，验证 SHA-256，并默认安装到 `$HOME/.local/bin/procora`：

```bash
curl --fail --location --proto '=https' --tlsv1.2 https://raw.githubusercontent.com/laull/procora/main/scripts/install.sh | sh
```

Windows PowerShell 脚本使用运行时架构选择 zip，验证 SHA-256，并默认安装到 `%LOCALAPPDATA%\Procora\bin\procora.exe`：

```powershell
irm https://raw.githubusercontent.com/laull/procora/main/scripts/install.ps1 | iex
```

两个脚本都支持以下环境变量：

- `PROCORA_VERSION`：默认 `latest`；可设为 `v0.3.0` 等固定标签。
- `PROCORA_INSTALL_DIR`：覆盖默认安装目录。
- `PROCORA_REPO`：覆盖默认 GitHub 仓库 `laull/procora`，用于 fork 或发布演练。

安装脚本不会自动修改 PATH，也不会擅自注册后台托管。首次安装后若命令不可见，用户需要把安装目录加入自己的 PATH；需要登录后自动运行 Center 时，由用户显式执行 `procora enable`，卸载前可执行 `procora disable`。

## 发布操作

1. 确认版本号、`Cargo.lock` 和文档已经提交。
2. 本地运行格式、Clippy、测试和文档检查。
3. 创建并单独推送 `v*` 标签（普通 `git push` 不会推送标签），或在 Actions 中手动运行 release workflow 并输入已存在的标签。
4. 确认六组构建产物和校验文件全部进入同一个 GitHub Release。
5. 至少在每个平台各验证一次脚本安装、`procora --help`、`procora up/status/down`。
