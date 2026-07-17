use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::Read as _,
    path::{Component, Path, PathBuf},
};

use super::{ConfigDiagnostic, RawProject, diagnostic};

/// 单个显式环境文件的最大字节数。
const MAX_ENV_FILE_BYTES: u64 = 1024 * 1024;
/// 单次配置闭包中不同环境文件的最大总字节数。
const MAX_ENV_TOTAL_BYTES: usize = 4 * 1024 * 1024;
/// 单个环境文件最多接受的环境变量数量。
const MAX_ENV_VARIABLES: usize = 4096;

impl RawProject {
    /// 读取显式环境文件，将其合并进 Task 环境并捕获修订输入。
    pub(crate) fn load_env_files(
        &mut self,
        base_directory: Option<&Path>,
        root: Option<&Path>,
        inputs: &mut BTreeMap<PathBuf, Vec<u8>>,
        watched_paths: &mut BTreeSet<PathBuf>,
    ) -> Result<(), Vec<ConfigDiagnostic>> {
        let mut diagnostics = Vec::new();
        self.apply_profile(&mut diagnostics);
        self.resolve_task_templates(&mut diagnostics);
        let mut env_inputs = BTreeSet::new();
        let mut total_bytes = 0_usize;
        for (task_id, task) in &mut self.tasks {
            let Some(configured_path) = task.env_file.as_ref() else {
                continue;
            };
            let field = format!("tasks.{task_id}.env_file");
            let Some(attempted) = resolve_path(configured_path, base_directory) else {
                diagnostics.push(diagnostic(
                    field,
                    "相对路径必须通过配置文件路径加载，不能从无路径文本读取",
                ));
                continue;
            };
            watched_paths.insert(attempted.clone());
            let Some(canonical) = canonical_env_path(&attempted, root, &field, &mut diagnostics)
            else {
                continue;
            };
            watched_paths.insert(canonical.clone());
            let Some(bytes) = read_env_file(&canonical, &field, &mut diagnostics) else {
                continue;
            };
            if env_inputs.insert(canonical.clone()) {
                total_bytes = total_bytes.saturating_add(bytes.len());
                if total_bytes > MAX_ENV_TOTAL_BYTES {
                    diagnostics.push(diagnostic(
                        &field,
                        format!("环境文件总量不能超过 {MAX_ENV_TOTAL_BYTES} 字节"),
                    ));
                    continue;
                }
            }
            let Some(mut file_env) = parse_env_file(&bytes, &field, &mut diagnostics) else {
                inputs.insert(canonical, bytes);
                continue;
            };
            inputs.insert(canonical, bytes);
            let inline_env = std::mem::take(&mut task.env);
            file_env.extend(inline_env.clone());
            task.env = file_env;
            task.inline_env_before_file = Some(inline_env);
        }
        if diagnostics.is_empty() {
            Ok(())
        } else {
            Err(diagnostics)
        }
    }
}

/// 解析相对声明目录的环境文件路径。
fn resolve_path(path: &Path, base_directory: Option<&Path>) -> Option<PathBuf> {
    if path.is_absolute() {
        return Some(normalize_path(path));
    }
    base_directory.map(|base| normalize_path(&base.join(path)))
}

/// 解析真实路径并阻止通过绝对路径或符号链接越过服务根目录。
fn canonical_env_path(
    attempted: &Path,
    root: Option<&Path>,
    field: &str,
    diagnostics: &mut Vec<ConfigDiagnostic>,
) -> Option<PathBuf> {
    if let Some(root) = root
        && !attempted.starts_with(root)
    {
        diagnostics.push(diagnostic(field, "必须位于服务根目录内"));
        return None;
    }
    let canonical = match std::fs::canonicalize(attempted) {
        Ok(path) => path,
        Err(error) => {
            diagnostics.push(diagnostic(
                field,
                format!("无法读取 `{}`：{error}", attempted.display()),
            ));
            return None;
        }
    };
    if let Some(root) = root
        && !canonical.starts_with(root)
    {
        diagnostics.push(diagnostic(field, "不能通过符号链接越过服务根目录"));
        return None;
    }
    Some(canonical)
}

/// 有界读取单个 UTF-8 环境文件。
fn read_env_file(
    path: &Path,
    field: &str,
    diagnostics: &mut Vec<ConfigDiagnostic>,
) -> Option<Vec<u8>> {
    let metadata = match std::fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) => {
            diagnostics.push(diagnostic(field, format!("读取元数据失败：{error}")));
            return None;
        }
    };
    if !metadata.is_file() {
        diagnostics.push(diagnostic(field, "必须指向普通文件"));
        return None;
    }
    if metadata.len() > MAX_ENV_FILE_BYTES {
        diagnostics.push(diagnostic(
            field,
            format!("不能超过 {MAX_ENV_FILE_BYTES} 字节"),
        ));
        return None;
    }
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) => {
            diagnostics.push(diagnostic(field, format!("打开失败：{error}")));
            return None;
        }
    };
    let mut bytes = Vec::with_capacity(metadata.len().min(64 * 1024) as usize);
    if let Err(error) = file.take(MAX_ENV_FILE_BYTES + 1).read_to_end(&mut bytes) {
        diagnostics.push(diagnostic(field, format!("读取失败：{error}")));
        return None;
    }
    if bytes.len() as u64 > MAX_ENV_FILE_BYTES {
        diagnostics.push(diagnostic(
            field,
            format!("读取期间增长并超过 {MAX_ENV_FILE_BYTES} 字节"),
        ));
        return None;
    }
    Some(bytes)
}

/// 解析受限且确定性的 dotenv 文本，不执行变量展开。
fn parse_env_file(
    bytes: &[u8],
    field: &str,
    diagnostics: &mut Vec<ConfigDiagnostic>,
) -> Option<BTreeMap<String, String>> {
    let text = match std::str::from_utf8(bytes) {
        Ok(text) => text.strip_prefix('\u{feff}').unwrap_or(text),
        Err(error) => {
            diagnostics.push(diagnostic(field, format!("必须是 UTF-8：{error}")));
            return None;
        }
    };
    let mut variables = BTreeMap::new();
    for (index, line) in text.lines().enumerate() {
        let line_number = index + 1;
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").map_or(line, str::trim_start);
        let Some((key, raw_value)) = line.split_once('=') else {
            diagnostics.push(diagnostic(field, format!("第 {line_number} 行缺少 `=`")));
            continue;
        };
        let key = key.trim();
        if !valid_key(key) {
            diagnostics.push(diagnostic(
                field,
                format!("第 {line_number} 行变量名 `{key}` 不合法"),
            ));
            continue;
        }
        match parse_value(raw_value.trim_start()) {
            Ok(value) if !value.contains('\0') => {
                variables.insert(key.to_owned(), value);
                if variables.len() > MAX_ENV_VARIABLES {
                    diagnostics.push(diagnostic(
                        field,
                        format!("变量数量不能超过 {MAX_ENV_VARIABLES}"),
                    ));
                    return None;
                }
            }
            Ok(_) => diagnostics.push(diagnostic(
                field,
                format!("第 {line_number} 行值不能包含 NUL 字符"),
            )),
            Err(message) => {
                diagnostics.push(diagnostic(field, format!("第 {line_number} 行{message}")));
            }
        }
    }
    Some(variables)
}

/// 判断变量名能否在三平台稳定传给子进程。
fn valid_key(key: &str) -> bool {
    let mut bytes = key.bytes();
    bytes
        .next()
        .is_some_and(|byte| byte == b'_' || byte.is_ascii_alphabetic())
        && bytes.all(|byte| byte == b'_' || byte.is_ascii_alphanumeric())
}

/// 解析单行 dotenv 值以及可选行尾注释。
fn parse_value(value: &str) -> Result<String, &'static str> {
    if let Some(rest) = value.strip_prefix('\'') {
        return parse_single_quoted(rest);
    }
    if let Some(rest) = value.strip_prefix('"') {
        return parse_double_quoted(rest);
    }
    let comment = value.char_indices().find_map(|(index, character)| {
        (character == '#'
            && value[..index]
                .chars()
                .next_back()
                .is_some_and(char::is_whitespace))
        .then_some(index)
    });
    Ok(value[..comment.unwrap_or(value.len())]
        .trim_end()
        .to_owned())
}

/// 解析不处理转义的单引号值。
fn parse_single_quoted(value: &str) -> Result<String, &'static str> {
    let Some(end) = value.find('\'') else {
        return Err("单引号没有闭合");
    };
    validate_suffix(&value[end + 1..])?;
    Ok(value[..end].to_owned())
}

/// 解析支持少量可移植转义的双引号值。
fn parse_double_quoted(value: &str) -> Result<String, &'static str> {
    let mut output = String::new();
    let mut escaped = false;
    for (index, character) in value.char_indices() {
        if escaped {
            output.push(match character {
                'n' => '\n',
                'r' => '\r',
                't' => '\t',
                '\\' => '\\',
                '"' => '"',
                _ => return Err("包含不支持的双引号转义"),
            });
            escaped = false;
        } else if character == '\\' {
            escaped = true;
        } else if character == '"' {
            validate_suffix(&value[index + character.len_utf8()..])?;
            return Ok(output);
        } else {
            output.push(character);
        }
    }
    Err("双引号没有闭合")
}

/// 引号结束后只允许空白或注释。
fn validate_suffix(suffix: &str) -> Result<(), &'static str> {
    let suffix = suffix.trim_start();
    if suffix.is_empty() || suffix.starts_with('#') {
        Ok(())
    } else {
        Err("引号结束后存在多余内容")
    }
}

/// 按词法规范化路径，不要求目标已经存在。
fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}
