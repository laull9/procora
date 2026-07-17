use super::{
    config_form::{FormConfig, FormTask},
    config_form_dialog::{
        DialogField, args_text, dependencies_text, field, map_text, optional, parse_args,
        parse_dependencies, parse_duration, parse_i32_list, parse_map, parse_u32, replace_entry,
        required_value,
    },
};
use crate::config::{ValueOrigin, split_command_text};

/// 构造 Task 弹窗，并用空值或 inherit 明确表示没有本地覆盖。
pub(super) fn fields(original: Option<&str>, task: &FormTask) -> Vec<DialogField> {
    vec![
        field("Task 名称", original.unwrap_or(""), &[]),
        field(
            "命令覆盖（空=继承，可内嵌参数）",
            explicit_text(task, "command", Some(&task.command)),
            &[],
        ),
        field(
            "参数覆盖（JSON 数组）",
            &if task.explicit("args") {
                args_text(&task.args)
            } else {
                "[]".to_owned()
            },
            &[],
        ),
        field(
            "工作目录覆盖（空=继承）",
            explicit_text(task, "cwd", task.cwd.as_deref()),
            &[],
        ),
        field(
            "环境文件（可空）",
            task.env_file.as_deref().unwrap_or(""),
            &[],
        ),
        field(
            "环境变量（JSON 对象或 KEY=VALUE）",
            &map_text(&task.env),
            &[],
        ),
        field(
            "依赖（task:条件，逗号分隔）",
            &dependencies_text(&task.depends_on),
            &[],
        ),
        field(
            "成功退出码覆盖（空=继承）",
            &explicit_list(task, "success_exit_codes", &task.success_exit_codes),
            &[],
        ),
        field(
            "重启策略覆盖",
            if task.explicit("restart") {
                &task.restart
            } else {
                "inherit"
            },
            &["inherit", "never", "on-failure", "always"],
        ),
        field(
            "重启等待覆盖（如 750ms/5s，空=继承）",
            &explicit_duration(task, "restart_delay_ms", task.restart_delay_ms),
            &[],
        ),
        field(
            "最大重启次数覆盖（空=继承）",
            &explicit_number(task, "max_restarts", &task.max_restarts),
            &[],
        ),
        field(
            "计数重置覆盖（如 1m，空=继承）",
            &explicit_duration(task, "restart_reset_after_ms", task.restart_reset_after_ms),
            &[],
        ),
        field(
            "停止超时覆盖（如 5s，空=继承）",
            &explicit_duration(task, "shutdown_timeout_ms", task.shutdown_timeout_ms),
            &[],
        ),
        field(
            "继承模板（可空）",
            task.extends.as_deref().unwrap_or(""),
            &[],
        ),
    ]
}

/// 校验 Task 弹窗并显式设置或清除每个可继承字段的覆盖来源。
pub(super) fn commit(
    original: Option<&str>,
    baseline: &FormTask,
    fields: &[DialogField],
    config: &mut FormConfig,
) -> Result<(), String> {
    let name = fields[0].value.trim();
    required_value(name, "Task 名称")?;
    let mut task = baseline.clone();
    commit_command_and_template(baseline, fields, config, &mut task)?;
    task.env_file = optional(&fields[4].value);
    task.env = parse_map(&fields[5].value, "环境变量")?;
    task.depends_on = parse_dependencies(&fields[6].value)?;
    apply_inheritable_fields(fields, &mut task)?;

    mark_local_changes(baseline, &mut task);
    config.task_defaults.apply_to(&mut task);
    replace_entry(&mut config.tasks, original, name, task, "Task")
}

/// 提交命令、参数和模板引用，并维持继承来源的准确性。
fn commit_command_and_template(
    baseline: &FormTask,
    fields: &[DialogField],
    config: &FormConfig,
    task: &mut FormTask,
) -> Result<(), String> {
    let template = optional(&fields[13].value);
    if let Some(template) = template.as_deref()
        && !config.has_template(template)
    {
        return Err(format!("Task 模板 `{template}` 不存在"));
    }
    task.extends = template;
    let command_text = optional(&fields[1].value);
    let separate_args = parse_args(&fields[2].value, "参数")?;
    if let Some(command_text) = command_text {
        let parse_embedded = separate_args.is_empty()
            && (!baseline.explicit("args") || command_text != baseline.command);
        if parse_embedded {
            (task.command, task.args) = split_command_text(&command_text)?;
        } else {
            task.command = command_text;
            task.args = separate_args;
        }
        task.origins
            .fields
            .insert("command".to_owned(), ValueOrigin::Task);
        task.origins
            .fields
            .insert("args".to_owned(), ValueOrigin::Task);
    } else if task.extends.is_none() {
        return Err("命令或继承模板至少需要配置一个".to_owned());
    } else if !separate_args.is_empty()
        || (baseline.explicit("args") && !baseline.explicit("command"))
    {
        task.args = separate_args;
        task.origins
            .fields
            .insert("args".to_owned(), ValueOrigin::Task);
    } else {
        task.origins
            .fields
            .insert("command".to_owned(), ValueOrigin::TaskTemplate);
        task.origins
            .fields
            .insert("args".to_owned(), ValueOrigin::TaskTemplate);
    }
    Ok(())
}

/// 提交可由默认值或模板提供的其余 Task 字段。
fn apply_inheritable_fields(fields: &[DialogField], task: &mut FormTask) -> Result<(), String> {
    set_optional(task, "cwd", optional(&fields[3].value), |task, value| {
        task.cwd = value;
    });
    set_optional(
        task,
        "success_exit_codes",
        optional(&fields[7].value)
            .map(|value| parse_i32_list(&value, "成功退出码"))
            .transpose()?,
        |task, value| {
            if let Some(value) = value {
                task.success_exit_codes = value;
            }
        },
    );
    set_optional(
        task,
        "restart",
        (fields[8].value != "inherit").then(|| fields[8].value.clone()),
        |task, value| {
            if let Some(value) = value {
                task.restart = value;
            }
        },
    );
    set_optional(
        task,
        "restart_delay_ms",
        optional(&fields[9].value)
            .map(|value| parse_duration(&value, "重启等待"))
            .transpose()?,
        |task, value| {
            if let Some(value) = value {
                task.restart_delay_ms = value;
            }
        },
    );
    set_optional(
        task,
        "max_restarts",
        optional(&fields[10].value)
            .map(|value| parse_u32(&value, "最大重启次数"))
            .transpose()?,
        |task, value| {
            if let Some(value) = value {
                task.max_restarts = value;
            }
        },
    );
    set_optional(
        task,
        "restart_reset_after_ms",
        optional(&fields[11].value)
            .map(|value| parse_duration(&value, "重启计数重置"))
            .transpose()?,
        |task, value| {
            if let Some(value) = value {
                task.restart_reset_after_ms = value;
            }
        },
    );
    set_optional(
        task,
        "shutdown_timeout_ms",
        optional(&fields[12].value)
            .map(|value| parse_duration(&value, "停止超时"))
            .transpose()?,
        |task, value| {
            if let Some(value) = value {
                task.shutdown_timeout_ms = value;
            }
        },
    );
    Ok(())
}

/// 返回只有 Task 显式声明时才显示的字符串。
fn explicit_text<'a>(task: &FormTask, field: &str, value: Option<&'a str>) -> &'a str {
    if task.explicit(field) {
        value.unwrap_or("")
    } else {
        ""
    }
}

/// 返回只有 Task 显式声明时才显示的整数数组。
fn explicit_list(task: &FormTask, field: &str, value: &[i32]) -> String {
    if task.explicit(field) {
        serde_json::to_string(value).expect("整数数组序列化不会失败")
    } else {
        String::new()
    }
}

/// 返回只有 Task 显式声明时才显示的数值。
fn explicit_number(task: &FormTask, field: &str, value: &impl ToString) -> String {
    if task.explicit(field) {
        value.to_string()
    } else {
        String::new()
    }
}

/// 返回只有 Task 显式声明时才显示的可读时长。
fn explicit_duration(task: &FormTask, field: &str, value: u64) -> String {
    if task.explicit(field) {
        crate::config::format_duration(value)
    } else {
        String::new()
    }
}

/// 根据输入是否为空切换 Task 显式或继承来源。
fn set_optional<T>(
    task: &mut FormTask,
    field: &str,
    value: Option<T>,
    assign: impl FnOnce(&mut FormTask, Option<T>),
) {
    let origin = if value.is_some() {
        ValueOrigin::Task
    } else {
        ValueOrigin::BuiltIn
    };
    assign(task, value);
    task.origins.fields.insert(field.to_owned(), origin);
}

/// 标记不参与默认层的 Task 本地字段变化。
fn mark_local_changes(previous: &FormTask, task: &mut FormTask) {
    for (field, changed) in [
        ("env_file", previous.env_file != task.env_file),
        ("depends_on", previous.depends_on != task.depends_on),
    ] {
        if changed || field == "command" {
            task.origins
                .fields
                .insert(field.to_owned(), ValueOrigin::Task);
        }
    }
    for key in task.env.keys() {
        task.origins.env.insert(key.clone(), ValueOrigin::Task);
    }
}
