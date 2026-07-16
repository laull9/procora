//! `SQLite` 服务状态与历史记录测试。

use std::{fs, path::PathBuf};

use procora::storage::{SqliteCenterRepository, StorageError, StoredService, StoredServiceStatus};
use rusqlite::Connection;
use uuid::Uuid;

/// 创建当前测试独占的 `SQLite` 状态目录。
fn temporary_directory() -> PathBuf {
    let directory = std::env::temp_dir().join(format!("procora-storage-{}", Uuid::new_v4()));
    fs::create_dir_all(&directory).unwrap();
    directory
}

/// 返回指定状态的测试服务记录。
fn service(status: StoredServiceStatus) -> StoredService {
    StoredService {
        name: "demo".to_owned(),
        root: PathBuf::from("/tmp/demo"),
        config_path: PathBuf::from("/tmp/demo/procora.yaml"),
        desired_running: status == StoredServiceStatus::Running,
        status,
        message: None,
        task_count: 2,
    }
}

#[test]
fn 保存后可以恢复完整服务状态() {
    let directory = temporary_directory();
    let repository = SqliteCenterRepository::new(directory.join("procora.sqlite3"));
    let expected = service(StoredServiceStatus::Running);

    repository.save_service(&expected).unwrap();

    assert_eq!(repository.load_services().unwrap(), vec![expected]);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn 只有状态或消息变化才追加历史() {
    let directory = temporary_directory();
    let repository = SqliteCenterRepository::new(directory.join("procora.sqlite3"));
    let running = service(StoredServiceStatus::Running);
    repository.save_service(&running).unwrap();
    repository.save_service(&running).unwrap();
    let mut failed = service(StoredServiceStatus::Failed);
    failed.message = Some("配置无效".to_owned());
    repository.save_service(&failed).unwrap();

    let history = repository.status_history("demo").unwrap();
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].status, StoredServiceStatus::Running);
    assert_eq!(history[1].status, StoredServiceStatus::Failed);
    assert_eq!(history[1].message.as_deref(), Some("配置无效"));
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn 删除服务会级联删除状态历史() {
    let directory = temporary_directory();
    let repository = SqliteCenterRepository::new(directory.join("procora.sqlite3"));
    repository
        .save_service(&service(StoredServiceStatus::Running))
        .unwrap();

    assert!(repository.remove_service("demo").unwrap());
    assert!(!repository.remove_service("demo").unwrap());
    assert!(repository.load_services().unwrap().is_empty());
    assert!(repository.status_history("demo").unwrap().is_empty());
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn 拒绝高于当前程序的数据库模式版本() {
    let directory = temporary_directory();
    let path = directory.join("procora.sqlite3");
    let connection = Connection::open(&path).unwrap();
    connection.pragma_update(None, "user_version", 99).unwrap();
    drop(connection);

    let repository = SqliteCenterRepository::new(path);
    assert!(matches!(
        repository.load_services(),
        Err(StorageError::UnsupportedSchema(99))
    ));
    fs::remove_dir_all(directory).unwrap();
}

/// Unix 路径的非 UTF-8 字节必须经过 `SQLite` 往返后保持不变。
#[cfg(unix)]
#[test]
fn unix非utf8路径可以无损恢复() {
    use std::os::unix::ffi::OsStringExt;

    let directory = temporary_directory();
    let repository = SqliteCenterRepository::new(directory.join("procora.sqlite3"));
    let mut expected = service(StoredServiceStatus::Running);
    expected.name = "binary-path".to_owned();
    expected.root = PathBuf::from(std::ffi::OsString::from_vec(vec![
        b'/', b't', b'm', b'p', b'/', 0xff,
    ]));
    expected.config_path = expected.root.join("procora.yaml");

    repository.save_service(&expected).unwrap();

    assert_eq!(repository.load_services().unwrap(), vec![expected]);
    fs::remove_dir_all(directory).unwrap();
}

/// Windows UTF-16 路径的非 ASCII 字符必须经过 SQLite 往返后保持不变。
#[cfg(windows)]
#[test]
fn windows宽字符路径可以无损恢复() {
    let directory = temporary_directory();
    let repository = SqliteCenterRepository::new(directory.join("procora.sqlite3"));
    let mut expected = service(StoredServiceStatus::Running);
    expected.name = "wide-path".to_owned();
    expected.root = PathBuf::from(r"C:\\Procora\\数据\\🦀");
    expected.config_path = expected.root.join("procora.yaml");

    repository.save_service(&expected).unwrap();

    assert_eq!(repository.load_services().unwrap(), vec![expected]);
    fs::remove_dir_all(directory).unwrap();
}
