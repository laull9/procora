use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use rusqlite::{Connection, OptionalExtension, params};

use super::StorageError;

/// 当前 `SQLite` 状态数据库的模式版本。
pub const STORAGE_SCHEMA_VERSION: u32 = 1;

/// `SQLite` 中保存的稳定服务状态。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StoredServiceStatus {
    /// 服务宿主已经加载并具有运行期望。
    Running,
    /// 服务仍注册但没有运行期望。
    Stopped,
    /// 服务加载或生命周期操作失败。
    Failed,
}

impl StoredServiceStatus {
    /// 返回数据库中使用的稳定文本值。
    const fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Stopped => "stopped",
            Self::Failed => "failed",
        }
    }

    /// 从数据库稳定文本值恢复状态。
    fn parse(value: String) -> Result<Self, StorageError> {
        match value.as_str() {
            "running" => Ok(Self::Running),
            "stopped" => Ok(Self::Stopped),
            "failed" => Ok(Self::Failed),
            _ => Err(StorageError::InvalidStatus(value)),
        }
    }
}

/// 单个托管服务的可恢复状态信息。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredService {
    /// 配置中的稳定服务名称。
    pub name: String,
    /// 服务目录的规范化路径。
    pub root: PathBuf,
    /// 被选中的配置文件规范化路径。
    pub config_path: PathBuf,
    /// 中心服务器重启后是否应恢复运行期望。
    pub desired_running: bool,
    /// 最近一次持久化的服务状态。
    pub status: StoredServiceStatus,
    /// 最近一次失败或降级说明。
    pub message: Option<String>,
    /// 最近一次成功加载的任务数量。
    pub task_count: usize,
}

/// 服务状态历史中的单条记录。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServiceStatusRecord {
    /// 记录对应的服务名称。
    pub service_name: String,
    /// 当时的服务状态。
    pub status: StoredServiceStatus,
    /// 当时的错误或降级说明。
    pub message: Option<String>,
    /// Unix 纪元后的毫秒时间戳。
    pub recorded_at_ms: i64,
}

/// 使用 `SQLite` 保存中心服务器注册表和服务状态历史的仓库。
#[derive(Clone, Debug)]
pub struct SqliteCenterRepository {
    path: PathBuf,
}

impl SqliteCenterRepository {
    /// 创建使用指定 `SQLite` 文件的中心状态仓库。
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// 返回当前 `SQLite` 数据库路径。
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 读取按名称排序的全部服务状态。
    ///
    /// # Errors
    ///
    /// 当数据库无法打开、迁移或查询时返回错误。
    pub fn load_services(&self) -> Result<Vec<StoredService>, StorageError> {
        let connection = self.open()?;
        let mut statement = connection.prepare(
            "SELECT name, root, config_path, desired_running, status, message, task_count
             FROM services ORDER BY name",
        )?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Vec<u8>>(1)?,
                    row.get::<_, Vec<u8>>(2)?,
                    row.get::<_, bool>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, i64>(6)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        rows.into_iter()
            .map(
                |(name, root, config_path, desired_running, status, message, task_count)| {
                    Ok(StoredService {
                        name,
                        root: bytes_to_path(&root),
                        config_path: bytes_to_path(&config_path),
                        desired_running,
                        status: StoredServiceStatus::parse(status)?,
                        message,
                        task_count: usize::try_from(task_count)
                            .map_err(|_| StorageError::NumericOverflow)?,
                    })
                },
            )
            .collect()
    }

    /// 写入服务当前状态，并在状态或消息变化时追加历史记录。
    ///
    /// # Errors
    ///
    /// 当数据库无法打开或事务提交失败时返回错误。
    pub fn save_service(&self, service: &StoredService) -> Result<(), StorageError> {
        let mut connection = self.open()?;
        let transaction = connection.transaction()?;
        let previous = transaction
            .query_row(
                "SELECT status, message FROM services WHERE name = ?1",
                [&service.name],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
            )
            .optional()?;
        let now = now_millis();
        let task_count =
            i64::try_from(service.task_count).map_err(|_| StorageError::NumericOverflow)?;
        transaction.execute(
            "INSERT INTO services (
                name, root, config_path, desired_running, status, message, task_count, updated_at_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(name) DO UPDATE SET
                root = excluded.root,
                config_path = excluded.config_path,
                desired_running = excluded.desired_running,
                status = excluded.status,
                message = excluded.message,
                task_count = excluded.task_count,
                updated_at_ms = excluded.updated_at_ms",
            params![
                &service.name,
                path_to_bytes(&service.root),
                path_to_bytes(&service.config_path),
                service.desired_running,
                service.status.as_str(),
                service.message.as_deref(),
                task_count,
                now,
            ],
        )?;
        let current = (service.status.as_str().to_owned(), service.message.clone());
        if previous.as_ref() != Some(&current) {
            transaction.execute(
                "INSERT INTO service_status_history (
                    service_name, status, message, recorded_at_ms
                 ) VALUES (?1, ?2, ?3, ?4)",
                params![
                    &service.name,
                    service.status.as_str(),
                    service.message.as_deref(),
                    now
                ],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    /// 删除服务当前状态，并通过外键级联删除全部状态历史。
    ///
    /// # Errors
    ///
    /// 当数据库无法打开或删除事务失败时返回错误。
    pub fn remove_service(&self, service_name: &str) -> Result<bool, StorageError> {
        let connection = self.open()?;
        let affected =
            connection.execute("DELETE FROM services WHERE name = ?1", [service_name])?;
        Ok(affected != 0)
    }

    /// 查询指定服务按写入顺序排列的状态历史。
    ///
    /// # Errors
    ///
    /// 当数据库无法打开或历史记录无法解码时返回错误。
    pub fn status_history(
        &self,
        service_name: &str,
    ) -> Result<Vec<ServiceStatusRecord>, StorageError> {
        let connection = self.open()?;
        let mut statement = connection.prepare(
            "SELECT service_name, status, message, recorded_at_ms
             FROM service_status_history WHERE service_name = ?1 ORDER BY id",
        )?;
        let rows = statement
            .query_map([service_name], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        rows.into_iter()
            .map(|(service_name, status, message, recorded_at_ms)| {
                Ok(ServiceStatusRecord {
                    service_name,
                    status: StoredServiceStatus::parse(status)?,
                    message,
                    recorded_at_ms,
                })
            })
            .collect()
    }

    /// 打开数据库、启用约束并执行首版模式迁移。
    fn open(&self) -> Result<Connection, StorageError> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let connection = Connection::open(&self.path)?;
        connection.busy_timeout(Duration::from_secs(2))?;
        connection.execute_batch("PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL;")?;
        initialize_schema(&connection)?;
        Ok(connection)
    }
}

/// 初始化空数据库并拒绝未知模式版本。
fn initialize_schema(connection: &Connection) -> Result<(), StorageError> {
    let version = connection.query_row("PRAGMA user_version", [], |row| row.get::<_, u32>(0))?;
    if version > STORAGE_SCHEMA_VERSION {
        return Err(StorageError::UnsupportedSchema(version));
    }
    if version == 0 {
        connection.execute_batch(
            "BEGIN;
             CREATE TABLE services (
                name TEXT PRIMARY KEY,
                root BLOB NOT NULL UNIQUE,
                config_path BLOB NOT NULL UNIQUE,
                desired_running INTEGER NOT NULL CHECK (desired_running IN (0, 1)),
                status TEXT NOT NULL CHECK (status IN ('running', 'stopped', 'failed')),
                message TEXT,
                task_count INTEGER NOT NULL CHECK (task_count >= 0),
                updated_at_ms INTEGER NOT NULL
             );
             CREATE TABLE service_status_history (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                service_name TEXT NOT NULL,
                status TEXT NOT NULL CHECK (status IN ('running', 'stopped', 'failed')),
                message TEXT,
                recorded_at_ms INTEGER NOT NULL,
                FOREIGN KEY(service_name) REFERENCES services(name) ON DELETE CASCADE
             );
             CREATE INDEX service_status_history_service
                ON service_status_history(service_name, id);
             PRAGMA user_version = 1;
             COMMIT;",
        )?;
    }
    Ok(())
}

/// 返回当前 Unix 毫秒时间戳。
fn now_millis() -> i64 {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    i64::try_from(millis).unwrap_or(i64::MAX)
}

/// 把当前平台路径编码为 `SQLite` BLOB。
#[cfg(unix)]
fn path_to_bytes(path: &Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;

    path.as_os_str().as_bytes().to_vec()
}

/// 从 `SQLite` BLOB 恢复当前平台路径。
#[cfg(unix)]
fn bytes_to_path(bytes: &[u8]) -> PathBuf {
    use std::os::unix::ffi::OsStringExt;

    std::ffi::OsString::from_vec(bytes.to_vec()).into()
}

/// 把 Windows UTF-16 路径编码为 `SQLite` BLOB。
#[cfg(windows)]
fn path_to_bytes(path: &Path) -> Vec<u8> {
    use std::os::windows::ffi::OsStrExt;

    path.as_os_str()
        .encode_wide()
        .flat_map(u16::to_le_bytes)
        .collect()
}

/// 从 `SQLite` BLOB 恢复 Windows UTF-16 路径。
#[cfg(windows)]
fn bytes_to_path(bytes: &[u8]) -> PathBuf {
    use std::os::windows::ffi::OsStringExt;

    let wide = bytes
        .chunks_exact(2)
        .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
        .collect::<Vec<_>>();
    std::ffi::OsString::from_wide(&wide).into()
}
