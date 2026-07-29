//! 本机已安装 Procora 包、不可变 release 与恢复状态目录。

use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};

use anyhow::{Context, bail};
use serde::Deserialize;

const MAX_STATE_BYTES: u64 = 4 * 1024 * 1024;

/// 当前用户全部包安装状态的稳定只读视图。
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct InstalledCatalog {
    /// 按 Service 名称排序的包安装项。
    pub services: Vec<InstalledService>,
}

/// 一个由 package install 或裸机接收器管理的 Service。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InstalledService {
    /// 状态文件声明的稳定 Service 名称。
    pub project: String,
    /// `services/<project>` 托管根目录。
    pub root: PathBuf,
    /// 最近一次确认可用的 release。
    pub active_release: Option<String>,
    /// 已开始切换但尚未确认或回滚的 release。
    pub pending_release: Option<String>,
    /// 已登记的不可变 release。
    pub releases: Vec<InstalledRelease>,
    /// 本机内容寻址保存的原始 `.pcpkg`。
    pub packages: Vec<StoredPackage>,
    /// 状态无法完整读取时的可恢复诊断。
    pub error: Option<String>,
}

/// 状态清单中的一个不可变 release。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InstalledRelease {
    /// release 的短内容标识。
    pub id: String,
    /// release 的完整内容摘要。
    pub sha256: String,
    /// release 内配置入口。
    pub config_path: PathBuf,
    /// 物化该 release 的运行平台。
    pub target_platform: Option<crate::config::DeployPlatform>,
    /// Unix 纪元毫秒部署时间。
    pub deployed_at_ms: i64,
    /// 当前是否为最近确认可用版本。
    pub active: bool,
    /// 当前是否处于未确认切换状态。
    pub pending: bool,
}

/// 一个保存在托管目录中的原始包。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredPackage {
    /// 原始包文件路径。
    pub path: PathBuf,
    /// 可读取时的逻辑 package digest。
    pub package_digest: Option<String>,
    /// 外层包文件字节数。
    pub package_bytes: u64,
    /// 可读取清单中的 Service 名称。
    pub project: Option<String>,
    /// 包清单损坏或不兼容时的诊断。
    pub error: Option<String>,
}

/// 与部署状态 JSON 兼容的最小目录模型。
#[derive(Debug, Deserialize)]
struct CatalogState {
    project: String,
    active_release: Option<String>,
    #[serde(default)]
    pending_release: Option<String>,
    #[serde(default)]
    releases: Vec<CatalogRelease>,
}

/// 与 release 记录兼容的最小目录模型。
#[derive(Debug, Deserialize)]
struct CatalogRelease {
    id: String,
    sha256: String,
    #[serde(default)]
    config_path: PathBuf,
    #[serde(default)]
    target_platform: Option<crate::config::DeployPlatform>,
    deployed_at_ms: i64,
}

/// 扫描指定托管根目录，保留单个损坏 Service 的诊断而不中断其他项。
///
/// # Errors
///
/// 当托管根本身存在但无法读取时返回错误。
pub fn installed_catalog(managed_root: &Path) -> anyhow::Result<InstalledCatalog> {
    if !managed_root.exists() {
        return Ok(InstalledCatalog::default());
    }
    let managed_root = crate::platform::canonicalize(managed_root)
        .with_context(|| format!("无法访问包安装根目录：{}", managed_root.display()))?;
    let mut entries = fs::read_dir(&managed_root)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(fs::DirEntry::file_name);
    let mut services = Vec::new();
    for entry in entries {
        let path = crate::platform::simplify_path(&entry.path());
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            continue;
        }
        services.push(read_service(&path));
    }
    services.sort_by(|left, right| left.project.cmp(&right.project));
    Ok(InstalledCatalog { services })
}

/// 读取单个 Service 状态并把局部错误降级为可展示项。
fn read_service(root: &Path) -> InstalledService {
    let fallback_project = root
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("unknown")
        .to_owned();
    match read_state(&root.join("state.json"), &fallback_project) {
        Ok(state) => {
            let active = state.active_release.clone();
            let pending = state.pending_release.clone();
            let mut releases = state
                .releases
                .into_iter()
                .map(|release| InstalledRelease {
                    active: active.as_deref() == Some(&release.id),
                    pending: pending.as_deref() == Some(&release.id),
                    id: release.id,
                    sha256: release.sha256,
                    config_path: release.config_path,
                    target_platform: release.target_platform,
                    deployed_at_ms: release.deployed_at_ms,
                })
                .collect::<Vec<_>>();
            releases.sort_by_key(|release| std::cmp::Reverse(release.deployed_at_ms));
            InstalledService {
                project: state.project,
                root: root.to_path_buf(),
                active_release: active,
                pending_release: pending,
                releases,
                packages: read_packages(&root.join("packages")),
                error: None,
            }
        }
        Err(error) => InstalledService {
            project: fallback_project,
            root: root.to_path_buf(),
            active_release: None,
            pending_release: None,
            releases: Vec::new(),
            packages: read_packages(&root.join("packages")),
            error: Some(format!("{error:#}")),
        },
    }
}

/// 有界读取并校验部署状态的 Service 身份。
fn read_state(path: &Path, expected_project: &str) -> anyhow::Result<CatalogState> {
    let mut file =
        fs::File::open(path).with_context(|| format!("无法读取安装状态：{}", path.display()))?;
    let size = file.metadata()?.len();
    if size == 0 || size > MAX_STATE_BYTES {
        bail!("安装状态大小必须在 1..={MAX_STATE_BYTES} 字节内");
    }
    let mut bytes = Vec::with_capacity(usize::try_from(size)?);
    Read::by_ref(&mut file)
        .take(MAX_STATE_BYTES + 1)
        .read_to_end(&mut bytes)?;
    let state: CatalogState = serde_json::from_slice(&bytes).context("安装状态不是有效 JSON")?;
    let _: crate::core::ServiceName = state.project.parse()?;
    if state.project != expected_project {
        bail!(
            "安装状态声明 Service `{}`，但托管目录名称是 `{expected_project}`",
            state.project
        );
    }
    Ok(state)
}

/// 稳定读取全部已保存 `.pcpkg` 的轻量清单状态。
fn read_packages(directory: &Path) -> Vec<StoredPackage> {
    let Ok(entries) = fs::read_dir(directory) else {
        return Vec::new();
    };
    let mut paths = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| super::is_package_path(path))
        .collect::<Vec<_>>();
    paths.sort();
    paths
        .into_iter()
        .map(|path| {
            let package_bytes = fs::metadata(&path).map_or(0, |metadata| metadata.len());
            match super::inspect(&path) {
                Ok(info) => StoredPackage {
                    path: crate::platform::simplify_path(&path),
                    package_digest: Some(info.package_digest),
                    package_bytes,
                    project: Some(info.manifest.project),
                    error: None,
                },
                Err(error) => StoredPackage {
                    path: crate::platform::simplify_path(&path),
                    package_digest: None,
                    package_bytes,
                    project: None,
                    error: Some(format!("{error:#}")),
                },
            }
        })
        .collect()
}
