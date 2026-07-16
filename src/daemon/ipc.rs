use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

#[cfg(windows)]
use std::sync::OnceLock;

use crate::protocol::{CenterHello, CenterRequest, CenterResponse, ClientHello, PROTOCOL_VERSION};
use crate::storage::SqliteCenterRepository;
use fs2::{FileExt, lock_contended_error};
use interprocess::local_socket::{
    GenericNamespaced, ListenerNonblockingMode, ListenerOptions, Stream, prelude::*,
};
use thiserror::Error;

use super::{Center, CenterError};

/// 中心服务器本地 IPC 客户端或服务端错误。
#[derive(Debug, Error)]
pub enum IpcError {
    /// 本地套接字连接、监听或传输失败。
    #[error("中心服务器 IPC 失败: {0}")]
    Io(#[from] std::io::Error),
    /// JSON 行协议编解码失败。
    #[error("中心服务器协议编解码失败: {0}")]
    Json(#[from] serde_json::Error),
    /// 中心服务器注册表无法恢复。
    #[error(transparent)]
    Center(#[from] CenterError),
    /// 服务端连接线程无法访问中心状态。
    #[error("中心服务器状态锁已损坏")]
    Poisoned,
    /// 同一状态目录已经存在运行中的中心服务器。
    #[error("当前用户的 Procora 中心服务器已经运行")]
    AlreadyRunning,
    /// 本地连接不属于运行 Center 的当前用户。
    #[error("拒绝非当前用户访问 Procora 中心服务器")]
    Unauthorized,
    /// 单条协议帧超过服务端允许的上限。
    #[error("中心服务器协议帧超过 {0} 字节上限")]
    FrameTooLarge(usize),
}

/// 单条 IPC 请求 JSON 帧允许占用的最大字节数。
const MAX_REQUEST_FRAME_BYTES: usize = 1024 * 1024;

/// 单条 IPC 响应 JSON 帧允许占用的最大字节数。
const MAX_RESPONSE_FRAME_BYTES: usize = 8 * 1024 * 1024;

/// 单个连接读写一帧的最长等待时间。
const CONNECTION_TIMEOUT: Duration = Duration::from_secs(2);

/// Center 自主推进 Task 状态机的轮询间隔。
const CENTER_TICK_INTERVAL: Duration = Duration::from_millis(20);

/// 同时处理的本地 IPC 连接线程上限。
const MAX_CONNECTIONS: usize = 64;

/// Windows 客户端共享的异步运行时，避免每次 IPC 请求重复创建线程池。
#[cfg(windows)]
static WINDOWS_IPC_RUNTIME: OnceLock<Result<tokio::runtime::Runtime, String>> = OnceLock::new();

/// 通过本地套接字发送单次请求的中心服务器客户端。
#[derive(Clone, Debug)]
pub struct CenterClient {
    endpoint: String,
}

impl CenterClient {
    /// 创建连接指定本地端点的客户端。
    pub fn new(endpoint: impl Into<String>) -> Self {
        Self {
            endpoint: endpoint.into(),
        }
    }

    /// 发送请求并读取对应的单行响应。
    ///
    /// # Errors
    ///
    /// 当中心服务器不存在、传输失败或响应无法解码时返回错误。
    pub fn request(&self, request: &CenterRequest) -> Result<CenterResponse, IpcError> {
        #[cfg(windows)]
        return request_windows(&self.endpoint, request);
        #[cfg(not(windows))]
        {
            let name = self.endpoint.clone().to_ns_name::<GenericNamespaced>()?;
            let mut connection = Stream::connect(name)?;
            configure_client_io(&connection)?;
            write_json_line(&mut connection, request)?;
            let response = read_limited_line(&mut connection, MAX_RESPONSE_FRAME_BYTES)?;
            Ok(serde_json::from_str(&response)?)
        }
    }

    /// 探测端点是否存在可用的 Procora 中心服务器。
    pub fn ping(&self) -> bool {
        matches!(self.request(&CenterRequest::Ping), Ok(CenterResponse::Pong))
    }

    /// 执行协议握手并返回中心身份和控制能力。
    ///
    /// # Errors
    ///
    /// 当中心不可用、协议版本不兼容或响应类型错误时返回错误。
    pub fn hello(&self, client_name: &str) -> Result<CenterHello, IpcError> {
        let response = self.request(&CenterRequest::Hello(ClientHello {
            protocol_version: PROTOCOL_VERSION,
            client_name: client_name.to_owned(),
        }))?;
        match response {
            CenterResponse::Hello(hello) => Ok(hello),
            CenterResponse::Error { message } => Err(IpcError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                message,
            ))),
            response => Err(IpcError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("中心服务器返回了意外握手响应: {response:?}"),
            ))),
        }
    }
}

/// 在当前线程运行阻塞的中心服务器本地 IPC 循环。
///
/// # Errors
///
/// 当注册表无法恢复或本地端点无法监听时返回错误。
pub fn run_center_server(endpoint: &str, database_path: &Path) -> Result<(), IpcError> {
    if let Some(parent) = database_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let lock_path = database_path.with_extension("lock");
    let lock_file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_path)?;
    if let Err(error) = lock_file.try_lock_exclusive() {
        let contended = lock_contended_error();
        if error.kind() == std::io::ErrorKind::WouldBlock
            || error.raw_os_error() == contended.raw_os_error()
        {
            return Err(IpcError::AlreadyRunning);
        }
        return Err(error.into());
    }
    let center = Center::load(SqliteCenterRepository::new(database_path))?;
    let center = Arc::new(Mutex::new(center));
    let stopping = Arc::new(AtomicBool::new(false));
    let name = endpoint.to_ns_name::<GenericNamespaced>()?;
    let options = ListenerOptions::new()
        .name(name)
        .try_overwrite(true)
        .nonblocking(ListenerNonblockingMode::Accept);
    #[cfg(windows)]
    let options = restrict_windows_pipe(options)?;
    let listener = options.create_sync()?;
    let mut workers = Vec::new();
    let mut last_tick = Instant::now();

    while !stopping.load(Ordering::Acquire) {
        reap_finished_workers(&mut workers);
        if last_tick.elapsed() >= CENTER_TICK_INTERVAL {
            center.lock().map_err(|_| IpcError::Poisoned)?.tick();
            last_tick = Instant::now();
        }
        match listener.accept() {
            Ok(connection) if workers.len() < MAX_CONNECTIONS => {
                let center = Arc::clone(&center);
                let stopping = Arc::clone(&stopping);
                workers.push(thread::spawn(move || {
                    if let Err(error) = handle_connection(connection, &center, &stopping) {
                        tracing::warn!(%error, "中心服务器请求处理失败");
                    }
                }));
            }
            Ok(_) => {
                tracing::warn!(limit = MAX_CONNECTIONS, "中心服务器并发连接已达到上限");
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(5));
            }
            Err(error) => tracing::warn!(%error, "中心服务器接受本地连接失败，继续监听"),
        }
    }
    for worker in workers {
        let _ = worker.join();
    }
    drop(lock_file);
    Ok(())
}

/// 处理一条连接中的单次 JSON 行请求。
fn handle_connection(
    connection: Stream,
    center: &Mutex<Center>,
    stopping: &AtomicBool,
) -> Result<(), IpcError> {
    #[cfg(unix)]
    authorize_peer(&connection)?;
    #[cfg(not(windows))]
    configure_server_io(&connection)?;
    let mut connection = connection;
    let request = read_limited_line(&mut connection, MAX_REQUEST_FRAME_BYTES)?;
    let request: CenterRequest = serde_json::from_str(&request)?;
    let should_stop = matches!(request, CenterRequest::Shutdown);
    if should_stop {
        stopping.store(true, Ordering::Release);
    }
    let response = {
        let mut center = center.lock().map_err(|_| IpcError::Poisoned)?;
        if stopping.load(Ordering::Acquire) && !should_stop {
            CenterResponse::Error {
                message: "中心服务器正在关闭".to_owned(),
            }
        } else {
            center.handle(request)
        }
    };
    write_json_line(&mut connection, &response)?;
    Ok(())
}

/// Unix 客户端使用本地套接字原生读写超时。
#[cfg(not(windows))]
fn configure_client_io(connection: &Stream) -> Result<(), IpcError> {
    connection.set_recv_timeout(Some(CONNECTION_TIMEOUT))?;
    connection.set_send_timeout(Some(CONNECTION_TIMEOUT))?;
    Ok(())
}

/// 服务端在 Unix 使用本地套接字原生读写超时。
#[cfg(not(windows))]
fn configure_server_io(connection: &Stream) -> Result<(), IpcError> {
    connection.set_recv_timeout(Some(CONNECTION_TIMEOUT))?;
    connection.set_send_timeout(Some(CONNECTION_TIMEOUT))?;
    Ok(())
}

/// 把一个 JSON 值编码成换行结尾的协议帧并完整写出。
fn write_json_line<T: serde::Serialize + ?Sized>(
    connection: &mut Stream,
    value: &T,
) -> Result<(), IpcError> {
    let mut frame = serde_json::to_vec(value)?;
    frame.push(b'\n');
    let deadline = Instant::now() + CONNECTION_TIMEOUT;
    let mut written = 0;
    while written < frame.len() {
        match connection.write(&frame[written..]) {
            Ok(0) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::WriteZero,
                    "中心服务器 IPC 未能写入完整协议帧",
                )
                .into());
            }
            Ok(count) => written += count,
            Err(error) if retry_io(&error, deadline) => {}
            Err(error) => return Err(normalize_timeout(error, deadline).into()),
        }
    }
    connection.flush()?;
    Ok(())
}

/// 读取一条带换行结束符且大小受限的 UTF-8 JSON 帧。
fn read_limited_line(connection: &mut Stream, max_bytes: usize) -> Result<String, IpcError> {
    let deadline = Instant::now() + CONNECTION_TIMEOUT;
    let mut response = Vec::new();
    let mut buffer = [0_u8; 8192];
    while response.len() <= max_bytes {
        let remaining = max_bytes + 1 - response.len();
        let capacity = remaining.min(buffer.len());
        match connection.read(&mut buffer[..capacity]) {
            Ok(0) => break,
            Ok(count) => {
                let bytes = &buffer[..count];
                if let Some(end) = bytes.iter().position(|byte| *byte == b'\n') {
                    response.extend_from_slice(&bytes[..=end]);
                    break;
                }
                response.extend_from_slice(bytes);
            }
            Err(error) if retry_io(&error, deadline) => {}
            Err(error) => return Err(normalize_timeout(error, deadline).into()),
        }
    }
    if response.len() > max_bytes || !response.ends_with(b"\n") {
        return Err(IpcError::FrameTooLarge(max_bytes));
    }
    String::from_utf8(response)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error).into())
}

/// 对非阻塞或原生超时错误执行短暂退避，并指示调用方是否应重试。
fn retry_io(error: &std::io::Error, deadline: Instant) -> bool {
    if is_transient_io(error) && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(5));
        true
    } else {
        false
    }
}

/// 把到达应用层期限的瞬时错误统一转换为明确的超时错误。
fn normalize_timeout(error: std::io::Error, deadline: Instant) -> std::io::Error {
    if is_transient_io(&error) && Instant::now() >= deadline {
        std::io::Error::new(std::io::ErrorKind::TimedOut, "中心服务器 IPC 读写超时")
    } else {
        error
    }
}

/// 判断错误是否表示当前传输暂时没有可读写数据。
fn is_transient_io(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
    )
}

/// Windows 使用异步命名管道执行带整体期限的单次请求。
#[cfg(windows)]
fn request_windows(endpoint: &str, request: &CenterRequest) -> Result<CenterResponse, IpcError> {
    use interprocess::{ConnectWaitMode, local_socket::ConnectOptions};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let runtime = WINDOWS_IPC_RUNTIME
        .get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .enable_all()
                .build()
                .map_err(|error| error.to_string())
        })
        .as_ref()
        .map_err(|message| {
            IpcError::Io(std::io::Error::other(format!(
                "无法创建 Windows IPC 运行时: {message}"
            )))
        })?;
    let name = endpoint.to_owned().to_ns_name::<GenericNamespaced>()?;
    runtime.block_on(async {
        tokio::time::timeout(CONNECTION_TIMEOUT, async {
            let options = ConnectOptions::new()
                .name(name)
                .wait_mode(ConnectWaitMode::Timeout(CONNECTION_TIMEOUT));
            let mut connection = options.connect_tokio().await?;
            let mut frame = serde_json::to_vec(request)?;
            frame.push(b'\n');
            connection.write_all(&frame).await?;
            connection.flush().await?;

            let mut response = Vec::new();
            let mut buffer = [0_u8; 8192];
            while response.len() <= MAX_RESPONSE_FRAME_BYTES {
                let remaining = MAX_RESPONSE_FRAME_BYTES + 1 - response.len();
                let capacity = remaining.min(buffer.len());
                let count = connection.read(&mut buffer[..capacity]).await?;
                if count == 0 {
                    break;
                }
                let bytes = &buffer[..count];
                if let Some(end) = bytes.iter().position(|byte| *byte == b'\n') {
                    response.extend_from_slice(&bytes[..=end]);
                    break;
                }
                response.extend_from_slice(bytes);
            }
            if response.len() > MAX_RESPONSE_FRAME_BYTES || !response.ends_with(b"\n") {
                return Err(IpcError::FrameTooLarge(MAX_RESPONSE_FRAME_BYTES));
            }
            let response = String::from_utf8(response)
                .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
            Ok(serde_json::from_str(&response)?)
        })
        .await
        .map_err(|_| {
            IpcError::Io(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "中心服务器 IPC 读写超时",
            ))
        })?
    })
}

/// 回收已经结束的连接线程，避免长期运行时积累句柄。
fn reap_finished_workers(workers: &mut Vec<JoinHandle<()>>) {
    let mut index = 0;
    while index < workers.len() {
        if workers[index].is_finished() {
            let worker = workers.swap_remove(index);
            let _ = worker.join();
        } else {
            index += 1;
        }
    }
}

/// 校验 Unix 本地连接与 Center 进程属于同一有效用户。
#[cfg(unix)]
fn authorize_peer(connection: &Stream) -> Result<(), IpcError> {
    let peer = connection.peer_creds()?.euid();
    if peer == Some(rustix::process::geteuid().as_raw()) {
        Ok(())
    } else {
        Err(IpcError::Unauthorized)
    }
}

/// 为 Windows 命名管道设置仅所有者、系统和管理员可访问的 DACL。
#[cfg(windows)]
fn restrict_windows_pipe(options: ListenerOptions<'_>) -> Result<ListenerOptions<'_>, IpcError> {
    use interprocess::os::windows::{
        local_socket::ListenerOptionsExt, security_descriptor::SecurityDescriptor,
    };
    use widestring::U16CString;

    const CURRENT_USER_DACL: &str = "D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GA;;;OW)";
    let sddl = U16CString::from_str(CURRENT_USER_DACL).map_err(|error| {
        IpcError::Io(std::io::Error::new(std::io::ErrorKind::InvalidInput, error))
    })?;
    let descriptor = SecurityDescriptor::deserialize(&sddl)?;
    Ok(options.security_descriptor(descriptor))
}
