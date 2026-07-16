use std::path::PathBuf;

use crate::core::{GraphError, ServiceNameError};
use thiserror::Error;

use super::ConfigFormat;

/// 单条可操作的字段级配置诊断。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfigDiagnostic {
    /// 使用点分隔的配置字段路径。
    pub path: String,
    /// 可读的失败原因。
    pub message: String,
}

/// 配置读取和编译错误。
#[derive(Debug, Error)]
pub enum ConfigError {
    /// 配置扩展名不受支持。
    #[error("无法从路径 `{0}` 判断配置格式")]
    UnknownFormat(PathBuf),
    /// 配置文件读取失败。
    #[error("读取配置 `{path}` 失败: {source}")]
    Read {
        /// 配置路径。
        path: PathBuf,
        /// 底层文件错误。
        source: std::io::Error,
    },
    /// include 路径违反根目录、相对路径或资源限制。
    #[error("include 配置无效：{0}")]
    Include(String),
    /// include 文件自身无法解析或校验。
    #[error("include 文件 `{path}` 无效: {source}")]
    IncludedFile {
        /// 出错的 include 文件。
        path: PathBuf,
        /// 该文件的底层配置错误。
        source: Box<ConfigError>,
    },
    /// 受控 Python 配置辅助进程失败。
    #[error("Python 配置 `{path}` 失败: {message}")]
    Python {
        /// 用户显式指定的 Python 配置入口。
        path: PathBuf,
        /// 解释器、超时、输出或 JSON 诊断。
        message: String,
    },
    /// 配置文本解析失败。
    #[error("{format} 配置解析失败: {message}")]
    Parse {
        /// 当前解析格式。
        format: ConfigFormat,
        /// 格式解析器返回的诊断。
        message: String,
    },
    /// 格式解析成功，但存在一个或多个独立语义错误。
    #[error("配置校验失败: {details}")]
    Validation {
        /// 拼接后用于 CLI 展示的完整诊断。
        details: String,
        /// 结构化字段诊断。
        diagnostics: Vec<ConfigDiagnostic>,
    },
    /// 配置模式版本暂不支持。
    #[error("不支持配置版本 {0}，当前只支持版本 1")]
    UnsupportedVersion(u32),
    /// 项目标识为空。
    #[error("项目标识不能为空")]
    EmptyProject,
    /// 项目标识不能作为本机服务名称。
    #[error("项目标识不能作为服务名称: {0}")]
    InvalidServiceName(#[from] ServiceNameError),
    /// 任务命令为空。
    #[error("任务 `{0}` 的 command 不能为空")]
    EmptyCommand(String),
    /// 任务图编译失败。
    #[error(transparent)]
    Graph(#[from] GraphError),
}
