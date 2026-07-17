use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::{ConfigDiagnostic, RawProject, diagnostic, task_defaults::RawTaskDefaults};

/// 命名运行场景对项目共享层和 Task 准入的显式覆盖。
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawProfile {
    /// 可选基础 profile；继承链在全部 include 合并后解析。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) extends: Option<String>,
    /// 只允许运行这些已声明 Task；省略时保留全部 Task。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) tasks: Option<Vec<String>>,
    /// 覆盖项目环境的键。
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(super) env: BTreeMap<String, String>,
    /// 覆盖项目 Task 默认层的字段。
    #[serde(default, skip_serializing_if = "RawTaskDefaults::is_empty")]
    pub(super) task_defaults: RawTaskDefaults,
}

/// 当前 profile 实际覆盖的键和字段集合。
#[derive(Clone, Debug, Default)]
pub(super) struct ProfileSources {
    /// profile 覆盖的项目环境键。
    pub(super) project_env: BTreeSet<String>,
    /// profile 覆盖的 Task 默认环境键。
    pub(super) default_env: BTreeSet<String>,
    /// profile 覆盖的 Task 默认普通字段。
    pub(super) default_fields: BTreeSet<String>,
}

impl RawProfile {
    /// 合并同名 profile：map 按键合并，标量和 Task 白名单整体替换。
    pub(super) fn overlay(&mut self, higher: Self) {
        if higher.extends.is_some() {
            self.extends = higher.extends;
        }
        if higher.tasks.is_some() {
            self.tasks = higher.tasks;
        }
        self.env.extend(higher.env);
        self.task_defaults.overlay(higher.task_defaults);
    }

    /// 把声明文件中的 profile 相对路径改写为稳定路径。
    pub(super) fn rebase(&mut self, base: &std::path::Path) {
        self.task_defaults.rebase(base);
    }
}

impl RawProject {
    /// 校验命名 profile，并应用配置中持久选择的场景覆盖。
    pub(super) fn apply_profile(&mut self, diagnostics: &mut Vec<ConfigDiagnostic>) {
        let task_names = self.tasks.keys().cloned().collect::<BTreeSet<_>>();
        for (name, profile) in &self.profiles {
            if !super::valid_dependency_id(name) {
                diagnostics.push(diagnostic(
                    format!("profiles.{name}"),
                    "profile 名称只能包含 ASCII 字母、数字、点、短横线和下划线",
                ));
            }
            validate_task_selection(name, profile, &task_names, diagnostics);
        }

        let profiles = self.profiles.clone();
        let mut resolved = BTreeMap::new();
        for name in profiles.keys() {
            let mut visiting = Vec::new();
            resolve_profile(name, &profiles, &mut resolved, &mut visiting, diagnostics);
        }

        let Some(name) = self.profile.as_deref() else {
            return;
        };
        let Some(profile) = resolved.get(name) else {
            if !profiles.contains_key(name) {
                diagnostics.push(diagnostic(
                    "profile",
                    format!("引用了不存在的 profile `{name}`"),
                ));
            }
            return;
        };
        self.profile_sources.project_env = profile.env.keys().cloned().collect();
        self.profile_sources.default_env = profile.task_defaults.env.keys().cloned().collect();
        self.profile_sources.default_fields = profile.task_defaults.declared_fields();
        self.admitted_tasks = profile
            .tasks
            .as_ref()
            .map(|tasks| tasks.iter().cloned().collect());
        self.env.extend(profile.env.clone());
        self.task_defaults.overlay(profile.task_defaults.clone());
    }
}

/// 解析一条 profile 继承链，并缓存已完成的有效声明。
fn resolve_profile(
    name: &str,
    profiles: &BTreeMap<String, RawProfile>,
    cache: &mut BTreeMap<String, RawProfile>,
    visiting: &mut Vec<String>,
    diagnostics: &mut Vec<ConfigDiagnostic>,
) -> Option<RawProfile> {
    if let Some(profile) = cache.get(name) {
        return Some(profile.clone());
    }
    if let Some(position) = visiting.iter().position(|item| item == name) {
        let mut chain = visiting[position..].to_vec();
        chain.push(name.to_owned());
        diagnostics.push(diagnostic(
            format!("profiles.{name}.extends"),
            format!("profile 继承形成循环：{}", chain.join(" -> ")),
        ));
        return None;
    }
    let profile = profiles.get(name)?.clone();
    visiting.push(name.to_owned());
    let mut resolved = if let Some(base) = profile.extends.as_deref() {
        if base == name {
            diagnostics.push(diagnostic(
                format!("profiles.{name}.extends"),
                "profile 不能继承自身",
            ));
            None
        } else if !profiles.contains_key(base) {
            diagnostics.push(diagnostic(
                format!("profiles.{name}.extends"),
                format!("引用了不存在的 profile `{base}`"),
            ));
            None
        } else {
            resolve_profile(base, profiles, cache, visiting, diagnostics)
        }
    } else {
        Some(RawProfile::default())
    };
    visiting.pop();
    let resolved = resolved.as_mut().map(|base| {
        base.overlay(profile);
        base.extends = None;
        base.clone()
    });
    if let Some(profile) = &resolved {
        cache.insert(name.to_owned(), profile.clone());
    }
    resolved
}

/// 校验 profile Task 白名单中的重复项和未知 Task。
fn validate_task_selection(
    profile_name: &str,
    profile: &RawProfile,
    task_names: &BTreeSet<String>,
    diagnostics: &mut Vec<ConfigDiagnostic>,
) {
    let Some(tasks) = &profile.tasks else {
        return;
    };
    let mut seen = BTreeSet::new();
    for (index, task) in tasks.iter().enumerate() {
        let path = format!("profiles.{profile_name}.tasks.{index}");
        if !seen.insert(task) {
            diagnostics.push(diagnostic(path, format!("Task `{task}` 重复出现")));
        } else if !task_names.contains(task) {
            diagnostics.push(diagnostic(path, format!("引用了不存在的 Task `{task}`")));
        }
    }
}
