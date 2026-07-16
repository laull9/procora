//! Service 与 Task 文件日志轮转压缩测试。

use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    str::FromStr,
};

use flate2::read::GzDecoder;
use procora::core::TaskId;
use procora::log::{FileLogCursor, FileLogPolicy, FileLogStore};
use uuid::Uuid;

/// 创建当前测试独占的服务目录。
fn temporary_service() -> PathBuf {
    let directory = std::env::temp_dir().join(format!("procora-log-{}", Uuid::new_v4()));
    fs::create_dir_all(&directory).unwrap();
    directory
}

/// 返回目录中唯一 gzip 文件解压后的内容。
fn read_single_archive(directory: &Path) -> Vec<u8> {
    let archive = fs::read_dir(directory)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| path.extension().is_some_and(|extension| extension == "gz"))
        .unwrap();
    let mut content = Vec::new();
    GzDecoder::new(fs::File::open(archive).unwrap())
        .read_to_end(&mut content)
        .unwrap();
    content
}

#[test]
fn 服务日志保存在服务目录并自动压缩() {
    let service = temporary_service();
    let store = FileLogStore::new(
        &service,
        FileLogPolicy {
            max_file_bytes: 5,
            max_archives: 2,
            ..FileLogPolicy::default()
        },
    )
    .unwrap();
    store.append_service(b"1234").unwrap();
    store.append_service(b"56").unwrap();

    assert_eq!(fs::read(store.service_log_path()).unwrap(), b"56");
    assert_eq!(read_single_archive(&service.join(".procora/logs")), b"1234");
    fs::remove_dir_all(service).unwrap();
}

#[test]
fn task日志独立轮转并执行归档保留策略() {
    let service = temporary_service();
    let store = FileLogStore::new(
        &service,
        FileLogPolicy {
            max_file_bytes: 3,
            max_archives: 1,
            ..FileLogPolicy::default()
        },
    )
    .unwrap();
    let task = TaskId::from_str("api").unwrap();
    store.append_task(&task, b"123").unwrap();
    store.append_task(&task, b"456").unwrap();
    store.append_task(&task, b"789").unwrap();

    let task_directory = service.join(".procora/logs/tasks");
    let archives = fs::read_dir(task_directory)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "gz")
        })
        .count();
    assert_eq!(archives, 1);
    assert_eq!(fs::read(store.task_log_path(&task)).unwrap(), b"789");
    fs::remove_dir_all(service).unwrap();
}

#[test]
fn 文件游标可以续读并在轮转后报告gap() {
    let service = temporary_service();
    let store = FileLogStore::new(
        &service,
        FileLogPolicy {
            max_file_bytes: 5,
            max_archives: 1,
            ..FileLogPolicy::default()
        },
    )
    .unwrap();
    store.append_service(b"1234").unwrap();
    let first = store.read_service(None, 2).unwrap();
    assert_eq!(first.bytes, b"12");
    let second = store.read_service(Some(first.next_cursor), 8).unwrap();
    assert_eq!(second.bytes, b"34");
    assert!(!second.gap);

    store.append_service(b"678").unwrap();
    let recovered = store.read_service(Some(second.next_cursor), 8).unwrap();
    assert_eq!(recovered.bytes, b"678");
    assert!(recovered.gap);
    assert_eq!(
        recovered.next_cursor,
        FileLogCursor {
            generation: 1,
            offset: 3
        }
    );
    fs::remove_dir_all(service).unwrap();
}

#[test]
fn gzip归档总字节上限可以独立触发清理() {
    let service = temporary_service();
    let store = FileLogStore::new(
        &service,
        FileLogPolicy {
            max_file_bytes: 3,
            max_archives: 10,
            max_archive_bytes: Some(0),
            max_archive_age: None,
        },
    )
    .unwrap();
    store.append_service(b"123").unwrap();
    store.append_service(b"456").unwrap();

    let archive_count = fs::read_dir(service.join(".procora/logs"))
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().is_some_and(|value| value == "gz"))
        .count();
    assert_eq!(archive_count, 0);
    fs::remove_dir_all(service).unwrap();
}
