use std::{collections::BTreeMap, path::PathBuf};

use serde::Serialize;

/// 依赖下载后的内容类型。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DependencyKind {
    /// 根据解包结果自动判断文件、目录或二进制。
    #[default]
    Auto,
    /// 单个需要可执行权限的二进制文件。
    Binary,
    /// 单个普通文件。
    File,
    /// 一个完整目录。
    Directory,
}

/// 下载内容的解包策略。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UnpackMode {
    /// 根据文件名与文件头识别常见归档。
    #[default]
    Auto,
    /// 保留下载的原始文件。
    Never,
}

/// 管理依赖的版本验证命令。
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct DependencyVerifySpec {
    /// 相对安装根目录的验证程序；省略时使用最终管理路径。
    pub command: Option<PathBuf>,
    /// 不经过 shell 解释的命令参数。
    pub args: Vec<String>,
    /// 输出必须包含的文本；省略时使用声明版本。
    pub contains: Option<String>,
}

/// 远端依赖的下载可靠性与资源边界。
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DependencyDownloadSpec {
    /// 单个来源在首次失败后的重试次数。
    pub retries: u8,
    /// 单次传输允许占用的最长时间，单位毫秒。
    #[serde(
        rename = "timeout",
        serialize_with = "crate::config::serialize_duration"
    )]
    pub timeout_ms: u64,
    /// 下载内容允许的最大字节数。
    pub max_bytes: u64,
    /// HTTP 请求头；值可通过 `${env.NAME}` 延迟读取进程环境。
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
}

impl Default for DependencyDownloadSpec {
    fn default() -> Self {
        Self {
            retries: 2,
            timeout_ms: 120_000,
            max_bytes: 2 * 1024 * 1024 * 1024,
            headers: BTreeMap::new(),
        }
    }
}

/// SSH 下载可选的显式认证与主机密钥文件。
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct DependencySshSpec {
    /// OpenSSH 私钥路径；省略时使用 SSH 配置和 agent。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identity_file: Option<PathBuf>,
    /// OpenSSH `known_hosts` 路径；省略时使用用户默认文件。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub known_hosts_file: Option<PathBuf>,
}

/// 单个可下载、解包和验证的项目依赖。
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ManagedDependencySpec {
    /// HTTP(S)、SSH、SCP 或本地文件来源。
    pub source: String,
    /// 主来源不可用时按顺序尝试的镜像。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub mirrors: Vec<String>,
    /// 用于安装目录和版本清单的固定版本。
    pub version: String,
    /// 可选的 `sha256:<hex>` 内容校验值。
    pub checksum: Option<String>,
    /// 下载后的解包策略。
    pub unpack: UnpackMode,
    /// 归档内要管理的相对路径。
    pub path: Option<PathBuf>,
    /// 最终内容类型。
    pub kind: DependencyKind,
    /// 可选的真实命令版本验证。
    pub verify: Option<DependencyVerifySpec>,
    /// 下载重试、超时、大小和 HTTP 请求头策略。
    pub download: DependencyDownloadSpec,
    /// SSH 来源的可选显式认证参数。
    #[serde(skip_serializing_if = "DependencySshSpec::is_empty")]
    pub ssh: DependencySshSpec,
}

impl DependencySshSpec {
    /// 返回是否完全依赖用户 SSH 默认配置。
    pub fn is_empty(&self) -> bool {
        self.identity_file.is_none() && self.known_hosts_file.is_none()
    }
}

/// 项目级依赖集合。
pub type ManagedDependencies = BTreeMap<String, ManagedDependencySpec>;
