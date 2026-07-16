use std::{collections::BTreeMap, path::PathBuf};

/// 依赖下载后的内容类型。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
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
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum UnpackMode {
    /// 根据文件名与文件头识别常见归档。
    #[default]
    Auto,
    /// 保留下载的原始文件。
    Never,
}

/// 管理依赖的版本验证命令。
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DependencyVerifySpec {
    /// 相对安装根目录的验证程序；省略时使用最终管理路径。
    pub command: Option<PathBuf>,
    /// 不经过 shell 解释的命令参数。
    pub args: Vec<String>,
    /// 输出必须包含的文本；省略时使用声明版本。
    pub contains: Option<String>,
}

/// 单个可下载、解包和验证的项目依赖。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedDependencySpec {
    /// HTTP(S)、SSH、SCP 或本地文件来源。
    pub source: String,
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
}

/// 项目级依赖集合。
pub type ManagedDependencies = BTreeMap<String, ManagedDependencySpec>;
