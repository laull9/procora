use std::collections::{BTreeMap, BTreeSet};

use crate::core::{DependencyCondition, ProjectSpec, TaskGraph, TaskId};
use uuid::Uuid;

use super::restart::{
    RestartConfig, restart_configs, restart_delay, restart_wanted, schedule_restart,
};
use super::{DesiredState, HealthState, ObservedState, TaskRuntimeState};

/// 一个 Task 进程运行实例的不可复用身份。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TaskRunIdentity {
    /// 影响进程身份的配置代次。
    pub generation: u64,
    /// 每次启动随机生成的运行 ID。
    pub run_id: Uuid,
}

/// 进入单写者引擎的用户级命令。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EngineCommand {
    /// 使全部 Task 进入运行期望并调度就绪项。
    StartAll,
    /// 按反向依赖顺序停止全部活动 Task。
    StopAll,
}

/// 进程适配器返回单写者引擎的运行事件。
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RuntimeEvent {
    /// 指定运行实例已经成功创建。
    Spawned {
        /// Task 稳定标识。
        task_id: TaskId,
        /// 创建结果对应的运行身份。
        identity: TaskRunIdentity,
    },
    /// 指定运行实例创建失败。
    SpawnFailed {
        /// Task 稳定标识。
        task_id: TaskId,
        /// 创建结果对应的运行身份。
        identity: TaskRunIdentity,
    },
    /// 指定运行实例已经退出。
    Exited {
        /// Task 稳定标识。
        task_id: TaskId,
        /// 退出结果对应的运行身份。
        identity: TaskRunIdentity,
        /// 可用时记录平台退出码。
        exit_code: Option<i32>,
        /// 退出是否成功。
        success: bool,
        /// 本次进程从创建到退出的运行毫秒数。
        run_duration_ms: u64,
    },
    /// 指定运行实例的健康检查状态发生变化。
    HealthChanged {
        /// Task 稳定标识。
        task_id: TaskId,
        /// 检查结果对应的运行身份。
        identity: TaskRunIdentity,
        /// 新健康状态。
        health: HealthState,
        /// 触发当前状态的有界检查结果说明。
        detail: Option<String>,
    },
}

/// 单写者状态转换产生的进程副作用意图。
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EngineEffect {
    /// 创建一个新的 Task 运行实例。
    Spawn {
        /// Task 稳定标识。
        task_id: TaskId,
        /// 新运行身份。
        identity: TaskRunIdentity,
        /// 自动重启时应用的有界退避。
        delay_ms: u64,
    },
    /// 停止当前 Task 运行实例。
    Stop {
        /// Task 稳定标识。
        task_id: TaskId,
        /// 只允许停止匹配的运行身份。
        identity: TaskRunIdentity,
    },
}

/// 单写者任务引擎，负责身份校验、依赖判定和副作用规划。
#[derive(Debug)]
pub struct Engine {
    project: String,
    graph: TaskGraph,
    states: BTreeMap<TaskId, TaskRuntimeState>,
    restart: BTreeMap<TaskId, RestartConfig>,
    health_configured: BTreeSet<TaskId>,
    generation: u64,
}

impl Engine {
    /// 根据已编译配置创建尚未执行副作用的引擎。
    pub fn new(spec: &ProjectSpec, graph: TaskGraph) -> Self {
        let states = spec
            .tasks
            .keys()
            .cloned()
            .map(|task_id| (task_id, TaskRuntimeState::default()))
            .collect();
        let restart = restart_configs(spec);
        let health_configured = spec
            .tasks
            .iter()
            .filter(|(_, task)| task.healthcheck.is_some())
            .map(|(task_id, _)| task_id.clone())
            .collect();
        Self {
            project: spec.project.clone(),
            graph,
            states,
            restart,
            health_configured,
            generation: 1,
        }
    }

    /// 返回项目稳定标识。
    pub fn project(&self) -> &str {
        &self.project
    }

    /// 返回任务的当前运行时状态。
    pub fn state(&self, task_id: &TaskId) -> Option<&TaskRuntimeState> {
        self.states.get(task_id)
    }

    /// 按拓扑顺序遍历全部任务状态。
    pub fn states(&self) -> impl Iterator<Item = (&TaskId, &TaskRuntimeState)> {
        self.graph
            .start_order()
            .iter()
            .map(|task_id| (task_id, &self.states[task_id]))
    }

    /// 返回首轮调度使用的确定性启动顺序。
    pub fn initial_start_plan(&self) -> &[TaskId] {
        self.graph.start_order()
    }

    /// 返回当前运行图，供宿主在配置提交失败时构造回退点。
    pub fn graph(&self) -> &TaskGraph {
        &self.graph
    }

    /// 原地更新不改变进程身份的退出与重启策略。
    pub fn update_runtime_policies(&mut self, spec: &ProjectSpec) -> Vec<EngineEffect> {
        self.restart = restart_configs(spec);
        for (task_id, state) in &mut self.states {
            let config = self.restart[task_id];
            if state.desired == DesiredState::Running
                && state.run_id.is_none()
                && restart_wanted(config.policy, state.observed)
            {
                schedule_restart(state, config.max_restarts);
            }
        }
        self.reconcile()
    }

    /// 在替换任务图前把受影响 Task 置为停止期望并生成反向停止意图。
    ///
    /// # Panics
    ///
    /// 仅当已编译任务图与运行状态表内部不一致时 panic。
    pub fn prepare_reconfigure(&mut self, affected: &BTreeSet<TaskId>) -> Vec<EngineEffect> {
        let mut effects = Vec::new();
        for task_id in self.graph.stop_order() {
            if !affected.contains(task_id) {
                continue;
            }
            let state = self.states.get_mut(task_id).expect("任务图与状态一致");
            state.desired = DesiredState::Stopped;
            state.health = HealthState::NotConfigured;
            if let Some(run_id) = state.run_id {
                state.observed = ObservedState::Stopping;
                effects.push(EngineEffect::Stop {
                    task_id: task_id.clone(),
                    identity: TaskRunIdentity {
                        generation: state.generation,
                        run_id,
                    },
                });
            } else {
                state.observed = ObservedState::Exited;
            }
        }
        effects
    }

    /// 提交新任务图，保留无影响 Task 身份并只对账受影响集合。
    pub fn reconfigure(
        &mut self,
        spec: &ProjectSpec,
        graph: TaskGraph,
        affected: &BTreeSet<TaskId>,
        desired_running: bool,
    ) -> Vec<EngineEffect> {
        self.generation = self.generation.saturating_add(1);
        let previous = std::mem::take(&mut self.states);
        self.states = spec
            .tasks
            .keys()
            .cloned()
            .map(|task_id| {
                let state = previous
                    .get(&task_id)
                    .filter(|_| !affected.contains(&task_id));
                let state = state.copied().unwrap_or_else(|| {
                    let mut state = TaskRuntimeState {
                        generation: self.generation,
                        ..TaskRuntimeState::default()
                    };
                    if desired_running {
                        state.health = if spec.tasks[&task_id].healthcheck.is_some() {
                            HealthState::Starting
                        } else {
                            HealthState::NotConfigured
                        };
                    } else {
                        state.desired = DesiredState::Stopped;
                        state.observed = ObservedState::Exited;
                    }
                    state
                });
                (task_id, state)
            })
            .collect();
        self.project.clone_from(&spec.project);
        self.graph = graph;
        self.restart = restart_configs(spec);
        self.health_configured = spec
            .tasks
            .iter()
            .filter(|(_, task)| task.healthcheck.is_some())
            .map(|(task_id, _)| task_id.clone())
            .collect();
        self.reconcile()
    }

    /// 处理一条用户命令并返回需要执行的副作用。
    ///
    /// # Panics
    ///
    /// 仅当构造时的任务图与内部状态表违反一致性不变量时 panic。
    pub fn command(&mut self, command: EngineCommand) -> Vec<EngineEffect> {
        match command {
            EngineCommand::StartAll => {
                self.generation = self.generation.saturating_add(1);
                for (task_id, state) in &mut self.states {
                    state.desired = DesiredState::Running;
                    state.observed = ObservedState::Pending;
                    state.health = if self.health_configured.contains(task_id) {
                        HealthState::Starting
                    } else {
                        HealthState::NotConfigured
                    };
                    state.generation = self.generation;
                    state.run_id = None;
                    state.exit_code = None;
                    state.restart_attempt = 0;
                    state.restart_exhausted = false;
                }
                self.reconcile()
            }
            EngineCommand::StopAll => {
                let mut effects = Vec::new();
                for task_id in self.graph.stop_order() {
                    let state = self.states.get_mut(task_id).expect("任务图与状态一致");
                    state.desired = DesiredState::Stopped;
                    if let Some(run_id) = state.run_id {
                        state.observed = ObservedState::Stopping;
                        effects.push(EngineEffect::Stop {
                            task_id: task_id.clone(),
                            identity: TaskRunIdentity {
                                generation: state.generation,
                                run_id,
                            },
                        });
                    } else {
                        state.observed = ObservedState::Exited;
                    }
                }
                effects
            }
        }
    }

    /// 应用一条带身份的运行事件；迟到事件会被忽略。
    ///
    /// # Panics
    ///
    /// 仅当已通过身份校验的 Task 在内部状态表中意外消失时 panic。
    pub fn event(&mut self, event: RuntimeEvent) -> Vec<EngineEffect> {
        let (task_id, identity) = event_identity(&event);
        if !self.identity_matches(task_id, identity) {
            return Vec::new();
        }
        match event {
            RuntimeEvent::Spawned { task_id, .. } => {
                self.states
                    .get_mut(&task_id)
                    .expect("身份已经匹配")
                    .observed = ObservedState::Running;
            }
            RuntimeEvent::HealthChanged {
                task_id, health, ..
            } => {
                self.states.get_mut(&task_id).expect("身份已经匹配").health = health;
            }
            RuntimeEvent::SpawnFailed { task_id, .. } => {
                self.finish_run(&task_id, None, false, 0);
            }
            RuntimeEvent::Exited {
                task_id,
                exit_code,
                success,
                run_duration_ms,
                ..
            } => self.finish_run(&task_id, exit_code, success, run_duration_ms),
        }
        self.reconcile()
    }

    /// 判断事件身份是否仍对应当前运行实例。
    fn identity_matches(&self, task_id: &TaskId, identity: TaskRunIdentity) -> bool {
        self.states.get(task_id).is_some_and(|state| {
            state.generation == identity.generation && state.run_id == Some(identity.run_id)
        })
    }

    /// 记录一次创建失败或进程退出并应用重启策略。
    fn finish_run(
        &mut self,
        task_id: &TaskId,
        exit_code: Option<i32>,
        success: bool,
        run_duration_ms: u64,
    ) {
        let state = self.states.get_mut(task_id).expect("身份已经匹配");
        state.exit_code = exit_code;
        state.run_id = None;
        state.health = if self.health_configured.contains(task_id) {
            HealthState::Unknown
        } else {
            HealthState::NotConfigured
        };
        state.observed = if success {
            ObservedState::Exited
        } else {
            ObservedState::Failed
        };
        state.restart_exhausted = false;
        if state.desired == DesiredState::Running {
            let config = self.restart[task_id];
            if config.reset_after_ms > 0 && run_duration_ms >= config.reset_after_ms {
                state.restart_attempt = 0;
            }
            if restart_wanted(config.policy, state.observed) {
                schedule_restart(state, config.max_restarts);
            }
        }
    }

    /// 对账全部任务并为依赖已满足的任务生成启动意图。
    fn reconcile(&mut self) -> Vec<EngineEffect> {
        let candidates = self.graph.start_order().to_vec();
        let mut effects = Vec::new();
        for task_id in candidates {
            let state = self.states[&task_id];
            let initial = matches!(
                state.observed,
                ObservedState::Pending | ObservedState::Blocked
            );
            let restarting = state.observed == ObservedState::Backoff;
            if state.desired != DesiredState::Running || (!initial && !restarting) {
                continue;
            }
            if !self.dependencies_satisfied(&task_id) {
                self.states.get_mut(&task_id).expect("任务存在").observed = ObservedState::Blocked;
                continue;
            }
            let identity = TaskRunIdentity {
                generation: state.generation,
                run_id: Uuid::new_v4(),
            };
            let delay_ms = if restarting {
                restart_delay(self.restart[&task_id].delay_ms, state.restart_attempt)
            } else {
                0
            };
            let state = self.states.get_mut(&task_id).expect("任务存在");
            state.run_id = Some(identity.run_id);
            state.observed = ObservedState::Starting;
            state.health = if self.health_configured.contains(&task_id) {
                HealthState::Starting
            } else {
                HealthState::NotConfigured
            };
            effects.push(EngineEffect::Spawn {
                task_id,
                identity,
                delay_ms,
            });
        }
        effects
    }

    /// 判断指定任务的全部依赖边是否已经满足。
    fn dependencies_satisfied(&self, task_id: &TaskId) -> bool {
        self.graph
            .dependencies(task_id)
            .into_iter()
            .all(|(dependency, condition)| {
                let state = self.states[dependency];
                match condition {
                    DependencyCondition::Started => {
                        state.observed == ObservedState::Running || state.exit_code.is_some()
                    }
                    DependencyCondition::Healthy => {
                        state.health == HealthState::Healthy
                            || (state.health == HealthState::NotConfigured
                                && state.observed == ObservedState::Running)
                    }
                    DependencyCondition::CompletedSuccessfully => {
                        state.observed == ObservedState::Exited
                    }
                }
            })
    }
}

/// 从任意运行事件中借用其 Task 与运行身份。
fn event_identity(event: &RuntimeEvent) -> (&TaskId, TaskRunIdentity) {
    match event {
        RuntimeEvent::Spawned { task_id, identity }
        | RuntimeEvent::SpawnFailed { task_id, identity }
        | RuntimeEvent::Exited {
            task_id, identity, ..
        }
        | RuntimeEvent::HealthChanged {
            task_id, identity, ..
        } => (task_id, *identity),
    }
}
