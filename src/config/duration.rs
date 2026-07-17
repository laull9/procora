use std::fmt;

use serde::{Deserialize, Deserializer, Serializer, de::Visitor};

/// 把带单位的紧凑时长解析为毫秒。
pub(crate) fn parse_duration(value: &str) -> Result<u64, String> {
    if value.is_empty() {
        return Err("时长不能为空".to_owned());
    }
    if value.bytes().any(|byte| byte.is_ascii_whitespace()) {
        return Err("时长不能包含空白，请使用 `1m30s` 形式".to_owned());
    }

    let bytes = value.as_bytes();
    let mut offset = 0;
    let mut total = 0_u64;
    let mut previous_rank = 4_u8;
    while offset < bytes.len() {
        let number_start = offset;
        while offset < bytes.len() && bytes[offset].is_ascii_digit() {
            offset += 1;
        }
        if number_start == offset {
            return Err(format!("时长 `{value}` 需要由整数和单位组成"));
        }
        let amount = value[number_start..offset]
            .parse::<u64>()
            .map_err(|_| format!("时长 `{value}` 超出支持范围"))?;
        let (rank, multiplier, unit_len) = match bytes.get(offset..) {
            Some(rest) if rest.starts_with(b"ms") => (0, 1_u64, 2),
            Some(rest) if rest.starts_with(b"s") => (1, 1_000_u64, 1),
            Some(rest) if rest.starts_with(b"m") => (2, 60_000_u64, 1),
            Some(rest) if rest.starts_with(b"h") => (3, 3_600_000_u64, 1),
            _ => {
                return Err(format!("时长 `{value}` 缺少有效单位，仅支持 h、m、s、ms"));
            }
        };
        if rank >= previous_rank {
            return Err(format!(
                "时长 `{value}` 的单位必须按 h、m、s、ms 降序且不能重复"
            ));
        }
        previous_rank = rank;
        offset += unit_len;
        total = total
            .checked_add(
                amount
                    .checked_mul(multiplier)
                    .ok_or_else(|| format!("时长 `{value}` 超出支持范围"))?,
            )
            .ok_or_else(|| format!("时长 `{value}` 超出支持范围"))?;
    }
    Ok(total)
}

/// 把毫秒格式化为稳定、紧凑的带单位时长。
pub(crate) fn format_duration(milliseconds: u64) -> String {
    if milliseconds != 0 && milliseconds.is_multiple_of(3_600_000) {
        format!("{}h", milliseconds / 3_600_000)
    } else if milliseconds != 0 && milliseconds.is_multiple_of(60_000) {
        format!("{}m", milliseconds / 60_000)
    } else if milliseconds != 0 && milliseconds.is_multiple_of(1_000) {
        format!("{}s", milliseconds / 1_000)
    } else {
        format!("{milliseconds}ms")
    }
}

/// 兼容旧整数毫秒和新带单位字符串的 Serde 值。
struct DurationValue(u64);

impl<'de> Deserialize<'de> for DurationValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(DurationVisitor)
    }
}

/// 访问不同配置格式中的时长标量。
struct DurationVisitor;

impl Visitor<'_> for DurationVisitor {
    type Value = DurationValue;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("非负整数毫秒或带 h、m、s、ms 单位的时长字符串")
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
        Ok(DurationValue(value))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        u64::try_from(value)
            .map(DurationValue)
            .map_err(|_| E::custom("时长毫秒数不能为负数"))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        parse_duration(value).map(DurationValue).map_err(E::custom)
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        self.visit_str(&value)
    }
}

/// 反序列化必填或带默认值的时长字段。
pub(crate) fn deserialize_duration<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: Deserializer<'de>,
{
    DurationValue::deserialize(deserializer).map(|value| value.0)
}

/// 反序列化可选时长字段。
pub(crate) fn deserialize_optional_duration<'de, D>(
    deserializer: D,
) -> Result<Option<u64>, D::Error>
where
    D: Deserializer<'de>,
{
    Option::<DurationValue>::deserialize(deserializer).map(|value| value.map(|value| value.0))
}

/// 把必填时长稳定序列化为带单位字符串。
#[allow(clippy::trivially_copy_pass_by_ref)]
pub(crate) fn serialize_duration<S>(value: &u64, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_str(&format_duration(*value))
}

/// 把可选时长稳定序列化为带单位字符串。
#[allow(clippy::ref_option)]
pub(crate) fn serialize_optional_duration<S>(
    value: &Option<u64>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    match value {
        Some(value) => serializer.serialize_some(&format_duration(*value)),
        None => serializer.serialize_none(),
    }
}
