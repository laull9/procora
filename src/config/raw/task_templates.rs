use std::collections::{BTreeMap, BTreeSet};

use crate::core::TaskId;

use super::{
    ConfigDiagnostic, RawProject, RawTask, command::RawCommand, diagnostic, normalize_task,
    valid_dependency_id,
};

/// 一个 Task 从模板继承字段时的最终模板名称。
#[derive(Clone, Debug, Default)]
pub(super) struct TemplateSources {
    /// 普通字段到最终获胜模板的映射。
    pub(super) fields: BTreeMap<String, String>,
    /// 环境变量键到最终获胜模板的映射。
    pub(super) env: BTreeMap<String, String>,
    /// 依赖边到最终获胜模板的映射。
    pub(super) depends_on: BTreeMap<String, String>,
}

/// 已解析模板的有效声明和值来源。
#[derive(Clone, Debug, Default)]
struct ResolvedTemplate {
    task: RawTask,
    sources: TemplateSources,
}

impl RawProject {
    /// 在读取环境文件前解析模板链，同时保留 Task 本地声明供来源和编辑器使用。
    pub(super) fn resolve_task_templates(&mut self, diagnostics: &mut Vec<ConfigDiagnostic>) {
        let templates = self.task_templates.clone();
        let mut cache = BTreeMap::new();
        let mut failed = BTreeSet::new();
        for name in templates.keys() {
            if !valid_dependency_id(name) {
                diagnostics.push(diagnostic(
                    format!("task_templates.{name}"),
                    "模板名称只能包含 ASCII 字母、数字、点、短横线和下划线",
                ));
                failed.insert(name.clone());
                continue;
            }
            let mut stack = Vec::new();
            resolve_template(
                name,
                &templates,
                &mut cache,
                &mut failed,
                &mut stack,
                diagnostics,
            );
        }
        validate_templates(&cache, self.tasks.keys(), diagnostics);

        let declarations = std::mem::take(&mut self.tasks);
        let mut resolved_tasks = BTreeMap::new();
        let mut task_sources = BTreeMap::new();
        for (name, declaration) in &declarations {
            let mut resolved = ResolvedTemplate::default();
            if let Some(template) = declaration.extends.as_deref() {
                if let Some(base) = cache.get(template) {
                    resolved = base.clone();
                } else if !failed.contains(template) {
                    diagnostics.push(diagnostic(
                        format!("tasks.{name}.extends"),
                        format!("引用了不存在的模板 `{template}`"),
                    ));
                }
            }
            overlay_task(&mut resolved.task, declaration.clone());
            resolved.task.extends = None;
            resolved_tasks.insert(name.clone(), resolved.task);
            task_sources.insert(name.clone(), resolved.sources);
        }
        self.tasks = resolved_tasks;
        self.task_declarations = declarations;
        self.task_template_sources = task_sources;
    }
}

/// 独立校验已解析模板，即使它当前没有被任何 Task 引用。
fn validate_templates<'a>(
    templates: &BTreeMap<String, ResolvedTemplate>,
    task_names: impl Iterator<Item = &'a String>,
    diagnostics: &mut Vec<ConfigDiagnostic>,
) {
    let valid_ids = task_names
        .filter_map(|name| name.parse::<TaskId>().ok())
        .collect::<BTreeSet<_>>();
    let empty_env = BTreeMap::new();
    for (name, template) in templates {
        let mut task = template.task.clone();
        if task.command.is_none() {
            task.command = Some(RawCommand::Program("__procora_task_template__".to_owned()));
        }
        normalize_task(
            task,
            &format!("task_templates.{name}"),
            None,
            &valid_ids,
            &empty_env,
            true,
            diagnostics,
        );
    }
}

/// 深度优先解析单个模板，并拒绝未知引用、自引用和循环链。
fn resolve_template(
    name: &str,
    templates: &BTreeMap<String, RawTask>,
    cache: &mut BTreeMap<String, ResolvedTemplate>,
    failed: &mut BTreeSet<String>,
    stack: &mut Vec<String>,
    diagnostics: &mut Vec<ConfigDiagnostic>,
) -> Option<ResolvedTemplate> {
    if let Some(resolved) = cache.get(name) {
        return Some(resolved.clone());
    }
    if failed.contains(name) {
        return None;
    }
    if let Some(position) = stack.iter().position(|item| item == name) {
        let chain = stack[position..]
            .iter()
            .chain(std::iter::once(&name.to_owned()))
            .cloned()
            .collect::<Vec<_>>()
            .join(" -> ");
        diagnostics.push(diagnostic(
            format!("task_templates.{name}.extends"),
            format!("检测到模板继承循环：{chain}"),
        ));
        failed.extend(stack[position..].iter().cloned());
        return None;
    }
    let declaration = templates.get(name)?;

    stack.push(name.to_owned());
    let mut resolved = ResolvedTemplate::default();
    if let Some(base_name) = declaration.extends.as_deref() {
        if base_name == name {
            diagnostics.push(diagnostic(
                format!("task_templates.{name}.extends"),
                "模板不能继承自身",
            ));
            failed.insert(name.to_owned());
        } else if !templates.contains_key(base_name) {
            diagnostics.push(diagnostic(
                format!("task_templates.{name}.extends"),
                format!("引用了不存在的模板 `{base_name}`"),
            ));
            failed.insert(name.to_owned());
        } else if let Some(base) =
            resolve_template(base_name, templates, cache, failed, stack, diagnostics)
        {
            resolved = base;
        }
    }
    stack.pop();
    if failed.contains(name) {
        return None;
    }

    record_sources(&mut resolved.sources, declaration, name);
    overlay_task(&mut resolved.task, declaration.clone());
    resolved.task.extends = None;
    cache.insert(name.to_owned(), resolved.clone());
    Some(resolved)
}

/// 按 map 合并、标量和列表替换的固定规则应用更高优先级声明。
fn overlay_task(target: &mut RawTask, mut higher: RawTask) {
    if higher.command.is_some() {
        target.command = higher.command.take();
        target.args = higher.args.take();
    } else if higher.args.is_some() {
        target.args = higher.args.take();
    }
    replace_if_some(&mut target.cwd, higher.cwd);
    target.env.extend(higher.env);
    replace_if_some(&mut target.env_file, higher.env_file);
    replace_if_some(&mut target.healthcheck, higher.healthcheck);
    replace_if_some(&mut target.success_exit_codes, higher.success_exit_codes);
    target.depends_on.extend(higher.depends_on);
    replace_if_some(&mut target.restart, higher.restart);
    replace_if_some(&mut target.restart_delay_ms, higher.restart_delay_ms);
    replace_if_some(&mut target.max_restarts, higher.max_restarts);
    replace_if_some(
        &mut target.restart_reset_after_ms,
        higher.restart_reset_after_ms,
    );
    replace_if_some(&mut target.shutdown_timeout_ms, higher.shutdown_timeout_ms);
}

/// 记录当前模板实际声明并因此获胜的字段和环境键。
fn record_sources(sources: &mut TemplateSources, task: &RawTask, name: &str) {
    for (field, declared) in [
        ("command", task.command.is_some()),
        ("args", task.command.is_some() || task.args.is_some()),
        ("cwd", task.cwd.is_some()),
        ("env_file", task.env_file.is_some()),
        ("healthcheck", task.healthcheck.is_some()),
        ("success_exit_codes", task.success_exit_codes.is_some()),
        ("depends_on", !task.depends_on.is_empty()),
        ("restart", task.restart.is_some()),
        ("restart_delay_ms", task.restart_delay_ms.is_some()),
        ("max_restarts", task.max_restarts.is_some()),
        (
            "restart_reset_after_ms",
            task.restart_reset_after_ms.is_some(),
        ),
        ("shutdown_timeout_ms", task.shutdown_timeout_ms.is_some()),
    ] {
        if declared {
            sources.fields.insert(field.to_owned(), name.to_owned());
        }
    }
    sources
        .env
        .extend(task.env.keys().cloned().map(|key| (key, name.to_owned())));
    sources.depends_on.extend(
        task.depends_on
            .keys()
            .cloned()
            .map(|key| (key, name.to_owned())),
    );
}

/// 仅在更高优先级显式声明时替换字段。
fn replace_if_some<T>(target: &mut Option<T>, higher: Option<T>) {
    if higher.is_some() {
        *target = higher;
    }
}
