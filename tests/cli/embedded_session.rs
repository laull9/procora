//! 临时 TUI 会话的状态、控制与日志回归测试。

use std::{
    fs,
    path::PathBuf,
    str::FromStr,
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use procora::cli::session::EmbeddedTuiSession;
use procora::config::{ConfigFormat, load_str};
use procora::core::TaskId;
use procora::daemon::ServiceHost;
use procora::protocol::{ServiceActionDto, SnapshotSourceDto, TaskStatusDto};
use procora::tui::LiveSession;

/// 创建当前测试独占的临时服务目录。
fn temporary_service() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory = std::env::temp_dir().join(format!(
        "procora-embedded-session-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(&directory).unwrap();
    directory
}

#[test]
// 临时会话可以刷新状态控制服务并读取日志。
fn embedded_session_refreshes_controls_and_reads_logs() {
    let service = temporary_service();
    let compiled = load_str(
        "version: 1\nproject: embedded\ntasks:\n  app:\n    command: rustc\n    args: ['--version']\n",
        ConfigFormat::Yaml,
    )
    .unwrap();
    let mut host = ServiceHost::from_compiled_at(compiled, &service);
    host.start().unwrap();
    let mut session = EmbeddedTuiSession::new(&mut host);
    let deadline = Instant::now() + Duration::from_secs(3);

    loop {
        let snapshot = session.poll_snapshot().unwrap().unwrap();
        assert_eq!(snapshot.source, SnapshotSourceDto::EmbeddedLive);
        if snapshot.tasks[0].status == TaskStatusDto::Stopped {
            break;
        }
        assert!(Instant::now() < deadline, "临时任务未在期限内结束");
        thread::sleep(Duration::from_millis(10));
    }

    let task_id = TaskId::from_str("app").unwrap();
    let update = session.poll_log(&task_id).unwrap().unwrap();
    assert!(String::from_utf8_lossy(&update.bytes).starts_with("rustc "));

    let stopped = session.manage(ServiceActionDto::Stop).unwrap();
    assert_eq!(stopped.tasks[0].status, TaskStatusDto::Stopped);
    drop(session);
    drop(host);
    fs::remove_dir_all(service).unwrap();
}
