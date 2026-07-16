//! 后台 CLI 集成测试辅助设施。

use std::{
    fs::{self, File},
    path::Path,
    process::{Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};

/// 执行可能启动后台进程的命令，并通过文件隔离 Windows 可继承输出句柄。
pub(crate) fn run_background_cli(
    command: &mut Command,
    output_directory: &Path,
    label: &str,
) -> Output {
    let stdout_path = output_directory.join(format!(".{label}.stdout"));
    let stderr_path = output_directory.join(format!(".{label}.stderr"));
    let mut child = command
        .stdout(Stdio::from(File::create(&stdout_path).unwrap()))
        .stderr(Stdio::from(File::create(&stderr_path).unwrap()))
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return Output {
                status,
                stdout: fs::read(&stdout_path).unwrap(),
                stderr: fs::read(&stderr_path).unwrap(),
            };
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            let status = child.wait().unwrap();
            let output = Output {
                status,
                stdout: fs::read(&stdout_path).unwrap(),
                stderr: fs::read(&stderr_path).unwrap(),
            };
            panic!(
                "测试命令超过 15 秒\nstdout:\n{}\nstderr:\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr),
            );
        }
        thread::sleep(Duration::from_millis(20));
    }
}

/// 在后台进程释放 Windows 文件句柄后删除测试目录。
pub(crate) fn remove_directory_when_released(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match fs::remove_dir_all(path) {
            Ok(()) => return,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
            Err(error)
                if error.kind() == std::io::ErrorKind::PermissionDenied
                    && Instant::now() < deadline =>
            {
                thread::sleep(Duration::from_millis(20));
            }
            Err(error) => panic!("无法清理测试目录 {}: {error}", path.display()),
        }
    }
}
