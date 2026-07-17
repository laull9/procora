//! 自动重启策略的规范化与次数控制。

use std::collections::BTreeMap;

use crate::core::{ProjectSpec, RestartPolicy, TaskId};

use super::{ObservedState, TaskRuntimeState};

/// 单个 Task 的规范化自动重启参数。
#[derive(Clone, Copy, Debug)]
pub(super) struct RestartConfig {
    /// 退出后是否需要自动重启。
    pub(super) policy: RestartPolicy,
    /// 首次重启前的基础退避毫秒数。
    pub(super) delay_ms: u64,
    /// 连续退出允许的最大自动重启次数，零表示不限制。
    pub(super) max_restarts: u32,
    /// 运行达到该时长后重置连续重启计数，零表示从不重置。
    pub(super) reset_after_ms: u64,
}

/// 从项目规范提取不改变进程身份的自动重启参数。
pub(super) fn restart_configs(spec: &ProjectSpec) -> BTreeMap<TaskId, RestartConfig> {
    spec.tasks
        .iter()
        .map(|(task_id, task)| {
            (
                task_id.clone(),
                RestartConfig {
                    policy: task.restart,
                    delay_ms: task.restart_delay_ms,
                    max_restarts: task.max_restarts,
                    reset_after_ms: task.restart_reset_after_ms,
                },
            )
        })
        .collect()
}

/// 判断当前退出状态是否符合自动重启策略。
pub(super) const fn restart_wanted(policy: RestartPolicy, observed: ObservedState) -> bool {
    matches!(policy, RestartPolicy::Always)
        || (matches!(policy, RestartPolicy::OnFailure) && matches!(observed, ObservedState::Failed))
}

/// 在次数仍可用时进入下一次退避，否则标记重启耗尽。
pub(super) fn schedule_restart(state: &mut TaskRuntimeState, max_restarts: u32) {
    if max_restarts == 0 || state.restart_attempt < max_restarts {
        state.restart_attempt = state.restart_attempt.saturating_add(1);
        state.restart_exhausted = false;
        state.observed = ObservedState::Backoff;
    } else {
        state.restart_exhausted = true;
    }
}

/// 计算以 30 秒为上限的指数重启退避。
pub(super) fn restart_delay(base_ms: u64, attempt: u32) -> u64 {
    let exponent = attempt.saturating_sub(1).min(10);
    base_ms.saturating_mul(1_u64 << exponent).min(30_000)
}
