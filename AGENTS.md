# 项目约束

## 分支与发布

- `dev` 是唯一的日常开发与集成分支；本地功能和修复直接在 `dev` 开发、提交并推送，不创建短期开发分支，也不从其他分支向 `dev` 合并。
- `dev` 禁止强推；`main` 只保存可发布版本，禁止直接提交或强推。
- 发布时由 `dev` 向 `main` 提 PR 并使用 merge commit；合入后仅在 `main` 创建与 `Cargo.toml` 版本一致的 `vX.Y.Z` 标签。
- 紧急修复同样直接在 `dev` 完成并按正常发布流程进入 `main`，不从 `main` 或其他分支回合到 `dev`。
- 推送前必须通过 `cargo fmt --all -- --check`、`cargo clippy --locked --all-targets --all-features -- -D warnings` 和 `cargo test --locked --all-features`。

## 代码规范

- 注释和文档使用中文；关键 trait、结构体、函数及静态全局变量前写一行简短说明。
- 单个代码文件原则上不超过 500 行，按职责拆分。
- 关键行为必须有测试，集成测试统一放在 `tests/`。
- 测试函数名使用英文 `snake_case`；中文测试意图写在函数前注释中。
