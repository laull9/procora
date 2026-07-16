use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// 服务名称校验错误。
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ServiceNameError {
    /// 服务名称为空。
    #[error("服务名称不能为空")]
    Empty,
    /// 服务名称包含不允许的字符。
    #[error("服务名称 `{0}` 只能包含 ASCII 字母、数字、点、短横线和下划线")]
    InvalidCharacters(String),
}

/// 本机中心服务器中稳定且可用于命令定位的服务名称。
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(try_from = "String", into = "String")]
pub struct ServiceName(String);

impl ServiceName {
    /// 返回服务名称文本。
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for ServiceName {
    type Error = ServiceNameError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.is_empty() {
            return Err(ServiceNameError::Empty);
        }
        if !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
        {
            return Err(ServiceNameError::InvalidCharacters(value));
        }
        Ok(Self(value))
    }
}

impl FromStr for ServiceName {
    type Err = ServiceNameError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::try_from(value.to_owned())
    }
}

impl From<ServiceName> for String {
    fn from(value: ServiceName) -> Self {
        value.0
    }
}

impl fmt::Display for ServiceName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}
