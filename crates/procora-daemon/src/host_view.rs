use procora_engine::{DesiredState, ObservedState, TaskRuntimeState};
use procora_protocol::{ResourceUsageDto, TaskStatusDto};

/// 返回不经过 shell 的命令展示文本。
pub(crate) fn command_label(task: &procora_core::TaskSpec) -> String {
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

/// 返回失败、阻塞、退避和退出状态的可读说明。
pub(crate) fn task_message(state: TaskRuntimeState) -> Option<String> {
    match state.observed {
        ObservedState::Blocked => Some("等待依赖条件满足".to_owned()),
        ObservedState::Backoff => Some(format!("等待第 {} 次自动重启", state.restart_attempt)),
        ObservedState::Failed => Some(format!("Task 失败，退出码 {:?}", state.exit_code)),
        ObservedState::Exited => Some(format!("Task 已退出，退出码 {:?}", state.exit_code)),
        ObservedState::Orphaned => Some("无法验证遗留进程身份，未执行接管或终止".to_owned()),
        _ => None,
    }
}

/// 把监测快照压缩为协议可选资源值。
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub(crate) fn resource_usage(snapshot: procora_monitor::ResourceSnapshot) -> ResourceUsageDto {
    let cpu = (snapshot.cpu_percent.max(0.0) * 10.0).round();
    ResourceUsageDto {
        cpu_tenths_percent: Some(cpu.min(f32::from(u16::MAX)) as u16),
        memory_bytes: Some(snapshot.memory_bytes),
    }
}
