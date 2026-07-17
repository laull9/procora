//! Procora 的领域类型、任务规范与任务图。

mod graph;
mod id;
mod service;
mod spec;

pub use graph::{GraphError, TaskGraph};
pub use id::{TaskId, TaskIdError};
pub use service::{ServiceName, ServiceNameError};
pub use spec::{
    DependencyCondition, DependencySpec, HealthCheckProbe, HealthCheckSpec, HttpHealthCheckSpec,
    HttpScheme, ProjectSpec, RestartPolicy, TaskSpec,
};
