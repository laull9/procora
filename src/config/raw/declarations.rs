use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use crate::{
    config::{TaskConfigOrigins, TaskDefaultsSpec, ValueOrigin},
    core::TaskId,
};

use super::profile::{ProfileSources, RawProfile};
use super::task_defaults::RawTaskDefaults;
use super::{RawTask, task_templates::TemplateSources};
use crate::config::upload::RawUploadTarget;

/// 规范化后仍需保留给编辑器和有效配置说明的声明层元数据。
pub(crate) struct RawDeclarations {
    /// 用户声明的原始变量表达式。
    pub(crate) vars: BTreeMap<String, String>,
    /// 完成链式解析的变量值。
    pub(crate) resolved_vars: BTreeMap<String, String>,
    /// 声明字段路径到直接引用变量名称的映射。
    pub(crate) variable_references: BTreeMap<String, BTreeSet<String>>,
    /// 应用 profile 后的有效项目级默认环境。
    pub(crate) project_env: BTreeMap<String, String>,
    /// 入口与 include 合并后的项目级环境声明，不包含 profile。
    pub(crate) declared_project_env: BTreeMap<String, String>,
    /// 应用 profile 后的有效 Task 默认声明。
    pub(crate) task_defaults: TaskDefaultsSpec,
    /// 入口与 include 合并后的 Task 默认声明，不包含 profile。
    pub(crate) declared_task_defaults: TaskDefaultsSpec,
    /// 当前持久选择的 profile。
    pub(crate) active_profile: Option<String>,
    /// 每个 profile 的直接继承目标。
    pub(crate) profile_extends: BTreeMap<String, String>,
    /// 全部命名 profile 声明，供有效配置和编辑器使用。
    pub(crate) profiles: BTreeMap<String, RawProfile>,
    /// 顶层命名模板的本地声明，供结构化编辑器无展开写回。
    pub(crate) task_templates: BTreeMap<String, RawTask>,
    /// 顶层上传目标的原始声明，供编辑器无损写回。
    pub(crate) uploads: BTreeMap<String, RawUploadTarget>,
    /// 每个 Task 的显式模板引用。
    pub(crate) task_extends: BTreeMap<TaskId, String>,
    /// 每个 Task 显式声明的环境文件路径。
    pub(crate) task_env_files: BTreeMap<TaskId, PathBuf>,
    /// 每个 Task 在环境文件之外显式声明的内联环境。
    pub(crate) task_inline_env: BTreeMap<TaskId, BTreeMap<String, String>>,
    /// 每个有效 Task 的字段与环境变量来源。
    pub(crate) task_origins: BTreeMap<TaskId, TaskConfigOrigins>,
    /// 活动 Task 的原始本地声明，保留变量表达式供编辑器写回。
    pub(crate) task_declarations: BTreeMap<TaskId, RawTask>,
    /// 当前 profile 未准入但仍需由编辑器原样保留的 Task 声明。
    pub(crate) inactive_tasks: BTreeMap<String, RawTask>,
}

/// 根据仍保留字段存在性的原始 Task 生成有效值来源。
pub(super) fn task_origins(
    local: &RawTask,
    effective: &RawTask,
    defaults: &RawTaskDefaults,
    project_env: &BTreeMap<String, String>,
    templates: &TemplateSources,
    profile: &ProfileSources,
) -> TaskConfigOrigins {
    let mut origins = TaskConfigOrigins::default();
    populate_field_origins(&mut origins, local, defaults, templates, profile);
    populate_environment_origins(
        &mut origins,
        local,
        effective,
        defaults,
        project_env,
        templates,
        profile,
    );
    populate_dependency_origins(&mut origins, local, templates);
    origins
}

/// 记录普通标量与列表字段的最终来源。
fn populate_field_origins(
    origins: &mut TaskConfigOrigins,
    local: &RawTask,
    defaults: &RawTaskDefaults,
    templates: &TemplateSources,
    profile: &ProfileSources,
) {
    field_origin(
        origins,
        "command",
        local.command.is_some(),
        templates,
        profile.default_fields.contains("command"),
        false,
    );
    field_origin(
        origins,
        "args",
        local.command.is_some() || local.args.is_some(),
        templates,
        profile.default_fields.contains("args"),
        false,
    );
    field_origin(
        origins,
        "cwd",
        local.cwd.is_some(),
        templates,
        profile.default_fields.contains("cwd"),
        defaults.cwd.is_some(),
    );
    field_origin(
        origins,
        "healthcheck",
        local.healthcheck.is_some(),
        templates,
        profile.default_fields.contains("healthcheck"),
        false,
    );
    field_origin(
        origins,
        "success_exit_codes",
        local.success_exit_codes.is_some(),
        templates,
        profile.default_fields.contains("success_exit_codes"),
        defaults.success_exit_codes.is_some(),
    );
    field_origin(
        origins,
        "depends_on",
        !local.depends_on.is_empty(),
        templates,
        profile.default_fields.contains("depends_on"),
        false,
    );
    field_origin(
        origins,
        "restart",
        local.restart.is_some(),
        templates,
        profile.default_fields.contains("restart"),
        defaults.restart.is_some(),
    );
    field_origin(
        origins,
        "restart_delay_ms",
        local.restart_delay_ms.is_some(),
        templates,
        profile.default_fields.contains("restart_delay_ms"),
        defaults.restart_delay_ms.is_some(),
    );
    field_origin(
        origins,
        "max_restarts",
        local.max_restarts.is_some(),
        templates,
        profile.default_fields.contains("max_restarts"),
        defaults.max_restarts.is_some(),
    );
    field_origin(
        origins,
        "restart_reset_after_ms",
        local.restart_reset_after_ms.is_some(),
        templates,
        profile.default_fields.contains("restart_reset_after_ms"),
        defaults.restart_reset_after_ms.is_some(),
    );
    field_origin(
        origins,
        "shutdown_timeout_ms",
        local.shutdown_timeout_ms.is_some(),
        templates,
        profile.default_fields.contains("shutdown_timeout_ms"),
        defaults.shutdown_timeout_ms.is_some(),
    );
    field_origin(
        origins,
        "env_file",
        local.env_file.is_some(),
        templates,
        profile.default_fields.contains("env_file"),
        false,
    );
}

/// 按项目、默认、模板、环境文件和 Task 顺序记录环境键来源。
fn populate_environment_origins(
    origins: &mut TaskConfigOrigins,
    local: &RawTask,
    effective: &RawTask,
    defaults: &RawTaskDefaults,
    project_env: &BTreeMap<String, String>,
    templates: &TemplateSources,
    profile: &ProfileSources,
) {
    origins.env.extend(project_env.keys().cloned().map(|key| {
        let origin = if profile.project_env.contains(&key) {
            ValueOrigin::Profile
        } else {
            ValueOrigin::ProjectEnv
        };
        (key, origin)
    }));
    origins.env.extend(defaults.env.keys().cloned().map(|key| {
        let origin = if profile.default_env.contains(&key) {
            ValueOrigin::Profile
        } else {
            ValueOrigin::TaskDefaults
        };
        (key, origin)
    }));
    for (key, template) in &templates.env {
        origins.env.insert(key.clone(), ValueOrigin::TaskTemplate);
        origins.template_env.insert(key.clone(), template.clone());
    }
    for key in effective.env.keys() {
        if local.env.contains_key(key) {
            origins.env.insert(key.clone(), ValueOrigin::Task);
            origins.template_env.remove(key);
        } else if let Some(template) = templates.env.get(key) {
            origins.env.insert(key.clone(), ValueOrigin::TaskTemplate);
            origins.template_env.insert(key.clone(), template.clone());
        } else {
            origins.env.insert(key.clone(), ValueOrigin::EnvFile);
            origins.template_env.remove(key);
        }
    }
}

/// 按键记录合并后依赖边的 Task 或具体模板来源。
fn populate_dependency_origins(
    origins: &mut TaskConfigOrigins,
    local: &RawTask,
    templates: &TemplateSources,
) {
    for (dependency, template) in &templates.depends_on {
        origins
            .depends_on
            .insert(dependency.clone(), ValueOrigin::TaskTemplate);
        origins
            .template_depends_on
            .insert(dependency.clone(), template.clone());
    }
    for dependency in local.depends_on.keys() {
        origins
            .depends_on
            .insert(dependency.clone(), ValueOrigin::Task);
        origins.template_depends_on.remove(dependency);
    }
}

/// 写入显式 Task 或内建默认来源。
fn field_origin(
    origins: &mut TaskConfigOrigins,
    field: &str,
    task_value: bool,
    templates: &TemplateSources,
    profile_value: bool,
    default_value: bool,
) {
    let origin = if task_value {
        ValueOrigin::Task
    } else if let Some(template) = templates.fields.get(field) {
        origins.templates.insert(field.to_owned(), template.clone());
        ValueOrigin::TaskTemplate
    } else if profile_value {
        ValueOrigin::Profile
    } else if default_value {
        ValueOrigin::TaskDefaults
    } else {
        ValueOrigin::BuiltIn
    };
    origins.fields.insert(field.to_owned(), origin);
}
