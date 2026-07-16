use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Component, Path, PathBuf},
};

use super::{CapturedConfigInput, ConfigLoadCapture, compile_raw, parse_raw};
use crate::config::{ConfigError, ConfigFormat, raw::RawProject};

/// include 递归深度上限，避免恶意或误配置耗尽栈。
const MAX_INCLUDE_DEPTH: usize = 16;
/// 单次闭包最多读取的文档数量。
const MAX_INCLUDE_DOCUMENTS: usize = 64;
/// 单次闭包原始文本总字节上限。
const MAX_INCLUDE_BYTES: usize = 4 * 1024 * 1024;

/// 一次递归加载共享的边界、输入与循环检测状态。
struct IncludeContext {
    root: PathBuf,
    expected_version: u32,
    expected_project: String,
    stack: Vec<PathBuf>,
    documents: usize,
    total_bytes: usize,
    inputs: BTreeMap<PathBuf, Vec<u8>>,
    watched_paths: BTreeSet<PathBuf>,
}

/// 读取入口和完整 include 闭包，并始终返回可用于监听的捕获信息。
pub(super) fn load(path: &Path) -> ConfigLoadCapture {
    load_capture(path, None)
}

/// 使用尚未写盘的入口字节校验完整闭包。
pub(super) fn load_with_entry(path: &Path, entry_bytes: &[u8]) -> ConfigLoadCapture {
    load_capture(path, Some(entry_bytes))
}

/// 读取磁盘入口或使用调用方覆盖，并汇总闭包捕获信息。
fn load_capture(path: &Path, entry_override: Option<&[u8]>) -> ConfigLoadCapture {
    let entry = absolute_path(path);
    let root = entry
        .parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
    let mut capture_root = fs::canonicalize(&root).unwrap_or(root);
    let mut inputs = BTreeMap::new();
    let mut watched_paths = BTreeSet::from([entry.clone()]);
    let result = load_entry(
        &entry,
        entry_override,
        &mut capture_root,
        &mut inputs,
        &mut watched_paths,
    );
    ConfigLoadCapture {
        result,
        inputs: inputs
            .into_iter()
            .map(|(path, bytes)| CapturedConfigInput { path, bytes })
            .collect(),
        watched_paths: watched_paths.into_iter().collect(),
        root: capture_root,
    }
}

/// 建立入口约束后递归合并所有文档。
fn load_entry(
    entry: &Path,
    entry_override: Option<&[u8]>,
    root: &mut PathBuf,
    inputs: &mut BTreeMap<PathBuf, Vec<u8>>,
    watched_paths: &mut BTreeSet<PathBuf>,
) -> Result<crate::config::CompiledProject, ConfigError> {
    let canonical = fs::canonicalize(entry).map_err(|source| ConfigError::Read {
        path: entry.to_path_buf(),
        source,
    })?;
    *root = canonical
        .parent()
        .expect("配置文件应具有父目录")
        .to_path_buf();
    watched_paths.clear();
    watched_paths.insert(canonical.clone());
    let bytes =
        entry_override.map_or_else(|| read_bytes(&canonical), |bytes| Ok(bytes.to_vec()))?;
    let entry_bytes = bytes.len();
    inputs.insert(canonical.clone(), bytes);
    if entry_bytes > MAX_INCLUDE_BYTES {
        return Err(ConfigError::Include(format!(
            "原始文本总量超过 {MAX_INCLUDE_BYTES} 字节"
        )));
    }
    let mut raw = parse_bytes(&canonical, &inputs[&canonical])?;
    let version = raw.declared_version().ok_or_else(|| {
        ConfigError::Include("入口文件必须显式声明 version，不能从 include 继承".to_owned())
    })?;
    let project = raw.declared_project().ok_or_else(|| {
        ConfigError::Include("入口文件必须显式声明 project，不能从 include 继承".to_owned())
    })?;
    let mut context = IncludeContext {
        root: root.clone(),
        expected_version: version,
        expected_project: project.to_owned(),
        stack: vec![canonical.clone()],
        documents: 1,
        total_bytes: entry_bytes,
        inputs: std::mem::take(inputs),
        watched_paths: std::mem::take(watched_paths),
    };
    let merged = merge_document(&canonical, &mut raw, 0, &mut context);
    *inputs = context.inputs;
    *watched_paths = context.watched_paths;
    compile_raw(merged?, None)
}

/// 以深度优先顺序合并当前文档声明的全部 include。
fn merge_document(
    path: &Path,
    raw: &mut RawProject,
    depth: usize,
    context: &mut IncludeContext,
) -> Result<RawProject, ConfigError> {
    validate_identity(path, raw, context)?;
    let include_paths = raw.take_includes();
    let base = path.parent().expect("规范化配置路径应具有父目录");
    raw.rebase(base);
    let current = std::mem::replace(raw, RawProject::empty());
    let mut merged = RawProject::empty();
    for include in include_paths {
        if depth + 1 > MAX_INCLUDE_DEPTH {
            return Err(ConfigError::Include(format!(
                "递归深度超过 {MAX_INCLUDE_DEPTH}：{}",
                path.display()
            )));
        }
        let included = resolve_include(base, &include, context)?;
        if let Some(position) = context.stack.iter().position(|item| item == &included) {
            let chain = context.stack[position..]
                .iter()
                .chain(std::iter::once(&included))
                .map(|item| item.display().to_string())
                .collect::<Vec<_>>()
                .join(" -> ");
            return Err(ConfigError::Include(format!("检测到循环：{chain}")));
        }
        context.documents += 1;
        if context.documents > MAX_INCLUDE_DOCUMENTS {
            return Err(ConfigError::Include(format!(
                "文档数量超过 {MAX_INCLUDE_DOCUMENTS}"
            )));
        }
        let bytes = read_bytes(&included)?;
        context.total_bytes = context.total_bytes.saturating_add(bytes.len());
        if context.total_bytes > MAX_INCLUDE_BYTES {
            return Err(ConfigError::Include(format!(
                "原始文本总量超过 {MAX_INCLUDE_BYTES} 字节"
            )));
        }
        context.inputs.insert(included.clone(), bytes);
        let mut child = parse_bytes(&included, &context.inputs[&included]).map_err(|source| {
            ConfigError::IncludedFile {
                path: included.clone(),
                source: Box::new(source),
            }
        })?;
        context.stack.push(included.clone());
        let child = merge_document(&included, &mut child, depth + 1, context)?;
        context.stack.pop();
        merged.overlay(child);
    }
    merged.overlay(current);
    Ok(merged)
}

/// 校验片段显式身份没有与入口冲突。
fn validate_identity(
    path: &Path,
    raw: &RawProject,
    context: &IncludeContext,
) -> Result<(), ConfigError> {
    if raw
        .declared_version()
        .is_some_and(|version| version != context.expected_version)
    {
        return Err(ConfigError::Include(format!(
            "{} 的 version 与入口不一致",
            path.display()
        )));
    }
    if raw
        .declared_project()
        .is_some_and(|project| project != context.expected_project)
    {
        return Err(ConfigError::Include(format!(
            "{} 的 project 与入口不一致",
            path.display()
        )));
    }
    Ok(())
}

/// 解析并约束单个 include 路径到入口服务根目录内。
fn resolve_include(
    base: &Path,
    include: &Path,
    context: &mut IncludeContext,
) -> Result<PathBuf, ConfigError> {
    if include.is_absolute()
        || !include
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
    {
        return Err(ConfigError::Include(format!(
            "`{}` 必须是不含点或父目录的相对路径",
            include.display()
        )));
    }
    let attempted = base.join(include);
    context.watched_paths.insert(attempted.clone());
    let canonical = fs::canonicalize(&attempted).map_err(|source| ConfigError::Read {
        path: attempted,
        source,
    })?;
    if !canonical.starts_with(&context.root) {
        return Err(ConfigError::Include(format!(
            "`{}` 通过符号链接越过服务根目录",
            include.display()
        )));
    }
    context.watched_paths.insert(canonical.clone());
    Ok(canonical)
}

/// 读取单个配置文件的原始字节。
fn read_bytes(path: &Path) -> Result<Vec<u8>, ConfigError> {
    let metadata = fs::metadata(path).map_err(|source| ConfigError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    if metadata.len() > MAX_INCLUDE_BYTES as u64 {
        return Err(ConfigError::Include(format!(
            "`{}` 超过单次闭包 {} 字节上限",
            path.display(),
            MAX_INCLUDE_BYTES
        )));
    }
    fs::read(path).map_err(|source| ConfigError::Read {
        path: path.to_path_buf(),
        source,
    })
}

/// 把已捕获字节按扩展名解析为原始 DTO。
fn parse_bytes(path: &Path, bytes: &[u8]) -> Result<RawProject, ConfigError> {
    let format = ConfigFormat::from_path(path)
        .ok_or_else(|| ConfigError::UnknownFormat(path.to_path_buf()))?;
    let input = std::str::from_utf8(bytes).map_err(|error| ConfigError::Parse {
        format,
        message: format!("配置必须是 UTF-8：{error}"),
    })?;
    parse_raw(input, format)
}

/// 返回不要求入口已经存在的绝对路径。
fn absolute_path(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    }
}
