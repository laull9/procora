use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

use super::{
    ConfigDiagnostic, RawProject, RawTask, command::RawCommand, diagnostic,
    task_defaults::RawTaskDefaults,
};

/// 一个变量字符串解析后的字面量或引用片段。
#[derive(Clone, Debug)]
enum Segment {
    Literal(String),
    Reference(String),
}

/// 解析项目变量定义并为其他配置字段提供确定性插值。
struct VariableResolver<'a> {
    declarations: &'a BTreeMap<String, String>,
    resolved: BTreeMap<String, String>,
    failed: BTreeSet<String>,
    stack: Vec<String>,
}

impl RawProject {
    /// 保留声明表达式、解析变量链，并展开显式支持的字符串字段。
    pub(super) fn resolve_variables(&mut self, diagnostics: &mut Vec<ConfigDiagnostic>) {
        self.capture_variable_declarations();
        let mut resolver = VariableResolver::new(&self.vars);
        resolver.resolve_all(diagnostics);
        self.resolved_vars = resolver.resolved.clone();
        let mut references = BTreeMap::new();
        expand_map(
            &mut self.env,
            "env",
            &resolver,
            &mut references,
            diagnostics,
        );
        expand_defaults(
            &mut self.task_defaults,
            "task_defaults",
            &resolver,
            &mut references,
            diagnostics,
        );
        for (name, profile) in &mut self.profiles {
            expand_map(
                &mut profile.env,
                &format!("profiles.{name}.env"),
                &resolver,
                &mut references,
                diagnostics,
            );
            expand_defaults(
                &mut profile.task_defaults,
                &format!("profiles.{name}.task_defaults"),
                &resolver,
                &mut references,
                diagnostics,
            );
        }
        for (name, template) in &mut self.task_templates {
            expand_task(
                template,
                &format!("task_templates.{name}"),
                &resolver,
                &mut references,
                diagnostics,
            );
        }
        for (name, task) in &mut self.tasks {
            expand_task(
                task,
                &format!("tasks.{name}"),
                &resolver,
                &mut references,
                diagnostics,
            );
        }
        self.variable_references = references;
    }

    /// 在任何展开发生前保存结构化编辑器需要的原始表达式。
    fn capture_variable_declarations(&mut self) {
        self.declared_env = self.env.clone();
        self.declared_task_defaults = self.task_defaults.clone();
        self.declared_profiles = self.profiles.clone();
        self.declared_task_templates = self.task_templates.clone();
        self.declared_tasks = self.tasks.clone();
    }
}

impl<'a> VariableResolver<'a> {
    /// 创建尚未解析任何声明的变量解析器。
    fn new(declarations: &'a BTreeMap<String, String>) -> Self {
        Self {
            declarations,
            resolved: BTreeMap::new(),
            failed: BTreeSet::new(),
            stack: Vec::new(),
        }
    }

    /// 校验名称并解析全部变量链，结果顺序不依赖声明顺序。
    fn resolve_all(&mut self, diagnostics: &mut Vec<ConfigDiagnostic>) {
        for name in self.declarations.keys() {
            if !super::valid_dependency_id(name) {
                diagnostics.push(diagnostic(
                    format!("vars.{name}"),
                    "变量名称只能包含 ASCII 字母、数字、点、短横线和下划线",
                ));
                self.failed.insert(name.clone());
            }
        }
        for name in self.declarations.keys() {
            self.resolve(name, diagnostics);
        }
    }

    /// 深度优先解析一个变量，并拒绝未知引用和循环链。
    fn resolve(&mut self, name: &str, diagnostics: &mut Vec<ConfigDiagnostic>) -> Option<String> {
        if let Some(value) = self.resolved.get(name) {
            return Some(value.clone());
        }
        if self.failed.contains(name) {
            return None;
        }
        if let Some(position) = self.stack.iter().position(|item| item == name) {
            let chain = self.stack[position..]
                .iter()
                .chain(std::iter::once(&name.to_owned()))
                .cloned()
                .collect::<Vec<_>>()
                .join(" -> ");
            diagnostics.push(diagnostic(
                format!("vars.{name}"),
                format!("检测到变量引用循环：{chain}"),
            ));
            self.failed.extend(self.stack[position..].iter().cloned());
            return None;
        }
        let declaration = self.declarations.get(name)?.clone();
        self.stack.push(name.to_owned());
        let path = format!("vars.{name}");
        let segments = parse_segments(&declaration, &path, diagnostics);
        let mut output = String::new();
        let mut valid = true;
        for segment in segments {
            match segment {
                Segment::Literal(value) => output.push_str(&value),
                Segment::Reference(reference) => {
                    if !self.declarations.contains_key(&reference) {
                        diagnostics.push(diagnostic(
                            &path,
                            format!("引用了不存在的变量 `{reference}`"),
                        ));
                        valid = false;
                    } else if let Some(value) = self.resolve(&reference, diagnostics) {
                        output.push_str(&value);
                    } else {
                        valid = false;
                    }
                }
            }
        }
        self.stack.pop();
        if valid && !self.failed.contains(name) {
            self.resolved.insert(name.to_owned(), output.clone());
            Some(output)
        } else {
            self.failed.insert(name.to_owned());
            None
        }
    }

    /// 展开普通配置字段，并返回该字段直接引用的变量名称。
    fn expand(
        &self,
        input: &str,
        path: &str,
        diagnostics: &mut Vec<ConfigDiagnostic>,
    ) -> (String, BTreeSet<String>) {
        let segments = parse_segments(input, path, diagnostics);
        let mut output = String::new();
        let mut references = BTreeSet::new();
        for segment in segments {
            match segment {
                Segment::Literal(value) => output.push_str(&value),
                Segment::Reference(reference) => {
                    references.insert(reference.clone());
                    if let Some(value) = self.resolved.get(&reference) {
                        output.push_str(value);
                    } else {
                        diagnostics.push(diagnostic(
                            path,
                            format!("变量 `{reference}` 不存在或无法解析"),
                        ));
                    }
                }
            }
        }
        (output, references)
    }
}

/// 展开一个字符串并记录非空的直接变量引用集合。
fn expand_value(
    value: &mut String,
    path: &str,
    resolver: &VariableResolver<'_>,
    references: &mut BTreeMap<String, BTreeSet<String>>,
    diagnostics: &mut Vec<ConfigDiagnostic>,
) {
    if !value.contains("${vars.") {
        return;
    }
    let (expanded, used) = resolver.expand(value, path, diagnostics);
    *value = expanded;
    if !used.is_empty() {
        references.insert(path.to_owned(), used);
    }
}

/// 展开字符串 map 的全部值，键保持字面量以维持稳定身份。
fn expand_map(
    values: &mut BTreeMap<String, String>,
    path: &str,
    resolver: &VariableResolver<'_>,
    references: &mut BTreeMap<String, BTreeSet<String>>,
    diagnostics: &mut Vec<ConfigDiagnostic>,
) {
    for (key, value) in values {
        expand_value(
            value,
            &format!("{path}.{key}"),
            resolver,
            references,
            diagnostics,
        );
    }
}

/// 展开项目或 profile 默认层允许引用变量的路径与环境字段。
fn expand_defaults(
    defaults: &mut RawTaskDefaults,
    path: &str,
    resolver: &VariableResolver<'_>,
    references: &mut BTreeMap<String, BTreeSet<String>>,
    diagnostics: &mut Vec<ConfigDiagnostic>,
) {
    if let Some(cwd) = defaults.cwd.as_mut() {
        let mut value = cwd.to_string_lossy().into_owned();
        expand_value(
            &mut value,
            &format!("{path}.cwd"),
            resolver,
            references,
            diagnostics,
        );
        *cwd = PathBuf::from(value);
    }
    expand_map(
        &mut defaults.env,
        &format!("{path}.env"),
        resolver,
        references,
        diagnostics,
    );
}

/// 展开 Task 或模板中明确属于执行配置的字符串字段。
fn expand_task(
    task: &mut RawTask,
    path: &str,
    resolver: &VariableResolver<'_>,
    references: &mut BTreeMap<String, BTreeSet<String>>,
    diagnostics: &mut Vec<ConfigDiagnostic>,
) {
    if let Some(command) = task.command.as_mut() {
        match command {
            RawCommand::Program(value) => expand_value(
                value,
                &format!("{path}.command"),
                resolver,
                references,
                diagnostics,
            ),
            RawCommand::Argv(values) => {
                for (index, value) in values.iter_mut().enumerate() {
                    expand_value(
                        value,
                        &format!("{path}.command.{index}"),
                        resolver,
                        references,
                        diagnostics,
                    );
                }
            }
        }
    }
    if let Some(args) = task.args.as_mut() {
        for (index, value) in args.iter_mut().enumerate() {
            expand_value(
                value,
                &format!("{path}.args.{index}"),
                resolver,
                references,
                diagnostics,
            );
        }
    }
    expand_task_paths(task, path, resolver, references, diagnostics);
    expand_map(
        &mut task.env,
        &format!("{path}.env"),
        resolver,
        references,
        diagnostics,
    );
    if let Some(healthcheck) = task.healthcheck.as_mut() {
        healthcheck.map_strings(&format!("{path}.healthcheck"), &mut |field, input| {
            let mut value = input.to_owned();
            expand_value(&mut value, field, resolver, references, diagnostics);
            value
        });
    }
}

/// 展开 Task 的工作目录与环境文件路径。
fn expand_task_paths(
    task: &mut RawTask,
    path: &str,
    resolver: &VariableResolver<'_>,
    references: &mut BTreeMap<String, BTreeSet<String>>,
    diagnostics: &mut Vec<ConfigDiagnostic>,
) {
    for (field, configured) in [("cwd", &mut task.cwd), ("env_file", &mut task.env_file)] {
        let Some(value) = configured.as_mut() else {
            continue;
        };
        let mut text = value.to_string_lossy().into_owned();
        expand_value(
            &mut text,
            &format!("{path}.{field}"),
            resolver,
            references,
            diagnostics,
        );
        *value = PathBuf::from(text);
    }
}

/// 把字符串解析为变量引用片段；`$${vars.NAME}` 产生字面量引用。
fn parse_segments(
    input: &str,
    path: &str,
    diagnostics: &mut Vec<ConfigDiagnostic>,
) -> Vec<Segment> {
    let mut segments = Vec::new();
    let mut cursor = 0_usize;
    while let Some(relative) = input[cursor..].find("${vars.") {
        let start = cursor + relative;
        let Some(close_relative) = input[start + 7..].find('}') else {
            push_literal(&mut segments, &input[cursor..start]);
            diagnostics.push(diagnostic(path, "变量引用缺少右花括号 `}`"));
            push_literal(&mut segments, &input[start..]);
            return segments;
        };
        let end = start + 7 + close_relative;
        if start > cursor && input.as_bytes()[start - 1] == b'$' {
            push_literal(&mut segments, &input[cursor..start - 1]);
            push_literal(&mut segments, &input[start..=end]);
            cursor = end + 1;
            continue;
        }
        push_literal(&mut segments, &input[cursor..start]);
        let name = &input[start + 7..end];
        if name.is_empty() || !super::valid_dependency_id(name) {
            diagnostics.push(diagnostic(path, format!("变量引用名称 `{name}` 格式无效")));
        }
        segments.push(Segment::Reference(name.to_owned()));
        cursor = end + 1;
    }
    push_literal(&mut segments, &input[cursor..]);
    segments
}

/// 合并相邻字面量片段，避免为普通文本产生大量小分配。
fn push_literal(segments: &mut Vec<Segment>, value: &str) {
    if value.is_empty() {
        return;
    }
    if let Some(Segment::Literal(existing)) = segments.last_mut() {
        existing.push_str(value);
    } else {
        segments.push(Segment::Literal(value.to_owned()));
    }
}
