//! 服务总览中的 Procora 包工作台会话与操作编排。

use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, bail};

use crate::{
    package::{self, PackagePlatform},
    tui::{
        PackageWorkspaceApp, PackageWorkspaceEntry, PackageWorkspaceExit, SelectionItem,
        select_inline, select_path_inline_named,
    },
};

/// 从服务总览进入并持续运行包工作台，直到用户返回。
pub(super) fn run(context_source: Option<&Path>, control_allowed: bool) -> anyhow::Result<()> {
    let mut opened = BTreeSet::new();
    let (packages, installed) = load_workspace(context_source, &opened)?;
    let mut app =
        PackageWorkspaceApp::new(packages, installed, context_source.map(Path::to_path_buf));
    loop {
        let exit = crate::tui::run_package_workspace(&mut app, control_allowed)?;
        let result = execute(exit, context_source, &mut opened);
        match result {
            Ok(WorkspaceFlow::Back) => return Ok(()),
            Ok(flow) => {
                let (feedback, selected_package) = match flow {
                    WorkspaceFlow::Back => unreachable!("返回流程已经提前处理"),
                    WorkspaceFlow::Continue(feedback) => (feedback, None),
                    WorkspaceFlow::ContinueSelecting { feedback, path } => (feedback, Some(path)),
                };
                match load_workspace(context_source, &opened) {
                    Ok((packages, installed)) => {
                        app.replace_data(packages, installed);
                        if let Some(path) = selected_package {
                            app.select_package_path(&path);
                        }
                    }
                    Err(error) => app.set_feedback(format!("刷新失败：{error:#}")),
                }
                app.set_feedback(feedback);
            }
            Err(error) => app.set_feedback(format!("操作失败：{error:#}")),
        }
    }
}

/// 一次工作台操作结束后的导航结果。
enum WorkspaceFlow {
    Back,
    Continue(String),
    ContinueSelecting { feedback: String, path: PathBuf },
}

/// 执行离开全屏 TUI 后的包操作。
fn execute(
    exit: PackageWorkspaceExit,
    context_source: Option<&Path>,
    opened: &mut BTreeSet<PathBuf>,
) -> anyhow::Result<WorkspaceFlow> {
    match exit {
        PackageWorkspaceExit::Back => Ok(WorkspaceFlow::Back),
        PackageWorkspaceExit::Refresh => {
            Ok(WorkspaceFlow::Continue("已重新扫描包与安装状态".to_owned()))
        }
        PackageWorkspaceExit::OpenPackage => open_package(opened),
        PackageWorkspaceExit::BuildPackage => build_package(context_source, opened),
        PackageWorkspaceExit::BuildAndDeploy => build_and_deploy(context_source, opened),
        PackageWorkspaceExit::Verify(path) => {
            let info = package::verify(&path)?;
            Ok(WorkspaceFlow::Continue(format!(
                "验证通过：{} · {}",
                info.manifest.project, info.package_digest
            )))
        }
        PackageWorkspaceExit::Install(path) => {
            super::super::package_command::install_path(&path, 30_000, 2_000, 3)?;
            Ok(WorkspaceFlow::Continue(
                "安装完成，活动 release 已通过验活".to_owned(),
            ))
        }
        PackageWorkspaceExit::Run(path) => {
            super::super::runtime::run_package_temporary(&path)?;
            Ok(WorkspaceFlow::Continue(
                "临时 Service 已正常退出".to_owned(),
            ))
        }
        PackageWorkspaceExit::Extract(path) => extract_package(&path),
        PackageWorkspaceExit::Deploy(path) => {
            let memory_root = context_memory_root(context_source, &path);
            let feedback = deploy_package_reviewed(&path, memory_root.as_deref(), None)?;
            Ok(WorkspaceFlow::Continue(feedback))
        }
        PackageWorkspaceExit::PushExport { package, entry } => {
            super::super::push::run(super::super::push::PushRequest {
                source: Some(package),
                target: None,
                package_entry: Some(&entry),
                package_platform: "current",
                ssh: None,
                remote_bin: None,
                batch: false,
                restart: false,
            })?;
            Ok(WorkspaceFlow::Continue(format!("导出项 `{entry}` 已推送")))
        }
        PackageWorkspaceExit::DeletePackage(path) => delete_package(&path, opened),
        PackageWorkspaceExit::Rollback(project) => {
            super::super::package_installed_command::rollback(&project, None, 30_000, 2_000)?;
            Ok(WorkspaceFlow::Continue(format!(
                "`{project}` 已回滚并通过验活"
            )))
        }
        PackageWorkspaceExit::Recover(project) => {
            super::super::package_installed_command::recover(&project, 30_000, 2_000)?;
            Ok(WorkspaceFlow::Continue(format!(
                "`{project}` 的中断状态已检查并恢复"
            )))
        }
        PackageWorkspaceExit::Purge(project) => {
            let feedback = super::super::package_installed_command::uninstall(&project, true)?;
            Ok(WorkspaceFlow::Continue(feedback))
        }
        PackageWorkspaceExit::Uninstall(project) => {
            let feedback = super::super::package_installed_command::uninstall(&project, false)?;
            Ok(WorkspaceFlow::Continue(feedback))
        }
    }
}

/// 永久删除选中的包文件，并从本次会话的显式打开集合移除。
fn delete_package(path: &Path, opened: &mut BTreeSet<PathBuf>) -> anyhow::Result<WorkspaceFlow> {
    package::delete_file(path)?;
    opened.remove(path);
    Ok(WorkspaceFlow::Continue(format!(
        "包文件已永久删除：{}",
        path.display()
    )))
}

/// 选择并加入一个已有 `.pcpkg`。
fn open_package(opened: &mut BTreeSet<PathBuf>) -> anyhow::Result<WorkspaceFlow> {
    let Some(path) = select_path_inline_named(
        None,
        "打开 Procora 包",
        "选择一个 `.pcpkg` 文件；Enter 进入目录或确认文件，Esc 取消。",
    )?
    else {
        return Ok(WorkspaceFlow::Continue("已取消打开包".to_owned()));
    };
    let path = crate::platform::canonicalize(&path)
        .with_context(|| format!("无法访问 `{}`", path.display()))?;
    if !package::is_package_path(&path) || !path.is_file() {
        bail!("请选择扩展名为 `.pcpkg` 的普通文件");
    }
    package::inspect(&path)?;
    opened.insert(path.clone());
    Ok(WorkspaceFlow::ContinueSelecting {
        feedback: format!("已加入包：{}", path.display()),
        path,
    })
}

/// 从上下文或用户选择的 Service 构建胖包或当前平台薄包。
fn build_package(
    context_source: Option<&Path>,
    opened: &mut BTreeSet<PathBuf>,
) -> anyhow::Result<WorkspaceFlow> {
    let built = build_package_artifact(context_source)?;
    opened.insert(built.result.path.clone());
    Ok(WorkspaceFlow::ContinueSelecting {
        feedback: format!(
            "构建完成：{} · {} 个文件 / {} 个二进制变体",
            built.result.path.display(),
            built.result.files,
            built.result.binary_variants
        ),
        path: built.result.path,
    })
}

/// 构建成功后预检并确认部署，失败或取消时仍保留并选中新包。
fn build_and_deploy(
    context_source: Option<&Path>,
    opened: &mut BTreeSet<PathBuf>,
) -> anyhow::Result<WorkspaceFlow> {
    let built = build_package_artifact(context_source)?;
    let path = built.result.path.clone();
    opened.insert(path.clone());
    let feedback = match deploy_package_reviewed(
        &path,
        Some(&built.service_root),
        Some(&built.result.project),
    ) {
        Ok(feedback) => format!("包已构建；{feedback}"),
        Err(error) => format!("包已构建并保留，但部署失败：{error:#}"),
    };
    Ok(WorkspaceFlow::ContinueSelecting { feedback, path })
}

/// 一次构建产生的包与稳定 Service 身份。
struct BuiltPackage {
    service_root: PathBuf,
    result: package::PackageBuildResult,
}

/// 选择来源与平台并构建经过自校验的包文件。
fn build_package_artifact(context_source: Option<&Path>) -> anyhow::Result<BuiltPackage> {
    let source = match context_source {
        Some(source) => source.to_path_buf(),
        None => select_path_inline_named(
            None,
            "选择待打包 Service",
            "选择 Service 目录或 Procora 配置文件；Space 选择当前目录。",
        )?
        .context("已取消构建来源选择")?,
    };
    let source = crate::platform::canonicalize(&source)
        .with_context(|| format!("无法访问构建来源 `{}`", source.display()))?;
    let discovered = crate::config::discover_path(&source)?;
    let platform = select_inline(
        "选择 Procora 包平台范围",
        "胖包适合分发到不同机器；薄包更小，只适合当前平台。",
        vec![
            SelectionItem::new(
                "多平台胖包（推荐）",
                "打入配置声明的全部平台二进制",
                PackagePlatform::All,
            ),
            SelectionItem::new(
                "当前平台薄包",
                "只打入当前操作系统与架构需要的二进制",
                PackagePlatform::Target(crate::config::DeployPlatform::current()),
            ),
        ],
    )?
    .context("已取消包平台选择")?;
    let output = next_output_path(
        &discovered.root,
        &format!("{}.pcpkg", discovered.compiled.spec.project),
    );
    eprintln!(
        "正在构建：{} → {}",
        discovered.root.display(),
        output.display()
    );
    let result = package::build(&source, &output, platform)?;
    Ok(BuiltPackage {
        service_root: discovered.root,
        result,
    })
}

/// 使用完整预检计划确认一次包部署，并生成返回工作台的结果摘要。
fn deploy_package_reviewed(
    path: &Path,
    memory_root: Option<&Path>,
    expected_service: Option<&str>,
) -> anyhow::Result<String> {
    let outcome = super::super::deploy::run_reviewed(
        &super::super::deploy::DeployArgs {
            source: path.to_path_buf(),
            ssh: None,
            remote_bin: None,
            service: expected_service.map(str::to_owned),
            timeout: 30_000,
            stable_for: 2_000,
            keep: 3,
            batch: false,
            dry_run: false,
        },
        memory_root,
    )?;
    Ok(match outcome {
        super::super::deploy::ReviewedDeploy::Cancelled { project } => {
            format!("`{project}` 部署已取消，远端未修改")
        }
        super::super::deploy::ReviewedDeploy::Completed {
            project,
            release,
            changed,
            ssh_target,
        } if changed => {
            format!("`{project}` 已部署到 {ssh_target}，release {release}")
        }
        super::super::deploy::ReviewedDeploy::Completed {
            project,
            release,
            ssh_target,
            ..
        } => format!("`{project}` 在 {ssh_target} 已运行 release {release}，无需更新"),
    })
}

/// 仅当包与当前上下文 Service 身份一致时复用稳定的部署目标记忆。
fn context_memory_root(context_source: Option<&Path>, package_path: &Path) -> Option<PathBuf> {
    let discovered = crate::config::discover_path(context_source?).ok()?;
    let info = package::inspect(package_path).ok()?;
    (discovered.compiled.spec.project == info.manifest.project).then_some(discovered.root)
}

/// 为解包生成不会覆盖既有内容的相邻目录。
fn extract_package(path: &Path) -> anyhow::Result<WorkspaceFlow> {
    let parent = path.parent().context("包路径没有父目录")?;
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("procora-package");
    let output = next_output_path(parent, &format!("{stem}-unpacked"));
    let result = package::extract(path, &output, crate::config::DeployPlatform::current())?;
    Ok(WorkspaceFlow::Continue(format!(
        "解包完成：{} · {} 个文件",
        output.display(),
        result.files
    )))
}

/// 扫描上下文、当前目录和托管原始包，保留损坏包诊断。
fn load_workspace(
    context_source: Option<&Path>,
    opened: &BTreeSet<PathBuf>,
) -> anyhow::Result<(Vec<PackageWorkspaceEntry>, Vec<package::InstalledService>)> {
    let catalog = package::installed_catalog(&super::super::api::managed_services_root()?)?;
    let mut paths = opened.clone();
    if let Some(source) = context_source {
        collect_direct_packages(source, &mut paths);
    }
    if let Ok(current) = crate::platform::current_dir() {
        collect_direct_packages(&current, &mut paths);
    }
    for service in &catalog.services {
        paths.extend(service.packages.iter().map(|stored| stored.path.clone()));
    }
    let packages = paths
        .into_iter()
        .filter(|path| path.is_file())
        .map(|path| match package::inspect(&path) {
            Ok(info) => PackageWorkspaceEntry {
                path,
                info: Some(info),
                error: None,
            },
            Err(error) => PackageWorkspaceEntry {
                path,
                info: None,
                error: Some(format!("{error:#}")),
            },
        })
        .collect();
    Ok((packages, catalog.services))
}

/// 收集一个目录第一层的包文件，避免 TUI 扫描大型目录树。
fn collect_direct_packages(source: &Path, paths: &mut BTreeSet<PathBuf>) {
    let directory = if source.is_dir() {
        source
    } else {
        source.parent().unwrap_or(source)
    };
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    paths.extend(
        entries
            .filter_map(Result::ok)
            .map(|entry| crate::platform::simplify_path(&entry.path()))
            .filter(|path| package::is_package_path(path)),
    );
}

/// 返回首个不存在的输出路径，避免隐式覆盖用户数据。
fn next_output_path(directory: &Path, name: &str) -> PathBuf {
    let initial = directory.join(name);
    if !initial.exists() {
        return initial;
    }
    let path = Path::new(name);
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or(name);
    let extension = path.extension().and_then(|value| value.to_str());
    for index in 2_u32.. {
        let candidate = extension.map_or_else(
            || directory.join(format!("{stem}-{index}")),
            |extension| directory.join(format!("{stem}-{index}.{extension}")),
        );
        if !candidate.exists() {
            return candidate;
        }
    }
    unreachable!("无限编号总能产生尚不存在的路径")
}
