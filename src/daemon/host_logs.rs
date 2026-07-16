use std::{
    io::Read,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender, TrySendError},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use crate::core::TaskId;
use crate::engine::TaskRunIdentity;
use crate::log::{FileLogStore, LogFrame, LogStream, TailBuffer};

/// 单条发送给异步磁盘写入器的受限日志消息。
#[derive(Debug)]
pub(crate) enum LogWrite {
    /// 追加一批 Task 原始输出。
    Append { task_id: TaskId, bytes: Vec<u8> },
    /// 确认此前已入队内容完成落盘。
    Flush(mpsc::Sender<()>),
    /// 请求写入线程完成当前队列后退出。
    Shutdown,
}

/// 独立于进程管道读取器的有界磁盘日志写入器。
#[derive(Debug)]
pub(crate) struct LogWriter {
    sender: SyncSender<LogWrite>,
    handle: Option<JoinHandle<()>>,
    done: Receiver<()>,
}

/// 可在限定时间内等待结束的单个输出读取线程。
#[derive(Debug)]
pub(crate) struct OutputReader {
    handle: Option<JoinHandle<()>>,
    done: Receiver<()>,
}

/// 创建一条输出读取线程所需的共享运行上下文。
pub(crate) struct LogReaderContext {
    /// 日志所属 Task。
    pub(crate) task_id: TaskId,
    /// 日志所属运行身份。
    pub(crate) identity: TaskRunIdentity,
    /// stdout/stderr 共享的运行内序号。
    pub(crate) sequence: Arc<AtomicU64>,
    /// 有界内存尾部缓冲。
    pub(crate) tail: Arc<Mutex<TailBuffer>>,
    /// 可选的有界磁盘日志队列。
    pub(crate) disk: Option<SyncSender<LogWrite>>,
    /// 因磁盘队列拥塞而丢弃的分片计数。
    pub(crate) dropped_chunks: Arc<AtomicU64>,
}

/// 每个 Service 最多缓存的待落盘日志消息数量。
const LOG_QUEUE_CAPACITY: usize = 1024;

/// 进程退出后等待继承管道关闭的最长时间。
const READER_DRAIN_TIMEOUT: Duration = Duration::from_secs(1);

/// 等待已入队日志完成落盘的最长时间。
const LOG_FLUSH_TIMEOUT: Duration = Duration::from_secs(1);

impl LogWriter {
    /// 创建单服务独占的有界日志写入线程。
    pub(crate) fn new(files: Arc<FileLogStore>) -> Self {
        let (sender, receiver) = mpsc::sync_channel(LOG_QUEUE_CAPACITY);
        let (done_sender, done) = mpsc::channel();
        let handle = thread::spawn(move || {
            while let Ok(message) = receiver.recv() {
                match message {
                    LogWrite::Append { task_id, bytes } => {
                        if let Err(error) = files.append_task(&task_id, &bytes) {
                            tracing::warn!(task = %task_id, %error, "Task 文件日志写入失败");
                        }
                    }
                    LogWrite::Flush(acknowledge) => {
                        let _ = acknowledge.send(());
                    }
                    LogWrite::Shutdown => break,
                }
            }
            let _ = done_sender.send(());
        });
        Self {
            sender,
            handle: Some(handle),
            done,
        }
    }

    /// 返回供输出读取器使用的有界发送端。
    pub(crate) fn sender(&self) -> SyncSender<LogWrite> {
        self.sender.clone()
    }

    /// 在限定时间内等待此前成功入队的日志完成写入。
    pub(crate) fn flush(&self) {
        let (sender, receiver) = mpsc::channel();
        match self.sender.try_send(LogWrite::Flush(sender)) {
            Ok(()) => {
                if receiver.recv_timeout(LOG_FLUSH_TIMEOUT).is_err() {
                    tracing::warn!("等待 Task 日志落盘超时，继续处理控制事件");
                }
            }
            Err(TrySendError::Full(_)) => {
                tracing::warn!("Task 日志队列已满，无法等待完整落盘");
            }
            Err(TrySendError::Disconnected(_)) => {
                tracing::warn!("Task 日志写入线程已经退出");
            }
        }
    }

    /// 请求写入线程退出，但不因慢磁盘无限阻塞宿主销毁。
    pub(crate) fn shutdown(&mut self) {
        if self.sender.try_send(LogWrite::Shutdown).is_ok()
            && self.done.recv_timeout(LOG_FLUSH_TIMEOUT).is_ok()
            && let Some(handle) = self.handle.take()
        {
            let _ = handle.join();
        }
    }
}

/// 在独立线程持续排空一个输出流并写入内存和有界磁盘队列。
pub(crate) fn spawn_log_reader<R: Read + Send + 'static>(
    mut reader: R,
    stream: LogStream,
    context: LogReaderContext,
) -> OutputReader {
    let (done_sender, done) = mpsc::channel();
    let handle = thread::spawn(move || {
        let mut buffer = vec![0; 8192];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(length) => {
                    let bytes = buffer[..length].to_vec();
                    if let Ok(mut tail) = context.tail.lock() {
                        tail.push(LogFrame {
                            task_id: context.task_id.clone(),
                            run_id: context.identity.run_id,
                            sequence: context.sequence.fetch_add(1, Ordering::Relaxed),
                            stream,
                            bytes: bytes.clone(),
                        });
                    }
                    enqueue_disk_log(
                        context.disk.as_ref(),
                        &context.task_id,
                        bytes,
                        &context.dropped_chunks,
                    );
                }
                Err(error) => {
                    tracing::warn!(task = %context.task_id, %error, "Task 输出读取失败");
                    break;
                }
            }
        }
        let _ = done_sender.send(());
    });
    OutputReader {
        handle: Some(handle),
        done,
    }
}

/// 非阻塞写入磁盘队列，拥塞时优先保证进程管道持续排空。
fn enqueue_disk_log(
    disk: Option<&SyncSender<LogWrite>>,
    task_id: &TaskId,
    bytes: Vec<u8>,
    dropped_chunks: &AtomicU64,
) {
    let Some(disk) = disk else {
        return;
    };
    if let Err(error) = disk.try_send(LogWrite::Append {
        task_id: task_id.clone(),
        bytes,
    }) {
        match error {
            TrySendError::Full(_) => {
                let dropped = dropped_chunks.fetch_add(1, Ordering::Relaxed) + 1;
                if dropped == 1 || dropped.is_power_of_two() {
                    tracing::warn!(task = %task_id, dropped, "Task 日志磁盘队列拥塞，已丢弃待落盘分片");
                }
            }
            TrySendError::Disconnected(_) => {
                tracing::warn!(task = %task_id, "Task 日志写入线程已经退出");
            }
        }
    }
}

/// 在限定时间内等待输出线程排空，超时线程会分离而不阻塞 Center。
pub(crate) fn join_readers(mut readers: Vec<OutputReader>) {
    let deadline = Instant::now() + READER_DRAIN_TIMEOUT;
    for reader in &mut readers {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if reader.done.recv_timeout(remaining).is_ok()
            && let Some(handle) = reader.handle.take()
        {
            let _ = handle.join();
        } else {
            tracing::warn!("等待 Task 输出管道关闭超时，已分离读取线程");
        }
    }
}
