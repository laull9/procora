use crate::engine::{DesiredState, HealthState, ObservedState, TaskRuntimeState};
use crate::protocol::{ResourceUsageDto, TaskHealthDto, TaskStatusDto};

/// 返回不经过 shell 的命令展示文本。
pub(crate) fn command_label(task: &crate::core::TaskSpec) -> String {
    std::iter::once(task.command.as_str())
        .chain(task.args.iter().map(String::as_str))
        .collect::<Vec<_>>()
        .join(" ")
}

/// 把引擎状态映射为稳定 TUI 状态。
pub(crate) fn task_status(state: TaskRuntimeState, service_running: bool) -> TaskStatusDto {
    if !service_running || state.desired == DesiredState::Stopped {
        return TaskStatusDto::Stopped;
    }
    match state.observed {
        ObservedState::Pending | ObservedState::Starting | ObservedState::Backoff => {
            TaskStatusDto::Pending
        }
        ObservedState::Blocked => TaskStatusDto::Blocked,
        ObservedState::Running => TaskStatusDto::Running,
        ObservedState::Stopping | ObservedState::Exited => TaskStatusDto::Stopped,
        ObservedState::Failed | ObservedState::Orphaned => TaskStatusDto::Failed,
    }
}

/// 把引擎健康状态映射为稳定协议值。
pub(crate) const fn task_health(health: HealthState) -> TaskHealthDto {
    match health {
        HealthState::Unknown => TaskHealthDto::Unknown,
        HealthState::Starting => TaskHealthDto::Starting,
        HealthState::Healthy => TaskHealthDto::Healthy,
        HealthState::Unhealthy => TaskHealthDto::Unhealthy,
        HealthState::NotConfigured => TaskHealthDto::NotConfigured,
    }
}

/// 返回失败、阻塞、退避和退出状态的可读说明。
pub(crate) fn task_message(state: TaskRuntimeState) -> Option<String> {
    match state.observed {
        ObservedState::Blocked => Some("等待依赖条件满足".to_owned()),
        ObservedState::Backoff => Some(format!("等待第 {} 次自动重启", state.restart_attempt)),
        ObservedState::Failed | ObservedState::Exited if state.restart_exhausted => Some(format!(
            "{}；已达到 {} 次自动重启上限",
            exit_message("Task 退出", state.exit_code),
            state.restart_attempt
        )),
        ObservedState::Failed => Some(exit_message("Task 失败", state.exit_code)),
        ObservedState::Exited => Some(exit_message("Task 已退出", state.exit_code)),
        ObservedState::Orphaned => Some("无法验证遗留进程身份，未执行接管或终止".to_owned()),
        _ => None,
    }
}

/// 把可选退出码转换为不泄漏 `Option` 调试格式的说明。
fn exit_message(prefix: &str, exit_code: Option<i32>) -> String {
    exit_code.map_or_else(
        || format!("{prefix}，未获得退出码"),
        |exit_code| format!("{prefix}，退出码 {exit_code}"),
    )
}

/// 把监测快照压缩为协议可选资源值。
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub(crate) fn resource_usage(snapshot: crate::monitor::ResourceSnapshot) -> ResourceUsageDto {
    let cpu = (snapshot.cpu_percent.max(0.0) * 10.0).round();
    ResourceUsageDto {
        cpu_tenths_percent: Some(cpu.min(f32::from(u16::MAX)) as u16),
        memory_bytes: Some(snapshot.memory_bytes),
    }
}
