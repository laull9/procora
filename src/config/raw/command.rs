use std::fmt;

use serde::de::{Deserializer, SeqAccess, Visitor};
use serde::{Deserialize, Serialize, Serializer};

use super::{ConfigDiagnostic, diagnostic};

/// 原始命令既接受命令行文本，也接受精确且紧凑的 argv 数组。
#[derive(Clone, Debug)]
pub(super) enum RawCommand {
    /// 未声明 args 时按命令行文本拆分，否则兼容旧版程序名称。
    Program(String),
    /// 第一个元素是程序，其余元素是完整参数数组。
    Argv(Vec<String>),
}

impl Serialize for RawCommand {
    /// 保留字符串命令或精确 argv 数组的原始表示类型。
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Program(command) => serializer.serialize_str(command),
            Self::Argv(argv) => argv.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for RawCommand {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(RawCommandVisitor)
    }
}

/// 跨 YAML、TOML 和 JSON 保持一致类型错误的命令访问器。
struct RawCommandVisitor;

impl<'de> Visitor<'de> for RawCommandVisitor {
    type Value = RawCommand;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("程序字符串或非空字符串 argv 数组")
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E> {
        Ok(RawCommand::Program(value.to_owned()))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
        Ok(RawCommand::Program(value))
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = Vec::with_capacity(sequence.size_hint().unwrap_or(0));
        while let Some(value) = sequence.next_element::<CommandArgument>()? {
            values.push(value.0);
        }
        Ok(RawCommand::Argv(values))
    }
}

/// 禁止 YAML 等前端把数字或布尔值隐式转换为 argv 字符串。
struct CommandArgument(String);

impl<'de> Deserialize<'de> for CommandArgument {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(CommandArgumentVisitor)
    }
}

/// 只接受真正字符串标量的 argv 元素访问器。
struct CommandArgumentVisitor;

impl Visitor<'_> for CommandArgumentVisitor {
    type Value = CommandArgument;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("argv 字符串")
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E> {
        Ok(CommandArgument(value.to_owned()))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
        Ok(CommandArgument(value))
    }
}

/// 把中立的命令行文本拆成程序和参数，不执行 shell 展开或运算符。
pub(crate) fn split_text(value: &str) -> Result<(String, Vec<String>), String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut quote = None;
    let mut started = false;
    let mut characters = value.chars().peekable();

    while let Some(character) = characters.next() {
        match quote {
            Some(delimiter) if character == delimiter => quote = None,
            Some('"') if character == '\\' && characters.peek() == Some(&'"') => {
                characters.next();
                word.push('"');
                started = true;
            }
            None if character == '\'' || character == '"' => {
                quote = Some(character);
                started = true;
            }
            None if character == '\\' => {
                if characters
                    .peek()
                    .is_some_and(|next| next.is_whitespace() || matches!(next, '\'' | '"'))
                {
                    word.push(characters.next().expect("已确认存在下一个字符"));
                } else {
                    word.push(character);
                }
                started = true;
            }
            None if character.is_whitespace() => {
                if started {
                    words.push(std::mem::take(&mut word));
                    started = false;
                }
            }
            Some(_) | None => {
                word.push(character);
                started = true;
            }
        }
    }

    if let Some(delimiter) = quote {
        let label = if delimiter == '\'' {
            "单引号"
        } else {
            "双引号"
        };
        return Err(format!("命令文本存在未闭合的{label}"));
    }
    if started {
        words.push(word);
    }
    if words.is_empty() || words[0].trim().is_empty() {
        return Err("命令不能为空".to_owned());
    }

    let command = words.remove(0);
    Ok((command, words))
}

/// 把命令行文本、兼容旧写法或 argv 简写展开成统一程序与参数。
pub(super) fn normalize(
    command: Option<RawCommand>,
    separate_args: Option<Vec<String>>,
    task_path: &str,
    diagnostics: &mut Vec<ConfigDiagnostic>,
) -> (String, Vec<String>) {
    match command {
        Some(RawCommand::Program(command)) if separate_args.is_some() => {
            if command.trim().is_empty() {
                diagnostics.push(diagnostic(format!("{task_path}.command"), "命令不能为空"));
            }
            (command, separate_args.unwrap_or_default())
        }
        Some(RawCommand::Program(command)) => match split_text(&command) {
            Ok(normalized) => normalized,
            Err(message) => {
                diagnostics.push(diagnostic(format!("{task_path}.command"), message));
                (String::new(), Vec::new())
            }
        },
        Some(RawCommand::Argv(mut command_parts)) => {
            if separate_args.as_ref().is_some_and(|args| !args.is_empty()) {
                diagnostics.push(diagnostic(
                    format!("{task_path}.args"),
                    "command 使用 argv 数组时不能再声明 args",
                ));
            }
            if command_parts.is_empty() {
                diagnostics.push(diagnostic(
                    format!("{task_path}.command"),
                    "argv 数组至少需要一个程序元素",
                ));
                return (String::new(), Vec::new());
            }
            let command = command_parts.remove(0);
            if command.trim().is_empty() {
                diagnostics.push(diagnostic(
                    format!("{task_path}.command.0"),
                    "程序元素不能为空",
                ));
            }
            (command, command_parts)
        }
        None => {
            diagnostics.push(diagnostic(format!("{task_path}.command"), "缺少必需字段"));
            (String::new(), separate_args.unwrap_or_default())
        }
    }
}
