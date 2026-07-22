use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

use serde::{Deserialize, Serialize};

use crate::config::TaskDefaultsSpec;

use super::{
    ConfigDiagnostic, RawRestartPolicy, RawTask, command::RawCommand, normalize_path,
    normalize_task, task::RawDependencies,
};

/// 只包含适合所有 Task 共享的项目级默认字段。
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawTaskDefaults {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) cwd: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(super) env: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) success_exit_codes: Option<Vec<i32>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) restart: Option<RawRestartPolicy>,
    #[serde(
        rename = "restart_delay",
        alias = "restart_delay_ms",
        default,
        deserialize_with = "crate::config::deserialize_optional_duration",
        serialize_with = "crate::config::serialize_optional_duration",
        skip_serializing_if = "Option::is_none"
    )]
    pub(super) restart_delay_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) max_restarts: Option<u32>,
    #[serde(
        rename = "restart_reset_after",
        alias = "restart_reset_after_ms",
        default,
        deserialize_with = "crate::config::deserialize_optional_duration",
        serialize_with = "crate::config::serialize_optional_duration",
        skip_serializing_if = "Option::is_none"
    )]
    pub(super) restart_reset_after_ms: Option<u64>,
    #[serde(
        rename = "shutdown_timeout",
        alias = "shutdown_timeout_ms",
        default,
        deserialize_with = "crate::config::deserialize_optional_duration",
        serialize_with = "crate::config::serialize_optional_duration",
        skip_serializing_if = "Option::is_none"
    )]
    pub(super) shutdown_timeout_ms: Option<u64>,
}

impl RawTaskDefaults {
    /// 判断当前默认层是否没有声明任何字段。
    pub(super) fn is_empty(&self) -> bool {
        self.cwd.is_none()
            && self.env.is_empty()
            && self.success_exit_codes.is_none()
            && self.restart.is_none()
            && self.restart_delay_ms.is_none()
            && self.max_restarts.is_none()
            && self.restart_reset_after_ms.is_none()
            && self.shutdown_timeout_ms.is_none()
    }

    /// 返回当前声明覆盖的普通字段集合，不包含逐键环境映射。
    pub(super) fn declared_fields(&self) -> BTreeSet<String> {
        [
            ("cwd", self.cwd.is_some()),
            ("success_exit_codes", self.success_exit_codes.is_some()),
            ("restart", self.restart.is_some()),
            ("restart_delay_ms", self.restart_delay_ms.is_some()),
            ("max_restarts", self.max_restarts.is_some()),
            (
                "restart_reset_after_ms",
                self.restart_reset_after_ms.is_some(),
            ),
            ("shutdown_timeout_ms", self.shutdown_timeout_ms.is_some()),
        ]
        .into_iter()
        .filter(|(_, declared)| *declared)
        .map(|(field, _)| field.to_owned())
        .collect()
    }

    /// 用更高优先级文档中实际声明的字段覆盖当前默认值。
    pub(super) fn overlay(&mut self, higher: Self) {
        replace_if_some(&mut self.cwd, higher.cwd);
        self.env.extend(higher.env);
        replace_if_some(&mut self.success_exit_codes, higher.success_exit_codes);
        replace_if_some(&mut self.restart, higher.restart);
        replace_if_some(&mut self.restart_delay_ms, higher.restart_delay_ms);
        replace_if_some(&mut self.max_restarts, higher.max_restarts);
        replace_if_some(
            &mut self.restart_reset_after_ms,
            higher.restart_reset_after_ms,
        );
        replace_if_some(&mut self.shutdown_timeout_ms, higher.shutdown_timeout_ms);
    }

    /// 把声明文件中的相对工作目录改写为稳定路径。
    pub(super) fn rebase(&mut self, base: &std::path::Path) {
        self.cwd = self
            .cwd
            .take()
            .map(|path| normalize_path(&path, Some(base)));
    }

    /// 把默认字段应用到尚未声明对应字段的 Task。
    pub(super) fn apply_to(&self, normalized: &TaskDefaultsSpec, task: &mut RawTask) {
        task.cwd = task.cwd.take().or_else(|| normalized.cwd.clone());
        let mut env = self.env.clone();
        env.extend(std::mem::take(&mut task.env));
        task.env = env;
        if task.success_exit_codes.is_none() {
            task.success_exit_codes = normalized
                .success_exit_codes
                .as_ref()
                .map(|values| values.iter().copied().collect());
        }
        inherit(&mut task.restart, self.restart.as_ref());
        inherit(&mut task.restart_delay_ms, self.restart_delay_ms.as_ref());
        inherit(&mut task.max_restarts, self.max_restarts.as_ref());
        inherit(
            &mut task.restart_reset_after_ms,
            self.restart_reset_after_ms.as_ref(),
        );
        inherit(
            &mut task.shutdown_timeout_ms,
            self.shutdown_timeout_ms.as_ref(),
        );
    }

    /// 独立校验默认声明并返回规范化、可序列化的声明层值。
    pub(super) fn normalize(
        &self,
        base_directory: Option<&std::path::Path>,
        diagnostics: &mut Vec<ConfigDiagnostic>,
    ) -> TaskDefaultsSpec {
        self.normalize_at("task_defaults", base_directory, diagnostics)
    }

    /// 在指定字段路径独立校验默认声明，供未启用 profile 复用。
    pub(super) fn normalize_at(
        &self,
        path: &str,
        base_directory: Option<&std::path::Path>,
        diagnostics: &mut Vec<ConfigDiagnostic>,
    ) -> TaskDefaultsSpec {
        let raw = RawTask {
            extends: None,
            command: Some(RawCommand::Program("__procora_task_defaults__".to_owned())),
            args: None,
            cwd: self.cwd.clone(),
            env: self.env.clone(),
            inline_env_before_file: None,
            env_file: None,
            healthcheck: None,
            success_exit_codes: self.success_exit_codes.clone(),
            depends_on: RawDependencies::default(),
            uploads: BTreeMap::new(),
            restart: self.restart,
            restart_delay_ms: self.restart_delay_ms,
            max_restarts: self.max_restarts,
            restart_reset_after_ms: self.restart_reset_after_ms,
            shutdown_timeout_ms: self.shutdown_timeout_ms,
        };
        let empty_ids = BTreeSet::new();
        let empty_env = BTreeMap::new();
        let normalized = normalize_task(
            raw,
            path,
            base_directory,
            &empty_ids,
            &empty_env,
            true,
            diagnostics,
        );
        TaskDefaultsSpec {
            cwd: self.cwd.as_ref().and(normalized.cwd),
            env: self.env.clone(),
            success_exit_codes: self
                .success_exit_codes
                .as_ref()
                .map(|_| normalized.success_exit_codes),
            restart: self.restart.map(|_| normalized.restart),
            restart_delay_ms: self.restart_delay_ms.map(|_| normalized.restart_delay_ms),
            max_restarts: self.max_restarts.map(|_| normalized.max_restarts),
            restart_reset_after_ms: self
                .restart_reset_after_ms
                .map(|_| normalized.restart_reset_after_ms),
            shutdown_timeout_ms: self
                .shutdown_timeout_ms
                .map(|_| normalized.shutdown_timeout_ms),
        }
    }
}

/// 仅在高优先级文档显式声明时替换标量或列表。
fn replace_if_some<T>(target: &mut Option<T>, higher: Option<T>) {
    if higher.is_some() {
        *target = higher;
    }
}

/// Task 未声明字段时继承项目级默认值。
fn inherit<T: Clone>(target: &mut Option<T>, default: Option<&T>) {
    if target.is_none() {
        *target = default.cloned();
    }
}
