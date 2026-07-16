# 项目约束

## 分支与发布

- `dev` 是日常开发与集成分支，`main` 只保存可发布版本；禁止直接在二者上提交或强推。
- 功能、修复从 `dev` 建短期分支，经 PR 和 CI 合入 `dev`。
- 发布时由 `dev` 向 `main` 提 PR 并使用 merge commit；合入后仅在 `main` 创建与 `Cargo.toml` 版本一致的 `vX.Y.Z` 标签。
- 紧急修复从 `main` 建分支，发布后将 `main` 同步回 `dev`，避免修复丢失。
- 合入前必须通过 `cargo fmt --all -- --check`、`cargo clippy --locked --all-targets --all-features -- -D warnings` 和 `cargo test --locked --all-features`。

## 代码规范

- 注释和文档使用中文；关键 trait、结构体、函数及静态全局变量前写一行简短说明。
- 单个代码文件原则上不超过 500 行，按职责拆分。
- 关键行为必须有测试，集成测试统一放在 `tests/`。
- 测试函数名使用英文 `snake_case`；中文测试意图写在函数前注释中。
