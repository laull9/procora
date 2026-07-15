use std::path::PathBuf;

use procora_core::{GraphError, ServiceNameError};
use thiserror::Error;

use crate::ConfigFormat;

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
