//! 真实子进程输出、退出与整树停止契约测试。

use std::{
    collections::{BTreeMap, BTreeSet},
    io::Read,
    thread,
    time::{Duration, Instant},
};

#[cfg(unix)]
use std::process::{Command, Stdio};

use procora::core::{RestartPolicy, TaskSpec};
use procora::process::spawn_task;

/// 创建指定命令的最小 Task 规范。
fn task(command: &str, args: &[&str]) -> TaskSpec {
    TaskSpec {
        command: command.to_owned(),
        args: args.iter().map(|value| (*value).to_owned()).collect(),
        cwd: None,
        env: BTreeMap::new(),
        healthcheck: None,
        success_exit_codes: BTreeSet::from([0]),
        depends_on: BTreeMap::new(),
        restart: RestartPolicy::Never,
        restart_delay_ms: 10,
        max_restarts: 0,
        restart_reset_after_ms: 60_000,
        shutdown_timeout_ms: 500,
    }
}

#[test]
// 真实子进程输出可以排空并等待退出。
fn real_child_output_drains_before_exit() {
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
// unix可以在宽限期内回收整个进程组。
fn unix_reclaims_process_group_within_grace_period() {
    let mut child = spawn_task(&task("sh", &["-c", "trap '' TERM; sleep 30 & wait"])).unwrap();
    let started = Instant::now();

    let outcome = child.stop(Duration::from_millis(50)).unwrap();

    assert!(!outcome.status.success() || outcome.forced);
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[cfg(unix)]
#[test]
// unix顶层进程提前退出后仍回收后台后代。
fn unix_reclaims_descendants_after_root_exits() {
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

/// Windows Job Object 必须能回收一个仍在执行的控制台命令。
#[cfg(windows)]
#[test]
// windows可以强制回收受管进程树。
fn windows_force_reclaims_managed_process_tree() {
    let mut child = spawn_task(&task("cmd.exe", &["/C", "ping -n 30 127.0.0.1 > NUL"])).unwrap();
    let started = Instant::now();

    let outcome = child.stop(Duration::from_millis(50)).unwrap();

    assert!(outcome.forced);
    assert!(started.elapsed() < Duration::from_secs(2));
}

/// Windows 轮询顶层退出后仍必须在期限内完成 Job Object 清理。
#[cfg(windows)]
#[test]
// windows轮询退出后清理不会等待已消费的job事件。
fn windows_cleanup_does_not_wait_for_consumed_job_event() {
    let mut child = spawn_task(&task("cmd.exe", &["/C", "exit", "0"])).unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while child.try_wait().unwrap().is_none() {
        assert!(Instant::now() < deadline, "顶层进程没有在期限内退出");
        thread::sleep(Duration::from_millis(1));
    }

    let (sender, receiver) = std::sync::mpsc::channel();
    thread::spawn(move || sender.send(child.cleanup_after_exit()).unwrap());
    let result = receiver
        .recv_timeout(Duration::from_secs(2))
        .expect("Job Object 清理不应永久等待完成端口事件");

    result.unwrap();
}
