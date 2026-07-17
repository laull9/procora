use std::str::FromStr;

use procora::core::TaskId;
use procora::protocol::{
    ProjectSnapshot, SnapshotSourceDto, TaskHealthDto, TaskStatusDto, TaskView,
};

#[allow(dead_code)]
pub mod http;

/// 创建包含阻塞任务的 TUI 测试快照。
#[allow(dead_code)]
pub fn snapshot() -> ProjectSnapshot {
    let database = TaskId::from_str("database").unwrap();
    let api = TaskId::from_str("api").unwrap();
    ProjectSnapshot {
        project: "demo".to_owned(),
        source: SnapshotSourceDto::ConfigPreview,
        tasks: vec![
            TaskView {
                task_id: database.clone(),
                command: "postgres".to_owned(),
                status: TaskStatusDto::Pending,
                health: TaskHealthDto::NotConfigured,
                dependencies: Vec::new(),
                resources: None,
                message: None,
            },
            TaskView {
                task_id: api,
                command: "cargo run -p api".to_owned(),
                status: TaskStatusDto::Blocked,
                health: TaskHealthDto::NotConfigured,
                dependencies: vec![database],
                resources: None,
                message: Some("等待 database 启动".to_owned()),
            },
        ],
    }
}
