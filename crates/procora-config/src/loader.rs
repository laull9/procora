use std::{fs, path::Path};

use procora_core::{ProjectSpec, TaskGraph};

use crate::{ConfigDiagnostic, ConfigError, ConfigFormat, ManagedDependencies, raw::RawProject};

/// 已通过结构、语义和任务图校验的项目配置。
#[derive(Debug)]
pub struct CompiledProject {
    /// 规范化项目配置。
    pub spec: ProjectSpec,
    /// 完成环检测的任务图。
    pub graph: TaskGraph,
    /// 已通过字段与来源校验的项目级管理依赖。
    pub dependencies: ManagedDependencies,
}

/// 从指定路径读取并编译项目配置。
///
/// # Errors
///
/// 当格式未知、文件无法读取、内容无效或任务图无法编译时返回错误。
pub fn load_path(path: impl AsRef<Path>) -> Result<CompiledProject, ConfigError> {
    let path = path.as_ref();
    let format = ConfigFormat::from_path(path)
        .ok_or_else(|| ConfigError::UnknownFormat(path.to_path_buf()))?;
    let input = fs::read_to_string(path).map_err(|source| ConfigError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let base = path
        .parent()
        .map(|parent| fs::canonicalize(parent).unwrap_or_else(|_| parent.to_path_buf()));
    compile(&input, format, base.as_deref())
}

/// 从内存文本解析并编译项目配置。
///
/// # Errors
///
/// 当内容无法解析、语义无效或任务图无法编译时返回错误。
pub fn load_str(input: &str, format: ConfigFormat) -> Result<CompiledProject, ConfigError> {
    compile(input, format, None)
}

/// 执行格式解析、原始 DTO 规范化和任务图编译。
fn compile(
    input: &str,
    format: ConfigFormat,
    base_directory: Option<&Path>,
) -> Result<CompiledProject, ConfigError> {
    let raw = parse_raw(input, format)?;
    let (spec, dependencies) = raw.normalize(base_directory).map_err(validation_error)?;
    let graph = TaskGraph::compile(&spec)?;
    Ok(CompiledProject {
        spec,
        graph,
        dependencies,
    })
}

/// 按输入格式反序列化原始配置并保留字段路径。
fn parse_raw(input: &str, format: ConfigFormat) -> Result<RawProject, ConfigError> {
    match format {
        ConfigFormat::Yaml => parse_yaml(input),
        ConfigFormat::Toml => parse_toml(input),
        ConfigFormat::Json => parse_json(input),
    }
}

/// 使用 serde-saphyr 的流式入口捕获 YAML 字段路径与位置。
fn parse_yaml(input: &str) -> Result<RawProject, ConfigError> {
    let mut field_path = None;
    serde_saphyr::with_deserializer_from_str(input, |deserializer| {
        serde_path_to_error::deserialize(deserializer).map_err(|error| {
            field_path = Some(error.path().to_string());
            error.into_inner()
        })
    })
    .map_err(|error| ConfigError::Parse {
        format: ConfigFormat::Yaml,
        message: parse_message(field_path.as_deref(), &error.to_string()),
    })
}

/// 捕获 TOML 字段路径以及解析器提供的源位置。
fn parse_toml(input: &str) -> Result<RawProject, ConfigError> {
    let deserializer =
        toml::Deserializer::parse(input).map_err(|error| toml_parse_error(input, None, &error))?;
    serde_path_to_error::deserialize(deserializer).map_err(|error| {
        let path = error.path().to_string();
        toml_parse_error(input, Some(&path), &error.into_inner())
    })
}

/// 把 TOML 错误的 span 换算为字段路径和行列号。
fn toml_parse_error(input: &str, path: Option<&str>, error: &toml::de::Error) -> ConfigError {
    let location = error
        .span()
        .map(|span| line_column(input, span.start))
        .map_or_else(String::new, |(line, column)| {
            format!("第 {line} 行第 {column} 列：")
        });
    ConfigError::Parse {
        format: ConfigFormat::Toml,
        message: format!("{location}{}", parse_message(path, &error.to_string())),
    }
}

/// 捕获 JSON 字段路径、行号和列号。
fn parse_json(input: &str) -> Result<RawProject, ConfigError> {
    let mut deserializer = serde_json::Deserializer::from_str(input);
    serde_path_to_error::deserialize(&mut deserializer).map_err(|error| {
        let path = error.path().to_string();
        let inner = error.into_inner();
        ConfigError::Parse {
            format: ConfigFormat::Json,
            message: format!(
                "第 {} 行第 {} 列：{}",
                inner.line(),
                inner.column(),
                parse_message(Some(&path), &inner.to_string())
            ),
        }
    })
}

/// 组合可选字段路径和底层解析器消息。
fn parse_message(path: Option<&str>, message: &str) -> String {
    path.filter(|path| !path.is_empty() && *path != ".")
        .map_or_else(
            || message.to_owned(),
            |path| format!("字段 `{path}`：{message}"),
        )
}

/// 把字节偏移换算为从一开始的行列号。
fn line_column(input: &str, offset: usize) -> (usize, usize) {
    let prefix = &input[..offset.min(input.len())];
    let line = prefix.bytes().filter(|byte| *byte == b'\n').count() + 1;
    let column = prefix
        .rsplit_once('\n')
        .map_or(prefix.chars().count() + 1, |(_, tail)| {
            tail.chars().count() + 1
        });
    (line, column)
}

/// 把多条结构化诊断转换为单个配置错误。
fn validation_error(diagnostics: Vec<ConfigDiagnostic>) -> ConfigError {
    let details = diagnostics
        .iter()
        .map(|diagnostic| format!("{}: {}", diagnostic.path, diagnostic.message))
        .collect::<Vec<_>>()
        .join("; ");
    ConfigError::Validation {
        details,
        diagnostics,
    }
}
