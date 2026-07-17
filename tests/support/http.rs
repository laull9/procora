//! 不依赖公网的 HTTP 健康检查夹具。

use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

/// 本地 HTTP 检查夹具的响应模式。
pub enum HttpMode {
    /// 就绪文件出现且请求格式正确时返回 204。
    ReadyWhen(PathBuf),
    /// 接受连接后保持打开，直到夹具被释放。
    Hang,
}

/// 可在释放时确定停止的本地 HTTP 检查服务。
pub struct HttpFixture {
    address: SocketAddr,
    stop: Arc<AtomicBool>,
    accepted: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl HttpFixture {
    /// 创建按就绪文件响应或保持连接的本地服务。
    pub fn start(mode: HttpMode) -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("应能监听本地端口");
        listener.set_nonblocking(true).expect("应能启用非阻塞监听");
        let address = listener.local_addr().expect("应能读取监听地址");
        let stop = Arc::new(AtomicBool::new(false));
        let accepted = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let thread_accepted = Arc::clone(&accepted);
        let thread = thread::spawn(move || {
            while !thread_stop.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        thread_accepted.store(true, Ordering::Release);
                        handle_connection(&mut stream, &mode, &thread_stop);
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            address,
            stop,
            accepted,
            thread: Some(thread),
        }
    }

    /// 返回监听端口。
    pub fn port(&self) -> u16 {
        self.address.port()
    }

    /// 返回是否已经接受过检查连接。
    pub fn accepted(&self) -> bool {
        self.accepted.load(Ordering::Acquire)
    }
}

impl Drop for HttpFixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        let _ = TcpStream::connect(self.address);
        if self
            .thread
            .take()
            .is_some_and(|thread| thread.join().is_err())
        {
            panic!("HTTP 检查夹具线程异常退出");
        }
    }
}

/// 处理一次测试 HTTP 连接，不读取或发送无界内容。
fn handle_connection(stream: &mut TcpStream, mode: &HttpMode, stop: &AtomicBool) {
    if matches!(mode, HttpMode::Hang) {
        while !stop.load(Ordering::Acquire) {
            thread::sleep(Duration::from_millis(5));
        }
        return;
    }
    stream
        .set_read_timeout(Some(Duration::from_millis(500)))
        .expect("应能设置读取超时");
    let mut request = [0_u8; 4096];
    let count = stream.read(&mut request).unwrap_or(0);
    let request = String::from_utf8_lossy(&request[..count]).to_ascii_lowercase();
    let HttpMode::ReadyWhen(ready) = mode else {
        unreachable!("挂起模式已提前返回");
    };
    let healthy = ready.exists()
        && request.starts_with("get /ready http/1.1\r\n")
        && request.contains("\r\nx-probe: yes\r\n");
    let (status, reason) = if healthy {
        (204, "No Content")
    } else {
        (503, "Service Unavailable")
    };
    let response =
        format!("HTTP/1.1 {status} {reason}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    let _ = stream.write_all(response.as_bytes());
}
