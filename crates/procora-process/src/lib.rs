//! 跨平台受管进程的创建、输出管道、等待和整树回收。

use std::{
    io,
    process::{ChildStderr, ChildStdout, ExitStatus, Stdio},
    thread,
    time::{Duration, Instant},
};

#[cfg(windows)]
use process_wrap::std::JobObject;
#[cfg(unix)]
use process_wrap::std::ProcessGroup;
use process_wrap::std::{ChildWrapper, CommandWrap};
use procora_core::TaskSpec;

/// 一次停止操作的退出状态与是否强制回收。
#[derive(Debug)]
pub struct StopOutcome {
    /// 顶层进程的最终退出状态。
    pub status: ExitStatus,
    /// 是否因超过宽限期而使用强制终止。
    pub forced: bool,
}

/// 使用平台进程组或 Job Object 包装的任务子进程。
#[derive(Debug)]
pub struct ManagedChild {
    inner: Box<dyn ChildWrapper>,
}

impl Drop for ManagedChild {
    fn drop(&mut self) {
        let _ = self.kill_remaining_tree();
    }
}

impl ManagedChild {
    /// 返回顶层任务进程标识。
    pub fn id(&self) -> u32 {
        self.inner.id()
    }

    /// 取出子进程标准输出管道，最多只能成功一次。
    pub fn take_stdout(&mut self) -> Option<ChildStdout> {
        self.inner.stdout().take()
    }

    /// 取出子进程标准错误管道，最多只能成功一次。
    pub fn take_stderr(&mut self) -> Option<ChildStderr> {
        self.inner.stderr().take()
    }

    /// 非阻塞检查受管进程是否已经退出。
    ///
    /// # Errors
    ///
    /// 当平台进程状态查询失败时返回错误。
    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        self.inner.try_wait()
    }

    /// 等待受管进程退出。
    ///
    /// # Errors
    ///
    /// 当底层进程等待操作失败时返回 I/O 错误。
    pub fn wait(&mut self) -> io::Result<ExitStatus> {
        self.inner.wait()
    }

    /// 先请求优雅退出，超过宽限期后强制回收整个进程树。
    ///
    /// Unix 向进程组发送 SIGTERM；Windows Job Object 暂无等价的通用控制台信号，
    /// 因而直接进入整树终止并在结果中标记为强制。
    ///
    /// # Errors
    ///
    /// 当信号发送、状态查询或强制终止失败时返回错误。
    pub fn stop(&mut self, timeout: Duration) -> io::Result<StopOutcome> {
        if let Some(status) = self.inner.try_wait()? {
            return Ok(StopOutcome {
                status,
                forced: self.kill_remaining_tree()?,
            });
        }
        #[cfg(unix)]
        {
            const SIGTERM: i32 = 15;
            self.inner.signal(SIGTERM)?;
            let deadline = Instant::now() + timeout;
            loop {
                if let Some(status) = self.inner.try_wait()? {
                    return Ok(StopOutcome {
                        status,
                        forced: self.kill_remaining_tree()?,
                    });
                }
                if Instant::now() >= deadline {
                    break;
                }
                thread::sleep(Duration::from_millis(10));
            }
        }
        self.inner.start_kill()?;
        let status = self.inner.wait()?;
        Ok(StopOutcome {
            status,
            forced: true,
        })
    }

    /// 立即强制回收受管进程组或 Job Object。
    ///
    /// # Errors
    ///
    /// 当平台终止或进程等待操作失败时返回 I/O 错误。
    pub fn kill(&mut self) -> io::Result<ExitStatus> {
        self.inner.kill()?;
        self.inner.wait()
    }

    /// 顶层进程已经退出后，仍强制清理可能存活的进程组或 Job Object 成员。
    ///
    /// # Errors
    ///
    /// 当平台无法向剩余进程树发送终止请求时返回 I/O 错误。
    pub fn cleanup_after_exit(&mut self) -> io::Result<bool> {
        self.kill_remaining_tree()
    }

    /// 尝试强制终止仍属于当前托管容器的全部进程。
    fn kill_remaining_tree(&mut self) -> io::Result<bool> {
        match self.inner.start_kill() {
            Ok(()) => {
                let _ = self.inner.wait()?;
                Ok(true)
            }
            Err(error) if process_tree_is_gone(&error) => {
                let _ = self.inner.wait();
                Ok(false)
            }
            Err(error) => Err(error),
        }
    }
}

/// 判断终止请求是否仅表示目标进程组已经不存在。
fn process_tree_is_gone(error: &io::Error) -> bool {
    if matches!(
        error.kind(),
        io::ErrorKind::NotFound | io::ErrorKind::InvalidInput
    ) {
        return true;
    }
    #[cfg(unix)]
    {
        const ESRCH: i32 = 3;
        error.raw_os_error() == Some(ESRCH)
    }
    #[cfg(not(unix))]
    false
}

/// 根据任务规范启动隔离的受管子进程。
///
/// # Errors
///
/// 当程序不存在、参数无效或平台进程组初始化失败时返回 I/O 错误。
pub fn spawn_task(task: &TaskSpec) -> io::Result<ManagedChild> {
    let mut command = CommandWrap::with_new(&task.command, |command| {
        command
            .args(&task.args)
            .envs(&task.env)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(cwd) = &task.cwd {
            command.current_dir(cwd);
        }
    });
    #[cfg(unix)]
    command.wrap(ProcessGroup::leader());
    #[cfg(windows)]
    command.wrap(JobObject);
    command.spawn().map(|inner| ManagedChild { inner })
}
