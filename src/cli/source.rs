//! 外部任务定义来源的无副作用预览与重新确认命令。

use std::path::{Path, PathBuf};

use anyhow::{Context, bail};

use crate::source::{GitDefinitionCandidate, GitSource};

use super::{GitConfirmArgs, GitDefinitionArgs, GitSourceCommand, SourceCommand, center_runtime};

/// 分发外部定义来源命令。
pub(super) fn run(command: SourceCommand) -> anyhow::Result<()> {
    match command {
        SourceCommand::Git { command } => match command {
            GitSourceCommand::Preview(arguments) => preview(&arguments),
            GitSourceCommand::Confirm(arguments) => confirm(&arguments),
        },
    }
}

/// 获取并展示不会自动应用的 Git 候选。
fn preview(arguments: &GitDefinitionArgs) -> anyhow::Result<()> {
    let source = build_source(arguments)?;
    let candidate = source.fetch_candidate().context("Git 候选获取失败")?;
    print_candidate(&candidate);
    if candidate.compiled.is_err() {
        bail!("Git 候选配置无效，未注册或启动任何 Task")
    }
    println!("预览完成：未注册服务，也未启动 Task。");
    println!("下一步：使用 source git confirm 并传入上述修订重新确认；该命令仍不会启动 Task。");
    Ok(())
}

/// 重新获取来源并拒绝已经变化的候选修订。
fn confirm(arguments: &GitConfirmArgs) -> anyhow::Result<()> {
    let source = build_source(&arguments.definition)?;
    let candidate = source
        .confirm_candidate(&arguments.revision)
        .context("Git 候选重新确认失败")?;
    print_candidate(&candidate);
    if candidate.compiled.is_err() {
        bail!("Git 候选配置无效，未注册或启动任何 Task")
    }
    println!("确认完成：来源和配置修订未变化；未注册服务，也未启动 Task。");
    Ok(())
}

/// 根据远端/本地选择和默认用户缓存目录构造来源。
fn build_source(arguments: &GitDefinitionArgs) -> anyhow::Result<GitSource> {
    let cache = arguments
        .cache
        .clone()
        .map_or_else(default_cache_root, Ok)?;
    if arguments.local {
        GitSource::local(
            Path::new(&arguments.repository),
            &arguments.reference,
            &arguments.config,
            cache,
        )
        .context("本地 Git 来源参数无效")
    } else {
        GitSource::remote(
            &arguments.repository,
            &arguments.reference,
            &arguments.config,
            cache,
        )
        .context("远端 Git 来源参数无效")
    }
}

/// 返回当前用户 Procora 数据目录内的 Git checkout 缓存。
fn default_cache_root() -> anyhow::Result<PathBuf> {
    let paths = center_runtime::center_paths()?;
    let root = paths
        .database
        .parent()
        .context("Procora 状态数据库路径没有父目录")?;
    Ok(root.join("git-sources"))
}

/// 输出提交、修订、入口和编译状态。
fn print_candidate(candidate: &GitDefinitionCandidate) {
    println!("仓库：{}", candidate.repository);
    println!("引用：{}", candidate.reference);
    println!("提交：{}", candidate.commit);
    println!("修订：{}", candidate.revision);
    println!("配置：{}", candidate.config_path.display());
    match &candidate.compiled {
        Ok(compiled) => println!(
            "有效：是（服务 `{}`，{} 个 Task）",
            compiled.spec.project,
            compiled.spec.tasks.len()
        ),
        Err(error) => println!("有效：否（{error}）"),
    }
}
