//! 服务宿主的退出、重启与健康状态刷新。

use crate::{
    engine::{HealthState, RuntimeEvent},
    protocol::TaskDiagnosticKindDto,
};

use super::{ServiceHost, diagnostics, elapsed_millis, exit_succeeded};
use crate::daemon::host_logs::join_readers;

impl ServiceHost {
    /// 轮询退出事件与重启退避，返回 Task 状态是否发生变化。
    ///
    /// # Panics
    ///
    /// 仅当内部 Task 索引在一次同步轮询中违反一致性不变量时 panic。
    pub fn refresh(&mut self) -> bool {
        let mut changed = self.start_due_spawns();
        changed |= self.refresh_processes();
        changed |= self.refresh_health();
        if changed {
            self.flush_logs();
        }
        changed
    }

    /// 轮询进程退出与查询错误，并继续执行重启副作用。
    fn refresh_processes(&mut self) -> bool {
        let mut changed = false;
        let task_ids = self.instances.keys().cloned().collect::<Vec<_>>();
        for task_id in task_ids {
            let result = self
                .instances
                .get_mut(&task_id)
                .expect("Task 实例仍存在")
                .child
                .try_wait();
            match result {
                Ok(Some(status)) => {
                    let mut instance = self.instances.remove(&task_id).expect("实例仍存在");
                    let identity = instance.identity;
                    self.resource_cache.invalidate();
                    self.health.stop(&task_id, identity);
                    if let Err(error) = instance.child.cleanup_after_exit() {
                        let (message, suggestion) = diagnostics::process_io_error(
                            "顶层进程退出后清理剩余进程树失败",
                            &error,
                        );
                        self.record_task_diagnostic(
                            &task_id,
                            Some(identity),
                            TaskDiagnosticKindDto::Process,
                            message,
                            suggestion,
                        );
                        tracing::warn!(task = %task_id, %error, "顶层进程退出后清理剩余进程树失败");
                    }
                    join_readers(instance.readers);
                    let exit_code = status.code();
                    let success = exit_succeeded(&self.spec.tasks[&task_id], status);
                    if !success {
                        let message = exit_code.map_or_else(
                            || "Task 异常退出，未获得退出码".to_owned(),
                            |code| format!("Task 异常退出，退出码 {code}"),
                        );
                        self.record_task_diagnostic(
                            &task_id,
                            Some(identity),
                            TaskDiagnosticKindDto::Exit,
                            message,
                            Some("查看本条诊断之前的 stderr/stdout，并检查退出码含义".to_owned()),
                        );
                    }
                    let effects = self.engine.event(RuntimeEvent::Exited {
                        task_id: task_id.clone(),
                        identity,
                        exit_code,
                        success,
                        run_duration_ms: elapsed_millis(instance.started_at),
                    });
                    if self
                        .engine
                        .state(&task_id)
                        .is_some_and(|state| state.restart_exhausted)
                    {
                        let max_restarts = self.spec.tasks[&task_id].max_restarts;
                        self.record_task_diagnostic(
                            &task_id,
                            Some(identity),
                            TaskDiagnosticKindDto::Restart,
                            format!("已达到 {max_restarts} 次自动重启上限"),
                            Some("修复根因后手动重启，或检查 max_restarts 配置".to_owned()),
                        );
                    }
                    if let Err(error) = self.execute_effects(effects) {
                        tracing::warn!(%error, "Task 退出后的调度失败");
                    }
                    changed = true;
                }
                Ok(None) => {}
                Err(error) => {
                    let identity = self
                        .instances
                        .get(&task_id)
                        .map(|instance| instance.identity);
                    let (message, suggestion) =
                        diagnostics::process_io_error("Task 退出状态查询失败", &error);
                    self.record_task_diagnostic(
                        &task_id,
                        identity,
                        TaskDiagnosticKindDto::Process,
                        message,
                        suggestion,
                    );
                    tracing::warn!(task = %task_id, %error, "Task 退出状态查询失败");
                }
            }
        }
        changed
    }

    /// 消费健康检查变化并把不健康详情写入诊断。
    fn refresh_health(&mut self) -> bool {
        let health_events = self.health.refresh();
        let changed = !health_events.is_empty();
        for event in health_events {
            if let RuntimeEvent::HealthChanged {
                task_id,
                identity,
                health: HealthState::Unhealthy,
                detail,
            } = &event
            {
                let detail = detail.as_deref().unwrap_or("健康检查未提供错误详情");
                self.record_task_diagnostic(
                    task_id,
                    Some(*identity),
                    TaskDiagnosticKindDto::Health,
                    format!("健康检查达到失败阈值：{detail}"),
                    Some("检查 healthcheck 配置、目标依赖及 Task 就绪状态".to_owned()),
                );
            }
            let effects = self.engine.event(event);
            if let Err(error) = self.execute_effects(effects) {
                tracing::warn!(%error, "健康状态变化后的调度失败");
            }
        }
        changed
    }
}
