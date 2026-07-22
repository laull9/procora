use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use crate::{
    config::{ManagedDependencies, TaskConfigOrigins, UploadTargetSpec},
    core::{ProjectSpec, TaskId, TaskSpec},
};

use super::{
    ConfigDiagnostic, RawProject, RawTask, declarations::RawDeclarations, diagnostic,
    normalize_dependencies, normalize_path, normalize_project, normalize_task,
    profile::ProfileSources, task_defaults::RawTaskDefaults, task_templates::TemplateSources,
};

/// 全部 Task 规范化后需要交给领域层和声明层的结果。
#[derive(Default)]
struct NormalizedTasks {
    tasks: BTreeMap<TaskId, TaskSpec>,
    task_extends: BTreeMap<TaskId, String>,
    task_env_files: BTreeMap<TaskId, PathBuf>,
    task_inline_env: BTreeMap<TaskId, BTreeMap<String, String>>,
    task_origins: BTreeMap<TaskId, TaskConfigOrigins>,
    task_declarations: BTreeMap<TaskId, RawTask>,
    inactive_tasks: BTreeMap<String, RawTask>,
}

/// 单次 Task 规范化共享的有效层与诊断上下文。
struct TaskNormalization<'a> {
    base_directory: Option<&'a Path>,
    valid_ids: BTreeSet<TaskId>,
    admitted_tasks: Option<BTreeSet<String>>,
    task_defaults: &'a RawTaskDefaults,
    normalized_defaults: &'a crate::config::TaskDefaultsSpec,
    project_env: &'a BTreeMap<String, String>,
    profile_sources: &'a ProfileSources,
    diagnostics: &'a mut Vec<ConfigDiagnostic>,
}

/// 原始配置完成校验与规范化后的编译输入集合。
pub(crate) type NormalizedProject = (
    ProjectSpec,
    ManagedDependencies,
    BTreeMap<String, UploadTargetSpec>,
    RawDeclarations,
);

impl RawProject {
    /// 校验并规范化原始 DTO，独立错误尽量一次返回。
    pub(crate) fn normalize(
        self,
        base_directory: Option<&Path>,
    ) -> Result<NormalizedProject, Vec<ConfigDiagnostic>> {
        let mut diagnostics = Vec::new();
        validate_profile_defaults(&self, base_directory, &mut diagnostics);
        let version = normalize_version(self.version, &mut diagnostics);
        let project = normalize_project(self.project, &mut diagnostics);
        let normalized_task_defaults = self
            .task_defaults
            .normalize(base_directory, &mut diagnostics);
        let normalized_declared_defaults = if self.profile.is_some() || !self.vars.is_empty() {
            self.declared_task_defaults
                .normalize(base_directory, &mut diagnostics)
        } else {
            normalized_task_defaults.clone()
        };
        let dependencies = normalize_dependencies(self.dependencies, &mut diagnostics);
        let uploads = normalize_uploads(
            &self.uploads,
            &self.tasks,
            self.admitted_tasks.as_ref(),
            &mut diagnostics,
        );
        let valid_ids = self
            .tasks
            .keys()
            .filter_map(|value| value.parse::<TaskId>().ok())
            .collect();
        let mut context = TaskNormalization {
            base_directory,
            valid_ids,
            admitted_tasks: self.admitted_tasks,
            task_defaults: &self.task_defaults,
            normalized_defaults: &normalized_task_defaults,
            project_env: &self.env,
            profile_sources: &self.profile_sources,
            diagnostics: &mut diagnostics,
        };
        let normalized_tasks = context.run(
            self.tasks,
            self.task_declarations,
            self.task_template_sources,
            self.declared_tasks,
        );
        if !diagnostics.is_empty() {
            return Err(diagnostics);
        }
        Ok((
            ProjectSpec {
                version,
                project,
                tasks: normalized_tasks.tasks,
            },
            dependencies,
            uploads,
            RawDeclarations {
                vars: self.vars,
                resolved_vars: self.resolved_vars,
                variable_references: self.variable_references,
                project_env: self.env,
                declared_project_env: self.declared_env,
                task_defaults: normalized_task_defaults,
                declared_task_defaults: normalized_declared_defaults,
                active_profile: self.profile,
                profile_extends: self
                    .declared_profiles
                    .iter()
                    .filter_map(|(name, profile)| {
                        profile
                            .extends
                            .as_ref()
                            .map(|base| (name.clone(), base.clone()))
                    })
                    .collect(),
                profiles: self.declared_profiles,
                task_templates: self.declared_task_templates,
                uploads: self.declared_uploads,
                task_extends: normalized_tasks.task_extends,
                task_env_files: normalized_tasks.task_env_files,
                task_inline_env: normalized_tasks.task_inline_env,
                task_origins: normalized_tasks.task_origins,
                task_declarations: normalized_tasks.task_declarations,
                inactive_tasks: normalized_tasks.inactive_tasks,
            },
        ))
    }
}

/// 把 Service 与活动 Task 的上传目标展平成稳定选择器后缀。
fn normalize_uploads(
    service_uploads: &BTreeMap<String, crate::config::upload::RawUploadTarget>,
    tasks: &BTreeMap<String, RawTask>,
    admitted_tasks: Option<&BTreeSet<String>>,
    diagnostics: &mut Vec<ConfigDiagnostic>,
) -> BTreeMap<String, UploadTargetSpec> {
    let mut output = BTreeMap::new();
    for (name, target) in service_uploads {
        let field = format!("uploads.{name}");
        if !super::valid_dependency_id(name) {
            diagnostics.push(diagnostic(
                &field,
                "上传目标名称只能包含 ASCII 字母、数字、点、短横线和下划线",
            ));
        } else if let Some(target) = target.clone().normalize(&field, diagnostics) {
            output.insert(name.to_owned(), target);
        }
    }
    for (task, declaration) in tasks {
        if admitted_tasks.is_some_and(|tasks| !tasks.contains(task)) {
            continue;
        }
        for (name, target) in &declaration.uploads {
            let field = format!("tasks.{task}.uploads.{name}");
            if !super::valid_dependency_id(name) {
                diagnostics.push(diagnostic(
                    &field,
                    "上传目标名称只能包含 ASCII 字母、数字、点、短横线和下划线",
                ));
            } else if let Some(target) = target.clone().normalize(&field, diagnostics) {
                output.insert(format!("{task}::{name}"), target);
            }
        }
    }
    output
}

impl TaskNormalization<'_> {
    /// 规范化全部 Task，并把未准入声明与活动 Task 元数据分开保存。
    fn run(
        &mut self,
        tasks: BTreeMap<String, RawTask>,
        mut declarations: BTreeMap<String, RawTask>,
        mut template_sources: BTreeMap<String, TemplateSources>,
        mut raw_declarations: BTreeMap<String, RawTask>,
    ) -> NormalizedTasks {
        let mut output = NormalizedTasks::default();
        for (raw_id, mut raw_task) in tasks {
            let path = format!("tasks.{raw_id}");
            let Ok(task_id) = raw_id.parse::<TaskId>() else {
                self.diagnostics
                    .push(diagnostic(&path, "Task ID 包含非法字符"));
                continue;
            };
            let local = declarations
                .remove(&raw_id)
                .unwrap_or_else(|| raw_task.clone());
            let sources = template_sources.remove(&raw_id).unwrap_or_default();
            let raw_local = raw_declarations
                .remove(&raw_id)
                .unwrap_or_else(|| local.clone());
            let active = self
                .admitted_tasks
                .as_ref()
                .is_none_or(|tasks| tasks.contains(&raw_id));
            self.record_declaration(&raw_id, &task_id, &raw_local, &mut output, active);
            if active {
                output.task_origins.insert(
                    task_id.clone(),
                    super::declarations::task_origins(
                        &local,
                        &raw_task,
                        self.task_defaults,
                        self.project_env,
                        &sources,
                        self.profile_sources,
                    ),
                );
            }
            raw_task.validate_runtime_limits(&path, self.diagnostics);
            self.task_defaults
                .apply_to(self.normalized_defaults, &mut raw_task);
            let task = normalize_task(
                raw_task,
                &path,
                self.base_directory,
                &self.valid_ids,
                self.project_env,
                false,
                self.diagnostics,
            );
            if active {
                output.tasks.insert(task_id, task);
            }
        }
        output
    }

    /// 保存活动 Task 的本地声明元数据，或保留未准入 Task 的完整声明。
    fn record_declaration(
        &self,
        raw_id: &str,
        task_id: &TaskId,
        local: &RawTask,
        output: &mut NormalizedTasks,
        active: bool,
    ) {
        if !active {
            output
                .inactive_tasks
                .insert(raw_id.to_owned(), local.clone());
            return;
        }
        output
            .task_declarations
            .insert(task_id.clone(), local.clone());
        if let Some(template) = &local.extends {
            output
                .task_extends
                .insert(task_id.clone(), template.clone());
        }
        if let Some(env_file) = &local.env_file {
            output.task_env_files.insert(
                task_id.clone(),
                normalize_path(env_file, self.base_directory),
            );
        }
        if !local.env.is_empty() {
            output
                .task_inline_env
                .insert(task_id.clone(), local.env.clone());
        }
    }
}

/// 规范化配置版本并聚合缺失或不支持诊断。
fn normalize_version(version: Option<u32>, diagnostics: &mut Vec<ConfigDiagnostic>) -> u32 {
    let version = version.unwrap_or_else(|| {
        diagnostics.push(diagnostic("version", "缺少必需字段"));
        0
    });
    if version != 0 && version != 1 {
        diagnostics.push(diagnostic(
            "version",
            format!("不支持版本 {version}，当前只支持版本 1"),
        ));
    }
    version
}

/// 独立校验全部 profile 默认层，即使当前没有选择它。
fn validate_profile_defaults(
    project: &RawProject,
    base_directory: Option<&Path>,
    diagnostics: &mut Vec<ConfigDiagnostic>,
) {
    for (name, profile) in &project.profiles {
        profile.task_defaults.normalize_at(
            &format!("profiles.{name}.task_defaults"),
            base_directory,
            diagnostics,
        );
    }
}
