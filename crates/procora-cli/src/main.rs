//! Procora 命令行二进制入口。

/// 启动 Procora 命令行入口。
fn main() -> anyhow::Result<()> {
    procora_cli::run()
}
