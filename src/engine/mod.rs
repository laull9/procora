//! Procora 的任务状态机、调度计划与对账入口。

mod runtime;
mod state;

pub use runtime::{Engine, EngineCommand, EngineEffect, RuntimeEvent, TaskRunIdentity};
pub use state::{DesiredState, HealthState, ObservedState, TaskRuntimeState};
