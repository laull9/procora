use std::collections::BTreeSet;

use crate::{config::CompiledProject, core::TaskId};

use super::{ServiceHost, ServiceHostError};

impl ServiceHost {
    /// 不重建运行进程地提交退出码、重启边界和停止宽限策略。
    ///
    /// # Errors
    ///
    /// 当放宽重启上限后恢复调度失败时返回错误。
    ///
    /// # Panics
    ///
    /// 仅当调用方把 Task 集合变化错误分类为原地更新时 panic。
    pub fn update_runtime_policies(
        &mut self,
        compiled: CompiledProject,
    ) -> Result<(), ServiceHostError> {
        let effects = self.engine.update_runtime_policies(&compiled.spec);
        for (task_id, candidate) in compiled.spec.tasks {
            let current = self
                .spec
                .tasks
                .get_mut(&task_id)
                .expect("Task 集合没有变化");
            current.success_exit_codes = candidate.success_exit_codes;
            current.restart = candidate.restart;
            current.restart_delay_ms = candidate.restart_delay_ms;
            current.max_restarts = candidate.max_restarts;
            current.restart_reset_after_ms = candidate.restart_reset_after_ms;
            current.shutdown_timeout_ms = candidate.shutdown_timeout_ms;
        }
        self.execute_effects(effects)
    }

    /// 提交候选图，只重启语义差异影响集合，并在失败时恢复旧图。
    ///
    /// # Errors
    ///
    /// 当受影响进程无法停止、候选无法启动或旧图无法恢复时返回错误。
    pub fn reconfigure(
        &mut self,
        compiled: CompiledProject,
        affected: &BTreeSet<TaskId>,
        desired_running: bool,
    ) -> Result<(), ServiceHostError> {
        let old_spec = self.spec.clone();
        let old_dependencies = self.dependencies.clone();
        let old_graph = self.engine.graph().clone();
        self.pending_spawns
            .retain(|pending| !affected.contains(&pending.task_id));

        let stop_effects = self.engine.prepare_reconfigure(affected);
        if let Err(apply) = self.execute_effects(stop_effects) {
            let rollback = self.restore_configuration(
                old_spec,
                old_graph,
                old_dependencies,
                affected,
                desired_running,
            );
            return match rollback {
                Ok(()) => Err(apply),
                Err(error) => Err(ServiceHostError::ReconfigureRollback {
                    apply: apply.to_string(),
                    rollback: error.to_string(),
                }),
            };
        }

        self.spec = compiled.spec;
        self.dependencies = compiled.dependencies;
        let effects =
            self.engine
                .reconfigure(&self.spec, compiled.graph, affected, desired_running);
        if let Err(apply) = self.execute_effects(effects) {
            let cleanup = self.engine.prepare_reconfigure(affected);
            let _ = self.execute_effects(cleanup);
            let rollback = self.restore_configuration(
                old_spec,
                old_graph,
                old_dependencies,
                affected,
                desired_running,
            );
            return match rollback {
                Ok(()) => Err(apply),
                Err(error) => Err(ServiceHostError::ReconfigureRollback {
                    apply: apply.to_string(),
                    rollback: error.to_string(),
                }),
            };
        }
        Ok(())
    }

    /// 把引擎和宿主规范恢复为应用前版本并重新对账受影响 Task。
    fn restore_configuration(
        &mut self,
        spec: crate::core::ProjectSpec,
        graph: crate::core::TaskGraph,
        dependencies: crate::config::ManagedDependencies,
        affected: &BTreeSet<TaskId>,
        desired_running: bool,
    ) -> Result<(), ServiceHostError> {
        self.spec = spec;
        self.dependencies = dependencies;
        let effects = self
            .engine
            .reconfigure(&self.spec, graph, affected, desired_running);
        self.execute_effects(effects)
    }
}
