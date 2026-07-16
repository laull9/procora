//! 中心服务器与服务宿主使用的 `SQLite` 状态存储。

mod center;

use thiserror::Error;

pub use center::{
    STORAGE_SCHEMA_VERSION, ServiceStatusRecord, SqliteCenterRepository, StoredService,
    StoredServiceStatus,
};

/// `SQLite` 状态仓库错误。
#[derive(Debug, Error)]
pub enum StorageError {
    /// 数据库父目录无法创建。
    #[error("无法创建状态数据库目录: {0}")]
    Io(#[from] std::io::Error),
    /// `SQLite` 操作失败。
    #[error("SQLite 状态存储失败: {0}")]
    Sqlite(#[from] rusqlite::Error),
    /// 数据库使用了当前程序无法理解的模式版本。
    #[error("不支持状态数据库模式版本 {0}")]
    UnsupportedSchema(u32),
    /// 数据库中出现未知服务状态。
    #[error("状态数据库包含未知服务状态 `{0}`")]
    InvalidStatus(String),
    /// 内存数值无法安全写入 `SQLite` 整数。
    #[error("状态数值超出 SQLite 整数范围")]
    NumericOverflow,
}
