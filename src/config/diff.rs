use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::core::{ProjectSpec, TaskId, TaskSpec};

/// 两个有效项目配置之间可供人和引擎消费的语义差异。
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProjectDiff {
    /// 仅存在于候选修订中的 Task。
    pub added: Vec<TaskId>,
    /// 仅存在于当前修订中的 Task。
    pub removed: Vec<TaskId>,
    /// 进程身份或依赖关系变化、应用时必须重启的 Task。
    pub restart: Vec<TaskId>,
    /// 只有运行策略变化、未来可在原地更新的 Task。
    pub update_in_place: Vec<TaskId>,
    /// 语义没有变化的 Task。
    pub unchanged: Vec<TaskId>,
}

impl ProjectDiff {
    /// 判断候选修订是否没有任何运行时语义变化。
    pub fn is_empty(&self) -> bool {
        self.added.is_empty()
            && self.removed.is_empty()
            && self.restart.is_empty()
            && self.update_in_place.is_empty()
    }
}

/// 比较当前与候选配置，并保守传播需要重启的下游 Task。
pub fn diff_projects(current: &ProjectSpec, candidate: &ProjectSpec) -> ProjectDiff {
    let current_ids = current.tasks.keys().cloned().collect::<BTreeSet<_>>();
    let candidate_ids = candidate.tasks.keys().cloned().collect::<BTreeSet<_>>();
    let added = candidate_ids.difference(&current_ids).cloned().collect();
    let removed = current_ids.difference(&candidate_ids).cloned().collect();
    let mut restart = BTreeSet::new();
    let mut update_in_place = BTreeSet::new();

    for task_id in current_ids.intersection(&candidate_ids) {
        let before = &current.tasks[task_id];
        let after = &candidate.tasks[task_id];
        if process_identity_changed(before, after) {
            restart.insert(task_id.clone());
        } else if before != after {
            update_in_place.insert(task_id.clone());
        }
    }
    propagate_dependents(current, &mut restart);
    propagate_dependents(candidate, &mut restart);
    update_in_place.retain(|task_id| !restart.contains(task_id));
    let unchanged = current_ids
        .intersection(&candidate_ids)
        .filter(|task_id| !restart.contains(*task_id) && !update_in_place.contains(*task_id))
        .cloned()
        .collect();

    ProjectDiff {
        added,
        removed,
        restart: restart.into_iter().collect(),
        update_in_place: update_in_place.into_iter().collect(),
        unchanged,
    }
}

/// 判断变化是否会改变真实进程身份或调度依赖。
fn process_identity_changed(before: &TaskSpec, after: &TaskSpec) -> bool {
    before.command != after.command
        || before.args != after.args
        || before.cwd != after.cwd
        || before.env != after.env
        || before.healthcheck != after.healthcheck
        || before.depends_on != after.depends_on
}

/// 把身份变化沿依赖图传播到所有仍存在的下游 Task。
fn propagate_dependents(spec: &ProjectSpec, affected: &mut BTreeSet<TaskId>) {
    loop {
        let next = spec
            .tasks
            .iter()
            .filter(|(task_id, task)| {
                !affected.contains(*task_id)
                    && task
                        .depends_on
                        .keys()
                        .any(|dependency| affected.contains(dependency))
            })
            .map(|(task_id, _)| task_id.clone())
            .collect::<Vec<_>>();
        if next.is_empty() {
            break;
        }
        affected.extend(next);
    }
}
