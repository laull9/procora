# 发布与安装

## 发布目标

Procora 不制作 deb、rpm、pkg、dmg、msi 等平台原生安装包。`dev` 和 `main` 的每次 push 各执行一轮完整 CI，`dev → main` PR 不重复运行；发布标签只能指向已通过 CI 的 `main` 最新提交。每个 `v*` 标签只触发 `.github/workflows/release.yml`，复用该 `main` CI 结果并为下列六个目标构建、打包和发布 `procora`，不再重复执行源码测试：

| 系统 | x86_64 | ARM64 |
| --- | --- | --- |
| Linux | `x86_64-unknown-linux-musl` | `aarch64-unknown-linux-musl` |
| macOS | `x86_64-apple-darwin` | `aarch64-apple-darwin` |
| Windows | `x86_64-pc-windows-msvc` | `aarch64-pc-windows-msvc` |

Unix 目标发布 `procora-<target>.tar.gz`，Windows 目标发布 `procora-<target>.zip`。每个归档旁都包含同名 `.sha256` 文件。流水线也可手动运行，但必须显式输入要创建或更新的 Release 标签。

Linux 使用静态 musl 目标，发布二进制没有 ELF `NEEDED` 项和动态加载器，不依赖目标机器的 glibc 或 musl 共享库。Windows 通过 `crt-static` 静态链接 MSVC/UCRT 运行时，并检查 PE 导入表不含 `MSVCP*`、`VCRUNTIME*`、`ucrtbase.dll` 或 `api-ms-win-crt-*`；Windows 内核与系统 API DLL 仍由操作系统提供。macOS 固定最低部署版本 11.0，允许的动态依赖只限 `/usr/lib` 和 `/System/Library` 中的 Apple 系统库与框架。

每个发布构建在打包前检查最终二进制，而不只依赖 Cargo 配置：Linux 使用 `readelf`，macOS 使用 `otool`，Windows 使用 Visual Studio `dumpbin`。任何意外动态依赖都会阻止该平台产物上传。

工作流中的 JavaScript Actions 统一使用 Node 24 版本并固定完整提交 SHA。构建产物首次上传失败时会等待 10 秒后覆盖重试一次；重试仍失败则终止发布，不会带着缺失平台的产物创建 Release。

## 一键安装

macOS/Linux 安装脚本检测系统和架构，下载对应 tar.gz，验证 SHA-256，并默认安装到 `$HOME/.local/bin/procora`：

```bash
curl --fail --location --proto '=https' --tlsv1.2 https://raw.githubusercontent.com/laull9/procora/main/scripts/install.sh | sh
```

Windows PowerShell 脚本使用运行时架构选择 zip，验证 SHA-256，并默认安装到 `%LOCALAPPDATA%\Procora\bin\procora.exe`：

```powershell
irm https://raw.githubusercontent.com/laull9/procora/main/scripts/install.ps1 | iex
```

两个脚本都支持以下环境变量：

- `PROCORA_VERSION`：默认 `latest`；可设为 `v0.3.0` 等固定标签。
- `PROCORA_INSTALL_DIR`：覆盖默认安装目录。
- `PROCORA_REPO`：覆盖默认 GitHub 仓库 `laull9/procora`，用于 fork 或发布演练。

安装脚本不会自动修改 PATH，也不会擅自注册后台托管。首次安装后若命令不可见，用户需要把安装目录加入自己的 PATH；需要登录后自动运行 Center 时，由用户显式执行 `procora enable`，卸载前可执行 `procora disable`。

## 一键卸载

卸载脚本先运行 `procora disable`，成功后只删除安装目录中的命令，保留数据库、运行状态和各 Service 日志：

```bash
curl --fail --location --proto '=https' --tlsv1.2 https://raw.githubusercontent.com/laull9/procora/main/scripts/uninstall.sh | sh
```

```powershell
irm https://raw.githubusercontent.com/laull9/procora/main/scripts/uninstall.ps1 | iex
```

自定义安装目录时必须向卸载脚本传入同一个 `PROCORA_INSTALL_DIR`。如果停用开机自启动失败，脚本默认中止；仅在确认无需保留后台托管时设置 `PROCORA_FORCE_UNINSTALL=1` 强制删除命令。

## 发布操作

1. 在 `dev` 提交源码并确认该 push 的完整 CI 成功。
2. 在 `dev` 更新 `Cargo.toml` 和 `Cargo.lock` 版本，推送并确认这一次 CI 成功。
3. 通过 `dev → main` PR 以 merge commit 合入；PR 本身不再触发重复 CI。
4. 确认 `main` merge commit 的唯一一轮 CI 成功。
5. 仅在 `main` 最新提交创建并单独推送与 `Cargo.toml` 版本一致的 `v*` 标签；发布工作流校验已成功的 `main` CI 后直接打包。
6. 确认六组构建产物和校验文件全部进入同一个 GitHub Release。
7. 至少在每个平台各验证一次脚本安装、`procora --help`、`procora up/status/down`。
