# 项目约束

## 分支与发布

- `dev` 是唯一的日常开发与集成分支；本地功能和修复直接在 `dev` 开发、提交并推送，不创建短期开发分支，也不从其他分支向 `dev` 合并。
- `dev` 禁止强推；`main` 只保存可发布版本，禁止直接提交或强推。
- 发布时由 `dev` 向 `main` 提 PR 并使用 merge commit；合入后仅在 `main` 创建与 `Cargo.toml` 版本一致的 `vX.Y.Z` 标签。
- 紧急修复同样直接在 `dev` 完成并按正常发布流程进入 `main`，不从 `main` 或其他分支回合到 `dev`。
- `Cargo.toml` 的 `[package].version` 是当前 Procora 版本的唯一事实来源；代码、测试、工作流和文档凡需引用当前版本，必须通过 `env!("CARGO_PKG_VERSION")`、`cargo metadata` 或由清单动态读取，禁止在 User-Agent、标签和输出断言中硬编码当前发布版本。
- 调整版本时只直接修改 `Cargo.toml` 并同步 `Cargo.lock`，随后搜索旧版本号，确认剩余命中仅为有明确测试语义的历史版本或协议夹具。
- 推送前必须通过 `cargo fmt --all -- --check`、`cargo clippy --locked --all-targets --all-features -- -D warnings` 和 `cargo test --locked --all-features`。

## 代码规范

- 注释和文档使用中文；关键 trait、结构体、函数及静态全局变量前写一行简短说明。
- 单个代码文件原则上不超过 500 行，按职责拆分。
- 关键行为必须有测试，集成测试统一放在 `tests/`。
- 测试函数名使用英文 `snake_case`；中文测试意图写在函数前注释中。

## Windows 路径兼容性

- Windows 上不得让普通驱动器路径或 UNC 路径的 `\\?\` / `\\?\UNC\` verbatim 前缀进入配置模型、持久化数据、协议、界面文本或外部命令参数。
- 源码不得绕过 `crate::platform` 直接调用 `std::fs::canonicalize`、`Path::canonicalize`、`std::env::current_dir`、`std::env::current_exe` 或 `std::env::temp_dir`；统一使用平台模块的对应入口。
- 从操作系统、用户输入、旧持久化数据或第三方组件接收绝对路径时，必须在边界调用 `simplify_path`；规范化已存在路径时调用 `platform::canonicalize`。
- 修改路径处理时必须覆盖驱动器路径、UNC 路径、非 ASCII 路径、重复清理和外部命令/持久化边界，并确认不会误改 `\\.\` 或无法安全降级的设备命名空间。
