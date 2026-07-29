//! 全托管部署接收器的交互式集成测试辅助设施。

use std::{
    fs::{self, File},
    io::Write,
    path::Path,
    process::{Child, Command, ExitStatus, Output, Stdio},
    thread,
    time::{Duration, Instant},
};

const DEPLOY_COMMAND_TIMEOUT: Duration = Duration::from_secs(30);

/// 通过文件捕获协议输出，避免 Windows 后台 Center 继承管道后阻塞 EOF。
pub(crate) fn receive_deploy(home: &Path, archive: &[u8], header: &serde_json::Value) -> Output {
    let nonce = uuid::Uuid::new_v4();
    let stdout_path = home.join(format!(".receive-deploy-{nonce}.stdout"));
    let stderr_path = home.join(format!(".receive-deploy-{nonce}.stderr"));
    let mut child = Command::new(env!("CARGO_BIN_EXE_procora"))
        .arg("__receive-deploy")
        .env("PROCORA_HOME", home)
        .stdin(Stdio::piped())
        .stdout(Stdio::from(File::create(&stdout_path).unwrap()))
        .stderr(Stdio::from(File::create(&stderr_path).unwrap()))
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    writeln!(stdin, "{header}").unwrap();
    stdin.flush().unwrap();

    wait_until_ready(&mut child, &stdout_path, &stderr_path);
    stdin.write_all(archive).unwrap();
    drop(stdin);

    let status = wait_until_exit(&mut child, &stdout_path, &stderr_path);
    captured_output(status, &stdout_path, &stderr_path)
}

/// 等待接收器完成部署协商，异常退出时保留完整诊断。
fn wait_until_ready(child: &mut Child, stdout_path: &Path, stderr_path: &Path) {
    let deadline = Instant::now() + DEPLOY_COMMAND_TIMEOUT;
    loop {
        let stdout = read_capture(stdout_path);
        if String::from_utf8_lossy(&stdout).contains(r#""type":"ready""#) {
            return;
        }
        if let Some(status) = child.try_wait().unwrap() {
            panic_with_output(
                "部署接收器在 Ready 前退出",
                &captured_output(status, stdout_path, stderr_path),
            );
        }
        if Instant::now() >= deadline {
            terminate_and_panic(child, stdout_path, stderr_path, "等待 Ready 超过 30 秒");
        }
        thread::sleep(Duration::from_millis(20));
    }
}

/// 等待接收器进程退出，而不是等待可能被 Windows 后台进程持有的输出 EOF。
fn wait_until_exit(child: &mut Child, stdout_path: &Path, stderr_path: &Path) -> ExitStatus {
    let deadline = Instant::now() + DEPLOY_COMMAND_TIMEOUT;
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return status;
        }
        if Instant::now() >= deadline {
            terminate_and_panic(child, stdout_path, stderr_path, "部署接收器超过 30 秒");
        }
        thread::sleep(Duration::from_millis(20));
    }
}

/// 终止失去响应的接收器并输出已有协议与错误信息。
fn terminate_and_panic(
    child: &mut Child,
    stdout_path: &Path,
    stderr_path: &Path,
    message: &str,
) -> ! {
    let _ = child.kill();
    let status = child.wait().unwrap();
    panic_with_output(message, &captured_output(status, stdout_path, stderr_path))
}

/// 读取接收器当前已刷新的输出。
fn read_capture(path: &Path) -> Vec<u8> {
    fs::read(path).unwrap_or_default()
}

/// 构造与标准命令接口一致的捕获结果。
fn captured_output(status: ExitStatus, stdout_path: &Path, stderr_path: &Path) -> Output {
    let mut stderr = read_capture(stderr_path);
    if !status.success() {
        stderr.extend_from_slice(format!("\n部署接收器退出状态：{status}\n").as_bytes());
    }
    Output {
        status,
        stdout: read_capture(stdout_path),
        stderr,
    }
}

/// 使用统一格式报告子进程协议故障。
fn panic_with_output(message: &str, output: &Output) -> ! {
    panic!(
        "{message}\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    )
}
