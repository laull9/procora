use std::{fmt, path::Path};

/// Procora 支持的声明式配置格式。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfigFormat {
    /// YAML 配置。
    Yaml,
    /// TOML 配置。
    Toml,
    /// JSON 配置。
    Json,
}

impl ConfigFormat {
    /// 根据文件扩展名识别配置格式。
    pub fn from_path(path: &Path) -> Option<Self> {
        match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
            "yaml" | "yml" => Some(Self::Yaml),
            "toml" => Some(Self::Toml),
            "json" => Some(Self::Json),
            _ => None,
        }
    }
}

impl fmt::Display for ConfigFormat {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Yaml => formatter.write_str("YAML"),
            Self::Toml => formatter.write_str("TOML"),
            Self::Json => formatter.write_str("JSON"),
        }
    }
}
