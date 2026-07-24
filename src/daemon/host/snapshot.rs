//! 服务宿主状态与资源的协议快照投影。

use crate::protocol::{ProjectSnapshot, SnapshotSourceDto, TaskView};

use super::ServiceHost;
use crate::daemon::host_view::{
    command_label, resource_usage, task_health, task_message, task_status,
};

impl ServiceHost {
    /// 把实时进程、资源与引擎状态投影为 TUI 一致性快照。
    pub fn snapshot(&mut self, source: SnapshotSourceDto, running: bool) -> ProjectSnapshot {
        self.refresh();
        let roots = self
            .instances
            .values()
            .map(|instance| instance.pid)
            .collect();
        let resources = self.resource_cache.snapshots(&mut self.monitor, roots);
        let diagnostics = self
            .diagnostics
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let tasks =
            self.engine
                .states()
                .map(|(task_id, state)| {
                    let task = &self.spec.tasks[task_id];
                    let resources = self.instances.get(task_id).and_then(|instance| {
                        resources.get(&instance.pid).copied().map(resource_usage)
                    });
                    let diagnostics = diagnostics.task(task_id);
                    let message = diagnostics
                        .last()
                        .map(|diagnostic| diagnostic.message.clone())
                        .or_else(|| task_message(*state));
                    TaskView {
                        task_id: task_id.clone(),
                        command: command_label(task),
                        status: task_status(*state, running),
                        health: task_health(state.health),
                        dependencies: task.depends_on.keys().cloned().collect(),
                        resources,
                        message,
                        diagnostics,
                    }
                })
                .collect();
        ProjectSnapshot {
            project: self.spec.project.clone(),
            source,
            tasks,
        }
    }
}
