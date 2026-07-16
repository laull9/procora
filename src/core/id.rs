use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// 任务标识校验错误。
#[derive(Debug, Error, PartialEq, Eq)]
pub enum TaskIdError {
    /// 任务标识为空。
    #[error("任务标识不能为空")]
    Empty,
    /// 任务标识含有非法字符。
    #[error("任务标识 `{0}` 只能包含 ASCII 字母、数字、点、短横线和下划线")]
    InvalidCharacters(String),
}

/// 配置内稳定且经过校验的任务标识。
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(try_from = "String", into = "String")]
pub struct TaskId(String);

impl TaskId {
    /// 返回任务标识文本。
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for TaskId {
    type Error = TaskIdError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.is_empty() {
            return Err(TaskIdError::Empty);
        }
        if !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
        {
            return Err(TaskIdError::InvalidCharacters(value));
        }
        Ok(Self(value))
    }
}

impl FromStr for TaskId {
    type Err = TaskIdError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::try_from(value.to_owned())
    }
}

impl From<TaskId> for String {
    fn from(value: TaskId) -> Self {
        value.0
    }
}

impl fmt::Display for TaskId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}
