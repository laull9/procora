use std::{fs, path::Path};

use anyhow::{Context, bail};

use super::TemplateFormat;

/// 在目标目录创建指定格式的示例服务配置。
pub fn initialize(
    directory: &Path,
    format: TemplateFormat,
    force: bool,
) -> anyhow::Result<std::path::PathBuf> {
    let project = project_name(directory);
    let (filename, content) = template(format, &project);
    let path = directory.join(filename);
    if path.exists() && !force {
        bail!(
            "配置文件 `{}` 已存在；使用 --force 明确覆盖",
            path.display()
        );
    }
    fs::create_dir_all(directory)
        .with_context(|| format!("无法创建项目目录 `{}`", directory.display()))?;
    fs::write(&path, content).with_context(|| format!("无法写入模板配置 `{}`", path.display()))?;
    println!("已创建示例服务 `{project}`：{}", path.display());
    Ok(path)
}

/// 从目录名生成符合服务名称约束的模板项目名。
fn project_name(directory: &Path) -> String {
    let candidate = directory
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("example")
        .to_ascii_lowercase();
    let mut name = candidate
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' || character == '_' {
                character
            } else {
                '-'
            }
        })
        .collect::<String>();
    while name.starts_with(|character: char| !character.is_ascii_alphanumeric()) {
        name.remove(0);
    }
    if name.is_empty() {
        "example".to_owned()
    } else {
        name
    }
}

/// 返回配置文件名和对应格式的完整示例。
fn template(format: TemplateFormat, project: &str) -> (&'static str, String) {
    match format {
        TemplateFormat::Yaml => (
            "procora.yaml",
            format!(
                "version: 1\nproject: {project}\n\n# 使用 `procora edit` 打开带字段说明的配置编辑页。\ntasks:\n  example:\n    command: procora\n    args: [\"doctor\"]\n"
            ),
        ),
        TemplateFormat::Json => (
            "procora.json",
            format!(
                "{{\n  \"version\": 1,\n  \"project\": \"{project}\",\n  \"tasks\": {{\n    \"example\": {{\n      \"command\": \"procora\",\n      \"args\": [\"doctor\"]\n    }}\n  }}\n}}\n"
            ),
        ),
        TemplateFormat::Toml => (
            "procora.toml",
            format!(
                "version = 1\nproject = \"{project}\"\n\n[tasks.example]\ncommand = \"procora\"\nargs = [\"doctor\"]\n"
            ),
        ),
    }
}
