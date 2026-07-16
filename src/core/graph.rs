use std::collections::BTreeMap;

use petgraph::{
    Direction,
    algo::toposort,
    graph::{DiGraph, NodeIndex},
    visit::EdgeRef,
};
use thiserror::Error;

use super::{DependencyCondition, ProjectSpec, TaskId};

/// 任务图编译错误。
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum GraphError {
    /// 依赖指向不存在的任务。
    #[error("任务 `{task}` 依赖不存在的任务 `{dependency}`")]
    MissingDependency {
        /// 声明依赖的任务。
        task: TaskId,
        /// 不存在的依赖。
        dependency: TaskId,
    },
    /// 任务图中检测到环。
    #[error("任务依赖图存在环，涉及任务 `{task}`")]
    Cycle {
        /// 环中可定位的一个任务。
        task: TaskId,
    },
}

/// 已完成依赖解析与环检测的任务图。
#[derive(Clone, Debug)]
pub struct TaskGraph {
    graph: DiGraph<TaskId, DependencyCondition>,
    indices: BTreeMap<TaskId, NodeIndex>,
    start_order: Vec<TaskId>,
}

impl TaskGraph {
    /// 从规范化项目配置编译任务图。
    ///
    /// # Errors
    ///
    /// 当依赖目标不存在或任务图包含环时返回错误。
    pub fn compile(spec: &ProjectSpec) -> Result<Self, GraphError> {
        let mut graph = DiGraph::new();
        let indices = spec
            .tasks
            .keys()
            .cloned()
            .map(|task_id| {
                let index = graph.add_node(task_id.clone());
                (task_id, index)
            })
            .collect::<BTreeMap<_, _>>();

        for (task_id, task) in &spec.tasks {
            let task_index = indices[task_id];
            for (dependency_id, dependency) in &task.depends_on {
                let Some(&dependency_index) = indices.get(dependency_id) else {
                    return Err(GraphError::MissingDependency {
                        task: task_id.clone(),
                        dependency: dependency_id.clone(),
                    });
                };
                graph.add_edge(dependency_index, task_index, dependency.condition);
            }
        }

        let sorted = toposort(&graph, None).map_err(|cycle| GraphError::Cycle {
            task: graph[cycle.node_id()].clone(),
        })?;
        let start_order = sorted
            .into_iter()
            .map(|index| graph[index].clone())
            .collect();

        Ok(Self {
            graph,
            indices,
            start_order,
        })
    }

    /// 返回确定性的任务启动拓扑序。
    pub fn start_order(&self) -> &[TaskId] {
        &self.start_order
    }

    /// 返回任务停止时使用的反向拓扑序。
    pub fn stop_order(&self) -> impl DoubleEndedIterator<Item = &TaskId> {
        self.start_order.iter().rev()
    }

    /// 返回直接依赖指定任务的下游任务。
    pub fn dependents(&self, task_id: &TaskId) -> Vec<&TaskId> {
        self.indices.get(task_id).map_or_else(Vec::new, |index| {
            self.graph
                .neighbors_directed(*index, Direction::Outgoing)
                .map(|neighbor| &self.graph[neighbor])
                .collect()
        })
    }

    /// 返回指定任务的直接依赖及其满足条件。
    pub fn dependencies(&self, task_id: &TaskId) -> Vec<(&TaskId, DependencyCondition)> {
        self.indices.get(task_id).map_or_else(Vec::new, |index| {
            self.graph
                .edges_directed(*index, Direction::Incoming)
                .map(|edge| (&self.graph[edge.source()], *edge.weight()))
                .collect()
        })
    }
}
