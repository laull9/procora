use std::{
    fs::{self, File, OpenOptions},
    io::{self, BufReader, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{
        Mutex, PoisonError,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use crate::core::TaskId;
use flate2::{Compression, write::GzEncoder};
use thiserror::Error;

/// 同一纳秒内生成归档名时使用的进程内序号。
static ARCHIVE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// 文件日志轮转、压缩和保留策略。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FileLogPolicy {
    /// 单个活动日志达到该字节数后轮转。
    pub max_file_bytes: u64,
    /// 每个活动日志最多保留的 gzip 归档数量。
    pub max_archives: usize,
    /// 单个活动日志全部 gzip 归档允许占用的总字节数。
    pub max_archive_bytes: Option<u64>,
    /// gzip 归档允许保留的最长时间。
    pub max_archive_age: Option<Duration>,
}

impl Default for FileLogPolicy {
    fn default() -> Self {
        Self {
            max_file_bytes: 10 * 1024 * 1024,
            max_archives: 5,
            max_archive_bytes: Some(50 * 1024 * 1024),
            max_archive_age: Some(Duration::from_hours(168)),
        }
    }
}

/// 指向活动日志某一代文件字节位置的持久游标。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FileLogCursor {
    /// 每次活动文件轮转后递增的代次。
    pub generation: u64,
    /// 当前代活动文件中已经消费的字节偏移。
    pub offset: u64,
}

/// 从文件日志游标读取的一批有界字节。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileLogBatch {
    /// 从活动文件读取的原始字节。
    pub bytes: Vec<u8>,
    /// 下一次读取使用的游标。
    pub next_cursor: FileLogCursor,
    /// 原游标所指代次已轮转或偏移失效，返回内容从当前可用尾部恢复。
    pub gap: bool,
}

/// 活动日志旁持久化的最小游标索引。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct FileLogIndex {
    generation: u64,
    length: u64,
}

/// 服务目录内文件日志操作错误。
#[derive(Debug, Error)]
pub enum FileLogError {
    /// 轮转阈值不能为零。
    #[error("日志轮转阈值必须大于零")]
    InvalidPolicy,
    /// 日志目录或文件操作失败。
    #[error("文件日志操作失败: {0}")]
    Io(#[from] io::Error),
    /// 同一日志存储的并发写入锁已损坏。
    #[error("文件日志写入锁已损坏")]
    Poisoned,
    /// 活动日志游标索引已损坏。
    #[error("日志游标索引 `{path}` 无效")]
    InvalidIndex {
        /// 无法解析的索引路径。
        path: PathBuf,
    },
}

/// 把 Service 和 Task 日志写入所属服务目录的文件存储。
#[derive(Debug)]
pub struct FileLogStore {
    log_root: PathBuf,
    policy: FileLogPolicy,
    write_lock: Mutex<()>,
}

impl FileLogStore {
    /// 使用默认轮转策略创建服务目录文件日志存储。
    pub fn for_service(service_root: impl AsRef<Path>) -> Self {
        Self {
            log_root: service_root.as_ref().join(".procora").join("logs"),
            policy: FileLogPolicy::default(),
            write_lock: Mutex::new(()),
        }
    }

    /// 创建以 `<service>/.procora/logs` 为根目录的文件日志存储。
    ///
    /// # Errors
    ///
    /// 当轮转阈值为零时返回错误。
    pub fn new(
        service_root: impl AsRef<Path>,
        policy: FileLogPolicy,
    ) -> Result<Self, FileLogError> {
        if policy.max_file_bytes == 0 {
            return Err(FileLogError::InvalidPolicy);
        }
        Ok(Self {
            log_root: service_root.as_ref().join(".procora").join("logs"),
            policy,
            write_lock: Mutex::new(()),
        })
    }

    /// 返回服务级活动日志文件路径。
    pub fn service_log_path(&self) -> PathBuf {
        self.log_root.join("service.log")
    }

    /// 返回指定 Task 的活动日志文件路径。
    pub fn task_log_path(&self, task_id: &TaskId) -> PathBuf {
        self.log_root.join("tasks").join(format!("{task_id}.log"))
    }

    /// 向服务级日志追加原始字节。
    ///
    /// # Errors
    ///
    /// 当目录创建、轮转压缩或文件写入失败时返回错误。
    pub fn append_service(&self, bytes: &[u8]) -> Result<(), FileLogError> {
        self.append(&self.service_log_path(), bytes)
    }

    /// 向指定 Task 日志追加原始字节。
    ///
    /// # Errors
    ///
    /// 当目录创建、轮转压缩或文件写入失败时返回错误。
    pub fn append_task(&self, task_id: &TaskId, bytes: &[u8]) -> Result<(), FileLogError> {
        self.append(&self.task_log_path(task_id), bytes)
    }

    /// 清空指定 Task 的活动日志和全部轮转归档，并推进文件代次。
    ///
    /// # Errors
    ///
    /// 当日志目录、活动文件、归档或游标索引无法更新时返回错误。
    pub fn clear_task(&self, task_id: &TaskId) -> Result<(), FileLogError> {
        let path = self.task_log_path(task_id);
        let _guard = self.write_lock.lock().map_err(map_poisoned_lock)?;
        let length = path.metadata().map_or(0, |metadata| metadata.len());
        let index = load_index(&path, length)?;
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        remove_archives(&path)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        save_index(
            &path,
            FileLogIndex {
                generation: index.generation.saturating_add(1),
                length: 0,
            },
        )
    }

    /// 从可选游标开始读取服务级活动日志。
    ///
    /// # Errors
    ///
    /// 当日志或游标索引无法读取时返回错误。
    pub fn read_service(
        &self,
        cursor: Option<FileLogCursor>,
        max_bytes: usize,
    ) -> Result<FileLogBatch, FileLogError> {
        self.read(&self.service_log_path(), cursor, max_bytes)
    }

    /// 从可选游标开始读取指定 Task 的活动日志。
    ///
    /// # Errors
    ///
    /// 当日志或游标索引无法读取时返回错误。
    pub fn read_task(
        &self,
        task_id: &TaskId,
        cursor: Option<FileLogCursor>,
        max_bytes: usize,
    ) -> Result<FileLogBatch, FileLogError> {
        self.read(&self.task_log_path(task_id), cursor, max_bytes)
    }

    /// 串行执行单个活动日志的轮转检查和追加。
    fn append(&self, path: &Path, bytes: &[u8]) -> Result<(), FileLogError> {
        let _guard = self.write_lock.lock().map_err(map_poisoned_lock)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let current_size = path.metadata().map_or(0, |metadata| metadata.len());
        let mut index = load_index(path, current_size)?;
        let incoming_size = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        if current_size > 0
            && current_size.saturating_add(incoming_size) > self.policy.max_file_bytes
        {
            self.rotate(path)?;
            index.generation = index.generation.saturating_add(1);
            index.length = 0;
            save_index(path, index)?;
        }
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?
            .write_all(bytes)?;
        index.length = path.metadata()?.len();
        save_index(path, index)?;
        Ok(())
    }

    /// 串行读取活动日志，并在轮转或截断后报告 Gap。
    fn read(
        &self,
        path: &Path,
        cursor: Option<FileLogCursor>,
        max_bytes: usize,
    ) -> Result<FileLogBatch, FileLogError> {
        let _guard = self.write_lock.lock().map_err(map_poisoned_lock)?;
        let length = path.metadata().map_or(0, |metadata| metadata.len());
        let index = load_index(path, length)?;
        let requested = cursor.unwrap_or(FileLogCursor {
            generation: index.generation,
            offset: 0,
        });
        let gap = requested.generation != index.generation || requested.offset > length;
        let max_bytes_u64 = u64::try_from(max_bytes).unwrap_or(u64::MAX);
        let start = if gap {
            length.saturating_sub(max_bytes_u64)
        } else {
            requested.offset
        };
        let readable = length.saturating_sub(start).min(max_bytes_u64);
        let mut bytes = Vec::with_capacity(usize::try_from(readable).unwrap_or(max_bytes));
        if readable > 0 {
            let mut file = File::open(path)?;
            file.seek(SeekFrom::Start(start))?;
            file.take(readable).read_to_end(&mut bytes)?;
        }
        Ok(FileLogBatch {
            bytes,
            next_cursor: FileLogCursor {
                generation: index.generation,
                offset: start.saturating_add(readable),
            },
            gap,
        })
    }

    /// 把活动日志压缩为 gzip 归档并执行保留清理。
    fn rotate(&self, path: &Path) -> Result<(), FileLogError> {
        let archive = archive_path(path);
        let temporary = archive.with_extension("gz.tmp");
        let input = File::open(path)?;
        let output = File::create(&temporary)?;
        let mut encoder = GzEncoder::new(output, Compression::default());
        io::copy(&mut BufReader::new(input), &mut encoder)?;
        encoder.finish()?.sync_all()?;
        fs::rename(&temporary, &archive)?;
        fs::remove_file(path)?;
        self.remove_expired_archives(path)?;
        Ok(())
    }

    /// 删除超过保留数量的最旧 gzip 归档。
    fn remove_expired_archives(&self, active_path: &Path) -> Result<(), FileLogError> {
        let Some(parent) = active_path.parent() else {
            return Ok(());
        };
        let Some(active_name) = active_path.file_name().and_then(|name| name.to_str()) else {
            return Ok(());
        };
        let prefix = format!("{active_name}.");
        let mut archives = fs::read_dir(parent)?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with(&prefix))
                    && path
                        .extension()
                        .is_some_and(|extension| extension.eq_ignore_ascii_case("gz"))
            })
            .map(|path| {
                let modified = path
                    .metadata()
                    .and_then(|metadata| metadata.modified())
                    .unwrap_or(UNIX_EPOCH);
                let length = path.metadata().map_or(0, |metadata| metadata.len());
                (modified, length, path)
            })
            .collect::<Vec<_>>();
        archives.sort();
        let now = SystemTime::now();
        let mut total_bytes = archives.iter().map(|(_, length, _)| length).sum::<u64>();
        let mut remaining = archives.len();
        for (modified, length, path) in archives {
            let expired_by_age = self.policy.max_archive_age.is_some_and(|age| {
                now.duration_since(modified)
                    .is_ok_and(|elapsed| elapsed > age)
            });
            let expired_by_count = remaining > self.policy.max_archives;
            let expired_by_bytes = self
                .policy
                .max_archive_bytes
                .is_some_and(|limit| total_bytes > limit);
            if expired_by_age || expired_by_count || expired_by_bytes {
                fs::remove_file(path)?;
                total_bytes = total_bytes.saturating_sub(length);
                remaining = remaining.saturating_sub(1);
            }
        }
        Ok(())
    }
}

/// 返回活动日志对应的持久游标索引路径。
fn index_path(active_path: &Path) -> PathBuf {
    let name = active_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("procora.log");
    active_path.with_file_name(format!("{name}.cursor"))
}

/// 读取游标索引；旧日志没有索引时从现有长度建立第零代。
fn load_index(active_path: &Path, current_length: u64) -> Result<FileLogIndex, FileLogError> {
    let path = index_path(active_path);
    let content = match fs::read_to_string(&path) {
        Ok(content) => content,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(FileLogIndex {
                generation: 0,
                length: current_length,
            });
        }
        Err(error) => return Err(error.into()),
    };
    let mut fields = content.lines();
    let generation = fields.next().and_then(|value| value.parse().ok());
    let length = fields.next().and_then(|value| value.parse().ok());
    match (generation, length, fields.next()) {
        (Some(generation), Some(length), None) => Ok(FileLogIndex { generation, length }),
        _ => Err(FileLogError::InvalidIndex { path }),
    }
}

/// 保存活动日志的持久游标索引。
fn save_index(active_path: &Path, index: FileLogIndex) -> Result<(), FileLogError> {
    let path = index_path(active_path);
    fs::write(path, format!("{}\n{}\n", index.generation, index.length))?;
    Ok(())
}

/// 创建不会与同进程其他轮转冲突的 gzip 归档路径。
fn archive_path(active_path: &Path) -> PathBuf {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let sequence = ARCHIVE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let name = active_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("procora.log");
    active_path.with_file_name(format!("{name}.{timestamp}-{sequence}.gz"))
}

/// 删除一个活动日志对应的全部 gzip 轮转归档。
fn remove_archives(active_path: &Path) -> Result<(), FileLogError> {
    let Some(parent) = active_path.parent() else {
        return Ok(());
    };
    let Some(active_name) = active_path.file_name().and_then(|name| name.to_str()) else {
        return Ok(());
    };
    let prefix = format!("{active_name}.");
    let entries = match fs::read_dir(parent) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    for path in entries.filter_map(Result::ok).map(|entry| entry.path()) {
        let is_archive = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with(&prefix))
            && path
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("gz"));
        if is_archive {
            fs::remove_file(path)?;
        }
    }
    Ok(())
}

/// 把互斥锁损坏映射为稳定日志错误。
fn map_poisoned_lock<T>(_: PoisonError<T>) -> FileLogError {
    FileLogError::Poisoned
}
