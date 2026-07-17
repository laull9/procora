//! 管理依赖基础与高级弹窗的提交逻辑。

use super::{
    config_form::{
        FormConfig, FormDependency, FormDependencyDownload, FormDependencySsh, FormVerify,
    },
    config_form_dialog::{
        DialogField, optional, parse_args, parse_duration, parse_map, replace_entry, required_value,
    },
};

/// 提交只包含高频字段的依赖弹窗。
pub(super) fn commit_basic(
    original: Option<&str>,
    baseline: &FormDependency,
    fields: &[DialogField],
    config: &mut FormConfig,
) -> Result<(), String> {
    let name = required_value(&fields[0].value, "依赖名称")?;
    let mut dependency = baseline.clone();
    dependency.source = required_value(&fields[1].value, "来源")?;
    dependency.version = optional(&fields[2].value).unwrap_or_else(|| "source".to_owned());
    dependency.checksum = optional(&fields[3].value);
    dependency.kind.clone_from(&fields[4].value);
    dependency.path = optional(&fields[5].value);
    replace_entry(
        &mut config.dependencies,
        original,
        &name,
        dependency,
        "管理依赖",
    )
}

/// 提交镜像、传输和版本验证高级策略。
pub(super) fn commit_advanced(
    name: &str,
    baseline: &FormDependency,
    fields: &[DialogField],
    config: &mut FormConfig,
) -> Result<(), String> {
    let mirrors = parse_args(&fields[0].value, "镜像")?;
    let retries = fields[2]
        .value
        .trim()
        .parse::<u8>()
        .map_err(|_| "失败重试次数必须是 0 到 255 的整数".to_owned())?;
    let timeout_ms = parse_duration(&fields[3].value, "单次总超时")?;
    let max_bytes = fields[4]
        .value
        .trim()
        .parse::<u64>()
        .map_err(|_| "最大下载字节必须是非负整数".to_owned())?;
    let headers = parse_map(&fields[5].value, "HTTP 请求头")?;
    let identity_file = optional(&fields[6].value);
    let known_hosts_file = optional(&fields[7].value);
    let command = optional(&fields[8].value);
    let args = parse_args(&fields[9].value, "验证参数")?;
    let contains = optional(&fields[10].value);
    let mut dependency = baseline.clone();
    dependency.mirrors = mirrors;
    dependency.unpack.clone_from(&fields[1].value);
    dependency.verify =
        (command.is_some() || !args.is_empty() || contains.is_some()).then_some(FormVerify {
            command,
            args,
            contains,
        });
    dependency.download = FormDependencyDownload {
        retries,
        timeout_ms,
        max_bytes,
        headers,
    };
    dependency.ssh = FormDependencySsh {
        identity_file,
        known_hosts_file,
    };
    config.dependencies.insert(name.to_owned(), dependency);
    Ok(())
}
