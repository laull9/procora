use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use crate::core::{ProjectSpec, TaskGraph};

use super::{
    ConfigDiagnostic, ConfigError, ConfigFormat, ManagedDependencies, TaskConfigOrigins,
    TaskDefaultsSpec, raw::RawProject,
};

mod include;

/// 单次闭包加载捕获的一个配置文件输入。
#[derive(Debug)]
pub(crate) struct CapturedConfigInput {
    pub(crate) path: PathBuf,
    pub(crate) bytes: Vec<u8>,
}

/// 无论成功失败都保留已读取输入和需要监听路径的加载结果。
#[derive(Debug)]
pub(crate) struct ConfigLoadCapture {
    pub(crate) result: Result<CompiledProject, ConfigError>,
    pub(crate) inputs: Vec<CapturedConfigInput>,
    pub(crate) watched_paths: Vec<PathBuf>,
    pub(crate) root: PathBuf,
    /// 入口及 include 文档数量，不包含环境文件等声明输入。
    pub(crate) definition_documents: usize,
}

/// Python 生成 JSON 编译时额外捕获的显式环境文件。
pub(super) struct GeneratedJsonCapture {
    pub(super) result: Result<CompiledProject, ConfigError>,
    pub(super) inputs: Vec<CapturedConfigInput>,
    pub(super) watched_paths: Vec<PathBuf>,
}

/// 已通过结构、语义和任务图校验的项目配置。
#[derive(Debug)]
pub struct CompiledProject {
    /// 用户声明的项目变量表达式。
    pub vars: BTreeMap<String, String>,
    /// 完成链式引用解析后的项目变量值。
    pub resolved_vars: BTreeMap<String, String>,
    /// 配置字段路径到直接变量引用集合的映射。
    pub variable_references: BTreeMap<String, BTreeSet<String>>,
    /// 规范化项目配置。
    pub spec: ProjectSpec,
    /// 完成环检测的任务图。
    pub graph: TaskGraph,
    /// 已通过字段与来源校验的项目级管理依赖。
    pub dependencies: ManagedDependencies,
    /// 已合并到每个 Task 的项目级默认环境变量。
    pub project_env: BTreeMap<String, String>,
    /// 不包含 profile 覆盖的项目级环境声明。
    pub declared_project_env: BTreeMap<String, String>,
    /// 项目显式声明的 Task 默认层。
    pub task_defaults: TaskDefaultsSpec,
    /// 不包含 profile 覆盖的 Task 默认声明。
    pub declared_task_defaults: TaskDefaultsSpec,
    /// 当前配置持久选择的 profile。
    pub active_profile: Option<String>,
    /// 配置中声明的全部 profile 名称。
    pub profile_names: BTreeSet<String>,
    /// 每个 profile 的直接继承目标。
    pub profile_extends: BTreeMap<String, String>,
    /// profile 原始声明，供结构化编辑器无展开写回。
    pub(crate) profiles: BTreeMap<String, super::raw::RawProfile>,
    /// 已声明的命名 Task 模板名称。
    pub task_template_names: BTreeSet<String>,
    /// 命名 Task 模板的本地声明，供结构化编辑器无展开写回。
    pub(crate) task_templates: BTreeMap<String, super::raw::RawTask>,
    /// 每个 Task 显式引用的模板名称。
    pub(crate) task_extends: BTreeMap<crate::core::TaskId, String>,
    /// Task 显式环境文件路径，供编辑器保留声明而非展开写回。
    pub task_env_files: BTreeMap<crate::core::TaskId, PathBuf>,
    /// Task 显式内联环境，不包含项目默认值或环境文件内容。
    pub task_inline_env: BTreeMap<crate::core::TaskId, BTreeMap<String, String>>,
    /// 每个有效 Task 字段和环境变量的生效来源。
    pub task_origins: BTreeMap<crate::core::TaskId, TaskConfigOrigins>,
    /// 活动 Task 的原始本地声明，供编辑器保留变量表达式。
    pub(crate) task_declarations: BTreeMap<crate::core::TaskId, super::raw::RawTask>,
    /// 当前 profile 未准入但仍需编辑器保留的 Task 声明。
    pub(crate) inactive_tasks: BTreeMap<String, super::raw::RawTask>,
}

/// 从指定路径读取并编译项目配置。
///
/// # Errors
///
/// 当格式未知、文件无法读取、内容无效或任务图无法编译时返回错误。
pub fn load_path(path: impl AsRef<Path>) -> Result<CompiledProject, ConfigError> {
    load_path_capture(path.as_ref()).result
}

/// 加载配置闭包并保留修订计算和动态监听需要的输入元数据。
pub(crate) fn load_path_capture(path: &Path) -> ConfigLoadCapture {
    if super::python::is_python_config(path) {
        super::python::load_capture(path)
    } else {
        include::load(path)
    }
}

/// 按目标文件路径校验尚未写入的入口文本及其 include 闭包。
pub(crate) fn load_path_text(path: &Path, input: &str) -> Result<CompiledProject, ConfigError> {
    include::load_with_entry(path, input.as_bytes()).result
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
    let mut raw = parse_raw(input, format)?;
    if raw.has_includes() {
        return Err(ConfigError::Include(
            "include 必须通过文件路径加载，内存文本没有安全的相对路径基准".to_owned(),
        ));
    }
    raw.load_env_files(
        base_directory,
        None,
        &mut BTreeMap::new(),
        &mut BTreeSet::new(),
    )
    .map_err(validation_error)?;
    compile_raw(raw, base_directory)
}

/// 规范化已经完成 include 合并的原始 DTO 并编译任务图。
fn compile_raw(
    raw: RawProject,
    base_directory: Option<&Path>,
) -> Result<CompiledProject, ConfigError> {
    let (spec, dependencies, declarations) =
        raw.normalize(base_directory).map_err(validation_error)?;
    let graph = TaskGraph::compile(&spec)?;
    Ok(CompiledProject {
        vars: declarations.vars,
        resolved_vars: declarations.resolved_vars,
        variable_references: declarations.variable_references,
        spec,
        graph,
        dependencies,
        project_env: declarations.project_env,
        declared_project_env: declarations.declared_project_env,
        task_defaults: declarations.task_defaults,
        declared_task_defaults: declarations.declared_task_defaults,
        active_profile: declarations.active_profile,
        profile_extends: declarations.profile_extends,
        profile_names: declarations.profiles.keys().cloned().collect(),
        profiles: declarations.profiles,
        task_template_names: declarations.task_templates.keys().cloned().collect(),
        task_templates: declarations.task_templates,
        task_extends: declarations.task_extends,
        task_env_files: declarations.task_env_files,
        task_inline_env: declarations.task_inline_env,
        task_origins: declarations.task_origins,
        task_declarations: declarations.task_declarations,
        inactive_tasks: declarations.inactive_tasks,
    })
}

/// 按脚本目录解析 Python 生成的严格 JSON 文档。
pub(super) fn load_generated_json(input: &str, base_directory: &Path) -> GeneratedJsonCapture {
    let mut inputs = BTreeMap::new();
    let mut watched_paths = BTreeSet::new();
    let result = parse_raw(input, ConfigFormat::Json).and_then(|mut raw| {
        if raw.has_includes() {
            return Err(ConfigError::Include(
                "Python 生成配置不能声明 include".to_owned(),
            ));
        }
        raw.load_env_files(
            Some(base_directory),
            Some(base_directory),
            &mut inputs,
            &mut watched_paths,
        )
        .map_err(validation_error)?;
        compile_raw(raw, Some(base_directory))
    });
    GeneratedJsonCapture {
        result,
        inputs: inputs
            .into_iter()
            .map(|(path, bytes)| CapturedConfigInput { path, bytes })
            .collect(),
        watched_paths: watched_paths.into_iter().collect(),
    }
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
