use std::collections::BTreeMap;

use super::config_form_value::map_text;

/// 表单字段的可编辑值和可选枚举值。
#[derive(Clone, Debug)]
pub(super) struct DialogField {
    pub(super) label: &'static str,
    pub(super) value: String,
    pub(super) choices: Vec<String>,
    pub(super) cursor: usize,
    pub(super) kind: DialogFieldKind,
}

/// 弹窗字段采用的输入控件类型。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DialogFieldKind {
    Text,
    Choice,
    Map,
    Directory,
}

/// 创建一个普通文本或固定选项字段。
pub(super) fn field(
    label: &'static str,
    value: &str,
    choices: &'static [&'static str],
) -> DialogField {
    DialogField {
        label,
        value: value.to_owned(),
        choices: choices.iter().map(|value| (*value).to_owned()).collect(),
        cursor: value.chars().count(),
        kind: if choices.is_empty() {
            DialogFieldKind::Text
        } else {
            DialogFieldKind::Choice
        },
    }
}

/// 创建运行期生成选项的弹窗字段。
pub(super) fn choice_field(label: &'static str, value: &str, choices: Vec<String>) -> DialogField {
    DialogField {
        label,
        value: value.to_owned(),
        choices,
        cursor: value.chars().count(),
        kind: DialogFieldKind::Choice,
    }
}

/// 创建使用键值表子弹窗编辑的映射字段。
pub(super) fn map_field(label: &'static str, values: &BTreeMap<String, String>) -> DialogField {
    let value = map_text(values);
    DialogField {
        label,
        cursor: value.chars().count(),
        value,
        choices: Vec::new(),
        kind: DialogFieldKind::Map,
    }
}

/// 创建既可手输也可按 F5 浏览的目录字段。
pub(super) fn directory_field(label: &'static str, value: &str) -> DialogField {
    DialogField {
        label,
        value: value.to_owned(),
        choices: Vec::new(),
        cursor: value.chars().count(),
        kind: DialogFieldKind::Directory,
    }
}

/// 把字符序号转换为 UTF-8 字节位置。
pub(super) fn char_to_byte(value: &str, index: usize) -> usize {
    value
        .char_indices()
        .nth(index)
        .map_or(value.len(), |(byte, _)| byte)
}
