//! 已验证依赖路径的任务字段替换。

use std::path::PathBuf;

use crate::{config::CompiledProject, core::HealthCheckProbe};

use super::ResolvedDependency;

/// 把 `${dependency.name}` 占位符替换为验证后的绝对路径。
pub(super) fn apply(compiled: &mut CompiledProject, resolved: &[ResolvedDependency]) {
    for task in compiled.spec.tasks.values_mut() {
        for dependency in resolved {
            let marker = format!("${{dependency.{}}}", dependency.name);
            let value = dependency.path.to_string_lossy();
            task.command = task.command.replace(&marker, &value);
            for argument in &mut task.args {
                *argument = argument.replace(&marker, &value);
            }
            for env_value in task.env.values_mut() {
                *env_value = env_value.replace(&marker, &value);
            }
            if let Some(cwd) = task.cwd.as_mut() {
                *cwd = PathBuf::from(cwd.to_string_lossy().replace(&marker, &value));
            }
            if let Some(healthcheck) = task.healthcheck.as_mut()
                && let HealthCheckProbe::Exec { command, args, cwd } = &mut healthcheck.probe
            {
                *command = command.replace(&marker, &value);
                for argument in args {
                    *argument = argument.replace(&marker, &value);
                }
                if let Some(cwd) = cwd {
                    *cwd = PathBuf::from(cwd.to_string_lossy().replace(&marker, &value));
                }
            }
        }
    }
}
