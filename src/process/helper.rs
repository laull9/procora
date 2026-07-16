//! 内部辅助命令的有界输出、超时等待与整树回收。

use std::{
    io::{self, Read},
    thread,
    time::{Duration, Instant},
};

use thiserror::Error;

use crate::core::TaskSpec;

use super::spawn_isolated_task;

/// 辅助命令 stdout 与 stderr 的完整有界结果。
#[derive(Debug)]
pub(crate) struct BoundedCommandOutput {
    pub(crate) status: std::process::ExitStatus,
    pub(crate) stdout: Vec<u8>,
    pub(crate) stderr: Vec<u8>,
}

/// 辅助命令启动、等待、输出或资源边界错误。
#[derive(Debug, Error)]
pub(crate) enum BoundedCommandError {
    /// 无法启动受管辅助进程。
    #[error("无法启动辅助进程：{0}")]
    Spawn(io::Error),
    /// 查询辅助进程状态失败。
    #[error("等待辅助进程失败：{0}")]
    Wait(io::Error),
    /// 辅助进程超过调用方给定期限。
    #[error("执行超过 {0} 毫秒")]
    Timeout(u128),
    /// stdout 超过调用方给定字节上限。
    #[error("stdout 超过 {0} 字节")]
    StdoutLimit(usize),
    /// stderr 超过调用方给定字节上限。
    #[error("stderr 超过 {0} 字节")]
    StderrLimit(usize),
    /// 输出读取线程或底层管道失败。
    #[error("读取 {stream} 失败：{message}")]
    Output {
        stream: &'static str,
        message: String,
    },
    /// 调用方的周期资源检查拒绝继续执行。
    #[error("资源监测拒绝继续执行：{0}")]
    Monitor(String),
}

/// 有界读取线程的完整结果。
struct LimitedOutput {
    bytes: Vec<u8>,
    exceeded: bool,
}

/// 运行清空继承环境的内部辅助命令并限制时间和两个输出流。
pub(crate) fn run_bounded_command(
    task: &TaskSpec,
    timeout: Duration,
    stdout_limit: usize,
    stderr_limit: usize,
) -> Result<BoundedCommandOutput, BoundedCommandError> {
    run_bounded_command_monitored(task, timeout, stdout_limit, stderr_limit, || Ok(()))
}

/// 运行辅助命令，并在等待期间周期执行调用方资源检查。
pub(crate) fn run_bounded_command_monitored(
    task: &TaskSpec,
    timeout: Duration,
    stdout_limit: usize,
    stderr_limit: usize,
    mut monitor: impl FnMut() -> Result<(), String>,
) -> Result<BoundedCommandOutput, BoundedCommandError> {
    let mut child = spawn_isolated_task(task).map_err(BoundedCommandError::Spawn)?;
    let stdout = child.take_stdout().expect("辅助命令 stdout 已配置为管道");
    let stderr = child.take_stderr().expect("辅助命令 stderr 已配置为管道");
    let stdout_reader = spawn_limited_reader(stdout, stdout_limit);
    let stderr_reader = spawn_limited_reader(stderr, stderr_limit);
    let deadline = Instant::now() + timeout;
    let status = loop {
        if let Err(message) = monitor() {
            let _ = child.kill();
            break Err(BoundedCommandError::Monitor(message));
        }
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            Ok(None) => {
                let _ = child.kill();
                break Err(BoundedCommandError::Timeout(timeout.as_millis()));
            }
            Err(error) => {
                let _ = child.kill();
                break Err(BoundedCommandError::Wait(error));
            }
        }
    };
    if status.is_ok() {
        let _ = child.cleanup_after_exit();
    }
    let stdout = join_reader(stdout_reader, "stdout")?;
    let stderr = join_reader(stderr_reader, "stderr")?;
    let status = status?;
    if stdout.exceeded {
        return Err(BoundedCommandError::StdoutLimit(stdout_limit));
    }
    if stderr.exceeded {
        return Err(BoundedCommandError::StderrLimit(stderr_limit));
    }
    Ok(BoundedCommandOutput {
        status,
        stdout: stdout.bytes,
        stderr: stderr.bytes,
    })
}

/// 在独立线程持续排空管道，同时只保留上限内字节。
fn spawn_limited_reader<R>(
    mut reader: R,
    limit: usize,
) -> thread::JoinHandle<io::Result<LimitedOutput>>
where
    R: Read + Send + 'static,
{
    thread::spawn(move || {
        let mut bytes = Vec::with_capacity(limit.min(64 * 1024));
        let mut buffer = [0_u8; 8192];
        let mut exceeded = false;
        loop {
            let count = reader.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            let remaining = limit.saturating_sub(bytes.len());
            bytes.extend_from_slice(&buffer[..count.min(remaining)]);
            exceeded |= count > remaining;
        }
        Ok(LimitedOutput { bytes, exceeded })
    })
}

/// 汇合输出线程并把 panic/I/O 映射为稳定诊断。
fn join_reader(
    reader: thread::JoinHandle<io::Result<LimitedOutput>>,
    stream: &'static str,
) -> Result<LimitedOutput, BoundedCommandError> {
    reader
        .join()
        .map_err(|_| BoundedCommandError::Output {
            stream,
            message: "读取线程异常退出".to_owned(),
        })?
        .map_err(|error| BoundedCommandError::Output {
            stream,
            message: error.to_string(),
        })
}
