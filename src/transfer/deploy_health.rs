//! 全托管部署的确定性任务验活门控。

use std::{
    thread,
    time::{Duration, Instant},
};

use anyhow::bail;

use crate::protocol::{TaskHealthDto, TaskStatusDto, TaskView};

/// 轮询任务状态，要求所有任务持续处于确定性可用状态。
pub(super) fn wait_until_accepted(
    project: &str,
    timeout_ms: u64,
    stable_for_ms: u64,
) -> anyhow::Result<()> {
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let stable_for = Duration::from_millis(stable_for_ms);
    let mut ready_since = None;
    loop {
        let snapshot = crate::cli::api::service_snapshot(project)?;
        let wait_message = match deployment_readiness(&snapshot.tasks) {
            Readiness::Ready => {
                let since = *ready_since.get_or_insert_with(Instant::now);
                if since.elapsed() >= stable_for {
                    return Ok(());
                }
                format!(
                    "正在通过 {} 稳定窗口",
                    crate::config::format_duration(stable_for_ms)
                )
            }
            Readiness::Waiting(message) => {
                ready_since = None;
                message
            }
            Readiness::Failed(message) => bail!(message),
        };
        if Instant::now() >= deadline {
            bail!(
                "Service `{project}` 未在 {} 内通过部署验收：{wait_message}",
                crate::config::format_duration(timeout_ms),
            );
        }
        thread::sleep(Duration::from_millis(100));
    }
}

/// 一次快照相对部署门控的判定。
fn deployment_readiness(tasks: &[TaskView]) -> Readiness {
    for task in tasks {
        if task.status == TaskStatusDto::Failed {
            return Readiness::Failed(task_detail(task, "Task 运行失败"));
        }
        if task.health == TaskHealthDto::Unhealthy {
            return Readiness::Failed(task_detail(task, "健康检查失败"));
        }
    }
    for task in tasks {
        match (task.status, task.health) {
            (TaskStatusDto::Running, TaskHealthDto::Healthy | TaskHealthDto::NotConfigured) => {}
            (TaskStatusDto::Running, _) => {
                return Readiness::Waiting(format!("Task `{}` 正在等待健康检查", task.task_id));
            }
            (status, _) => {
                return Readiness::Waiting(format!(
                    "Task `{}` 尚未运行（{status:?}）",
                    task.task_id
                ));
            }
        }
    }
    Readiness::Ready
}

/// 附带综合诊断生成部署失败说明。
fn task_detail(task: &TaskView, prefix: &str) -> String {
    let detail = task
        .diagnostics
        .last()
        .map(|diagnostic| diagnostic.message.as_str())
        .or(task.message.as_deref())
        .unwrap_or("无更多诊断");
    format!("{prefix}：`{}`：{detail}", task.task_id)
}

/// 部署门控的一次确定性判断。
enum Readiness {
    Ready,
    Waiting(String),
    Failed(String),
}
