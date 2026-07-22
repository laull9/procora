//! 文件日志并发吞吐、轮转和保留压力测试。

use std::{collections::BTreeSet, fs, str::FromStr, sync::Arc, thread};

use procora::{
    core::TaskId,
    log::{FileLogPolicy, FileLogStore},
};
use uuid::Uuid;

/// 创建当前测试独占的服务目录。
fn temporary_service(label: &str) -> std::path::PathBuf {
    let directory =
        std::env::temp_dir().join(format!("procora-log-stress-{label}-{}", Uuid::new_v4()));
    fs::create_dir_all(&directory).unwrap();
    directory
}

#[test]
// 多线程高速追加不会丢记录或产生撕裂写入。
fn concurrent_appends_do_not_lose_or_tear_records() {
    let service = temporary_service("concurrent");
    let store = Arc::new(
        FileLogStore::new(
            &service,
            FileLogPolicy {
                max_file_bytes: 16 * 1024 * 1024,
                max_archives: 2,
                max_archive_bytes: None,
                max_archive_age: None,
            },
        )
        .unwrap(),
    );
    let task = TaskId::from_str("noisy").unwrap();
    let workers = (0..8)
        .map(|worker| {
            let store = Arc::clone(&store);
            let task = task.clone();
            thread::spawn(move || {
                for sequence in 0..250 {
                    let record = format!("worker={worker:02};sequence={sequence:04}\n");
                    store.append_task(&task, record.as_bytes()).unwrap();
                }
            })
        })
        .collect::<Vec<_>>();
    for worker in workers {
        worker.join().unwrap();
    }

    let content = fs::read_to_string(store.task_log_path(&task)).unwrap();
    let records = content.lines().collect::<BTreeSet<_>>();
    assert_eq!(content.lines().count(), 2_000);
    assert_eq!(records.len(), 2_000, "每条固定身份日志必须恰好出现一次");
    fs::remove_dir_all(service).unwrap();
}

#[test]
// 密集轮转后归档数量有界且不遗留临时文件。
fn dense_rotation_bounds_archives_and_leaves_no_temporary_files() {
    let service = temporary_service("rotation");
    let store = FileLogStore::new(
        &service,
        FileLogPolicy {
            max_file_bytes: 4 * 1024,
            max_archives: 3,
            max_archive_bytes: Some(32 * 1024),
            max_archive_age: None,
        },
    )
    .unwrap();
    let payload = vec![b'x'; 1_024];
    for _ in 0..256 {
        store.append_service(&payload).unwrap();
    }

    let entries = fs::read_dir(service.join(".procora/logs"))
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    let archives = entries
        .iter()
        .filter(|path| path.extension().is_some_and(|extension| extension == "gz"))
        .count();
    assert!(archives <= 3);
    assert!(
        entries
            .iter()
            .all(|path| path.extension().is_none_or(|extension| extension != "tmp"))
    );
    assert!(fs::metadata(store.service_log_path()).unwrap().len() <= 4 * 1024);
    fs::remove_dir_all(service).unwrap();
}
