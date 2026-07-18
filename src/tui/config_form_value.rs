//! 结构化表单字段的文本转换与通用校验。

use std::collections::BTreeMap;

use super::config_form::FormTaskDependency;

/// 将环境变量映射转换为弹窗文本。
pub(super) fn map_text(values: &BTreeMap<String, String>) -> String {
    serde_json::to_string(values).expect("字符串映射序列化不会失败")
}

/// 将依赖映射转换为弹窗文本。
pub(super) fn dependencies_text(values: &BTreeMap<String, FormTaskDependency>) -> String {
    values
        .iter()
        .map(|(name, dependency)| format!("{name}:{}", dependency.condition))
        .collect::<Vec<_>>()
        .join(",")
}

/// 解析逗号分隔的环境变量集合。
pub(super) fn parse_map(value: &str, label: &str) -> Result<BTreeMap<String, String>, String> {
    let value = value.trim();
    if value.starts_with('{') {
        return serde_json::from_str(value).map_err(|error| format!("{label} JSON 无效：{error}"));
    }
    value
        .split(',')
        .filter(|item| !item.trim().is_empty())
        .map(|item| {
            let Some((key, value)) = item.split_once('=') else {
                return Err(format!("{label} 必须使用 KEY=VALUE 格式"));
            };
            let key = key.trim();
            require(key, label)?;
            Ok((key.to_owned(), value.trim().to_owned()))
        })
        .collect()
}

/// 解析逗号分隔的 Task 依赖集合。
pub(super) fn parse_dependencies(
    value: &str,
) -> Result<BTreeMap<String, FormTaskDependency>, String> {
    let mut dependencies = BTreeMap::new();
    for item in value.split(',').filter(|item| !item.trim().is_empty()) {
        let (name, condition) = item.split_once(':').unwrap_or((item, "started"));
        let name = name.trim();
        require(name, "依赖 Task")?;
        let condition = match condition.trim() {
            "started" | "process_started" => "started",
            "healthy" | "process_healthy" => "healthy",
            "completed_successfully" | "process_completed_successfully" => "completed_successfully",
            _ => {
                return Err("依赖条件只能是 started、healthy 或 completed_successfully".to_owned());
            }
        };
        if dependencies
            .insert(
                name.to_owned(),
                FormTaskDependency {
                    condition: condition.to_owned(),
                },
            )
            .is_some()
        {
            return Err(format!("依赖 Task `{name}` 重复出现"));
        }
    }
    Ok(dependencies)
}

/// 替换或新增一个带名称的配置条目，并防止意外覆盖。
pub(super) fn replace_entry<T>(
    entries: &mut BTreeMap<String, T>,
    original: Option<&str>,
    name: &str,
    value: T,
    label: &str,
) -> Result<(), String> {
    if original != Some(name) && entries.contains_key(name) {
        return Err(format!("{label} 名称 `{name}` 已存在"));
    }
    if let Some(original) = original {
        entries.remove(original);
    }
    entries.insert(name.to_owned(), value);
    Ok(())
}

/// 确保必填字段包含非空文本。
fn require(value: &str, label: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        Err(format!("{label} 不能为空"))
    } else {
        Ok(())
    }
}

/// 读取必填字段，并移除首尾空白。
pub(super) fn required_value(value: &str, label: &str) -> Result<String, String> {
    require(value, label)?;
    Ok(value.trim().to_owned())
}

/// 把可空文本转为可选字段。
pub(super) fn optional(value: &str) -> Option<String> {
    (!value.trim().is_empty()).then(|| value.trim().to_owned())
}

/// 将空白分隔的参数文本转换为参数数组。
pub(super) fn args_text(values: &[String]) -> String {
    serde_json::to_string(values).expect("字符串数组序列化不会失败")
}

/// 解析精确 JSON 参数数组，并兼容旧版空格分隔输入。
pub(super) fn parse_args(value: &str, label: &str) -> Result<Vec<String>, String> {
    let value = value.trim();
    if value.starts_with('[') {
        return serde_json::from_str(value).map_err(|error| format!("{label} JSON 无效：{error}"));
    }
    Ok(value.split_whitespace().map(str::to_owned).collect())
}

/// 解析一个带单位的紧凑时长字段。
pub(super) fn parse_duration(value: &str, label: &str) -> Result<u64, String> {
    crate::config::parse_duration(value).map_err(|error| format!("{label}无效：{error}"))
}

/// 解析表单中的非负 32 位整数。
pub(super) fn parse_u32(value: &str, label: &str) -> Result<u32, String> {
    value
        .trim()
        .parse()
        .map_err(|_| format!("{label} 必须是非负整数"))
}

/// 解析精确 JSON 整数数组，并兼容逗号或空白分隔输入。
pub(super) fn parse_i32_list(value: &str, label: &str) -> Result<Vec<i32>, String> {
    let value = value.trim();
    let values = if value.starts_with('[') {
        serde_json::from_str(value).map_err(|error| format!("{label} JSON 无效：{error}"))?
    } else {
        value
            .split([',', ' '])
            .filter(|item| !item.trim().is_empty())
            .map(|item| {
                item.trim()
                    .parse()
                    .map_err(|_| format!("{label} 必须是整数数组"))
            })
            .collect::<Result<Vec<_>, _>>()?
    };
    if values.iter().any(|value| *value < 0) {
        return Err(format!("{label}不能包含负数"));
    }
    Ok(values)
}
