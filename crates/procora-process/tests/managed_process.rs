//! 真实子进程输出、退出与整树停止契约测试。

use std::{
    collections::BTreeMap,
    io::Read,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use procora_core::{RestartPolicy, TaskSpec};
use procora_process::spawn_task;

/// 创建指定命令的最小 Task 规范。
fn task(command: &str, args: &[&str]) -> TaskSpec {
    TaskSpec {
        command: command.to_owned(),
        args: args.iter().map(|value| (*value).to_owned()).collect(),
        cwd: None,
        env: BTreeMap::new(),
        depends_on: BTreeMap::new(),
        restart: RestartPolicy::Never,
        restart_delay_ms: 10,
        shutdown_timeout_ms: 500,
    }
}

#[test]
fn 真实子进程输出可以排空并等待退出() {
    let mut child = spawn_task(&task("rustc", &["--version"])).unwrap();
    let mut stdout = child.take_stdout().unwrap();
    let mut content = String::new();
    stdout.read_to_string(&mut content).unwrap();
    let status = child.wait().unwrap();

    assert!(status.success());
    assert!(content.starts_with("rustc "));
}

#[cfg(unix)]
#[test]
fn unix可以在宽限期内回收整个进程组() {
    let mut child = spawn_task(&task("sh", &["-c", "trap '' TERM; sleep 30 & wait"])).unwrap();
    let started = Instant::now();

    let outcome = child.stop(Duration::from_millis(50)).unwrap();

    assert!(!outcome.status.success() || outcome.forced);
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[cfg(unix)]
#[test]
fn unix顶层进程提前退出后仍回收后台后代() {
    let mut child = spawn_task(&task(
        "sh",
        &["-c", "sleep 30 </dev/null >/dev/null 2>&1 & echo $!"],
    ))
    .unwrap();
    let mut stdout = child.take_stdout().unwrap();
    let mut content = String::new();
    stdout.read_to_string(&mut content).unwrap();
    assert!(child.wait().unwrap().success());
    let descendant = content.trim().to_owned();

    child.cleanup_after_exit().unwrap();

    let deadline = Instant::now() + Duration::from_secs(2);
    while Command::new("kill")
        .args(["-0", &descendant])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
    {
        assert!(Instant::now() < deadline, "后台后代没有被进程组回收");
        thread::sleep(Duration::from_millis(10));
    }
}
