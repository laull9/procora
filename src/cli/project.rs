use std::{env, fs, io::IsTerminal, path::Path};

use crate::config::{DiscoveredProject, discover_path, is_python_config};
use crate::daemon::ServiceHost;
use crate::source::DependencyManager;
use anyhow::{Context, bail};
use serde::Serialize;

/// 面向诊断输出的完整有效配置视图。
#[derive(Serialize)]
struct EffectiveConfig<'a> {
    /// 配置模式主版本。
    version: u32,
    /// 服务稳定名称。
    project: &'a str,
    /// 当前持久选择的运行 profile。
    active_profile: Option<&'a str>,
    /// 配置中可选择的全部 profile。
    profiles: &'a std::collections::BTreeSet<String>,
    /// 已规范化的项目管理依赖。
    dependencies: &'a crate::config::ManagedDependencies,
    /// 合并到各 Task 前的项目级默认环境。
    env: &'a std::collections::BTreeMap<String, String>,
    /// 项目显式声明的 Task 默认层。
    task_defaults: &'a crate::config::TaskDefaultsSpec,
    /// 可供 Task 显式引用的命名模板。
    task_templates: &'a std::collections::BTreeSet<String>,
    /// 已应用默认值和路径规范化的 Task。
    tasks: &'a std::collections::BTreeMap<crate::core::TaskId, crate::core::TaskSpec>,
    /// 每个 Task 字段及最终环境变量的生效来源。
    origins: &'a std::collections::BTreeMap<crate::core::TaskId, crate::config::TaskConfigOrigins>,
}

/// 打开指定目录或文件的配置编辑页面。
pub(crate) fn edit(path: Option<&Path>) -> anyhow::Result<()> {
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        bail!("`procora edit` 需要交互式终端")
    }
    let target = path.map_or_else(
        || env::current_dir().context("无法读取当前目录"),
        |path| Ok(path.to_path_buf()),
    )?;
    if is_python_config(&target) {
        bail!("内置配置编辑器不执行或改写 procora.py；请使用可信的外部代码编辑器")
    }
    let discovered = discover_path(&target)
        .with_context(|| format!("无法定位要编辑的配置：{}", target.display()))?;
    crate::tui::edit_config(&discovered.config_path).context("配置编辑器退出失败")
}

/// 在交互式终端中自动打开刚创建的配置。
pub(crate) fn edit_after_init(path: &Path, no_edit: bool) -> anyhow::Result<()> {
    if no_edit || !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        println!("下一步：procora edit {}", path.display());
        return Ok(());
    }
    crate::tui::edit_config(path).context("配置编辑器退出失败")
}

/// 清空指定服务目录中的 `.procora` 运行时数据。
///
/// # Errors
///
/// 当目标路径不可访问、不是文件或目录，或无法删除运行时目录时返回错误。
pub(crate) fn clean(path: Option<&Path>) -> anyhow::Result<()> {
    let target = path.map_or_else(
        || env::current_dir().context("无法读取当前目录"),
        |path| Ok(path.to_path_buf()),
    )?;
    let target = fs::canonicalize(&target)
        .with_context(|| format!("无法访问要清理的服务路径：{}", target.display()))?;
    let root = if target.is_file() {
        target
            .parent()
            .context("配置文件路径没有父目录")?
            .to_path_buf()
    } else if target.is_dir() {
        target
    } else {
        bail!("服务路径 `{}` 既不是文件也不是目录", target.display());
    };
    let runtime = root.join(".procora");
    if !runtime.exists() {
        println!("没有需要清理的运行时目录：{}", runtime.display());
        return Ok(());
    }
    if !runtime.is_dir() {
        bail!("运行时路径 `{}` 不是目录，拒绝删除", runtime.display());
    }
    fs::remove_dir_all(&runtime).with_context(|| format!("无法清理 `{}`", runtime.display()))?;
    println!("已清理运行时目录：{}", runtime.display());
    Ok(())
}

/// 完整发现并校验配置，但不下载、注册或启动服务。
pub(crate) fn validate(path: &Path) -> anyhow::Result<()> {
    warn_python_execution(path);
    let discovered =
        discover_path(path).with_context(|| format!("配置校验失败: {}", path.display()))?;
    println!(
        "配置有效：服务 `{}`，配置 `{}`，profile `{}`，共 {} 个任务、{} 个模板、{} 个管理依赖",
        discovered.compiled.spec.project,
        discovered.config_path.display(),
        discovered
            .compiled
            .active_profile
            .as_deref()
            .unwrap_or("基础配置"),
        discovered.compiled.spec.tasks.len(),
        discovered.compiled.task_template_names.len(),
        discovered.compiled.dependencies.len()
    );
    Ok(())
}

/// 输出指定配置中的确定性任务启动顺序。
pub(crate) fn graph(path: &Path) -> anyhow::Result<()> {
    warn_python_execution(path);
    let discovered =
        discover_path(path).with_context(|| format!("任务图编译失败: {}", path.display()))?;
    let host = ServiceHost::from_compiled(discovered.compiled);
    for (index, task) in host.start_plan().iter().enumerate() {
        println!("{}. {task}", index + 1);
    }
    Ok(())
}

/// 以确定性 JSON 输出默认值与路径规范化后的有效配置。
pub(crate) fn effective_config(path: &Path) -> anyhow::Result<()> {
    warn_python_execution(path);
    let discovered =
        discover_path(path).with_context(|| format!("有效配置生成失败: {}", path.display()))?;
    let output = EffectiveConfig {
        version: discovered.compiled.spec.version,
        project: &discovered.compiled.spec.project,
        active_profile: discovered.compiled.active_profile.as_deref(),
        profiles: &discovered.compiled.profile_names,
        dependencies: &discovered.compiled.dependencies,
        env: &discovered.compiled.project_env,
        task_defaults: &discovered.compiled.task_defaults,
        task_templates: &discovered.compiled.task_template_names,
        tasks: &discovered.compiled.spec.tasks,
        origins: &discovered.compiled.task_origins,
    };
    println!(
        "{}",
        serde_json::to_string_pretty(&output).context("有效配置 JSON 序列化失败")?
    );
    Ok(())
}

/// 同步或仅检查项目声明的管理依赖。
pub(crate) fn dependencies(path: &Path, check: bool) -> anyhow::Result<()> {
    warn_python_execution(path);
    let discovered =
        discover_path(path).with_context(|| format!("依赖配置校验失败: {}", path.display()))?;
    let manager = DependencyManager::new(&discovered.root);
    let resolved = if check {
        manager.check(&discovered.compiled.dependencies)
    } else {
        manager.sync(&discovered.compiled.dependencies)
    }
    .context("项目依赖处理失败")?;
    if resolved.is_empty() {
        println!("没有声明管理依赖");
    }
    for dependency in resolved {
        println!(
            "{} {} {} {}",
            if dependency.installed {
                "已安装"
            } else {
                "已验证"
            },
            dependency.name,
            dependency.version,
            dependency.path.display()
        );
    }
    Ok(())
}

/// 为任务启动同步依赖并应用路径占位符。
pub(crate) fn prepare(discovered: &mut DiscoveredProject) -> anyhow::Result<()> {
    DependencyManager::new(&discovered.root)
        .prepare(&mut discovered.compiled)
        .context("项目依赖准备失败")?;
    Ok(())
}

/// 对精确 Python 入口给出用户可见的可信代码执行提示。
pub(crate) fn warn_python_execution(path: &Path) {
    if is_python_config(path) {
        eprintln!(
            "警告：procora.py 将以当前用户权限执行可信代码；受控辅助进程提供资源边界，但不是安全沙箱。"
        );
    }
}
