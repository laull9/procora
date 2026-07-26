use serde::Deserialize;

/// 从整数或带单位文本读取字节数。
pub(crate) fn deserialize_byte_size<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum ByteSizeInput {
        Integer(u64),
        Text(String),
    }

    match ByteSizeInput::deserialize(deserializer)? {
        ByteSizeInput::Integer(bytes) => Ok(bytes),
        ByteSizeInput::Text(value) => parse_byte_size(&value).map_err(serde::de::Error::custom),
    }
}

/// 解析十进制 SI 与二进制 IEC 大小单位。
fn parse_byte_size(value: &str) -> Result<u64, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err("字节大小不能为空".to_owned());
    }
    let number_end = value
        .char_indices()
        .take_while(|(_, character)| character.is_ascii_digit() || *character == '.')
        .last()
        .map_or(0, |(index, character)| index + character.len_utf8());
    let number = &value[..number_end];
    let unit = value[number_end..].trim().to_ascii_uppercase();
    let factor = match unit.as_str() {
        "" | "B" => 1_u128,
        "KB" => 1_000,
        "MB" => 1_000_000,
        "GB" => 1_000_000_000,
        "TB" => 1_000_000_000_000,
        "KIB" => 1_u128 << 10,
        "MIB" => 1_u128 << 20,
        "GIB" => 1_u128 << 30,
        "TIB" => 1_u128 << 40,
        _ => {
            return Err(
                "不支持的字节单位；可使用 B、KB、MB、GB、TB 或 KiB、MiB、GiB、TiB".to_owned(),
            );
        }
    };
    let (whole, fraction) = number
        .split_once('.')
        .map_or((number, None), |(whole, fraction)| (whole, Some(fraction)));
    if whole.is_empty()
        || !whole.bytes().all(|byte| byte.is_ascii_digit())
        || fraction.is_some_and(|value| {
            value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit())
        })
    {
        return Err("字节大小应为非负整数或小数，例如 `20MB`、`1.5GiB`".to_owned());
    }
    let whole = whole
        .parse::<u128>()
        .map_err(|_| "字节大小数值过大".to_owned())?;
    let (numerator, scale) = if let Some(fraction) = fraction {
        let scale = 10_u128
            .checked_pow(u32::try_from(fraction.len()).map_err(|_| "字节大小精度过高".to_owned())?)
            .ok_or_else(|| "字节大小精度过高".to_owned())?;
        let fraction = fraction
            .parse::<u128>()
            .map_err(|_| "字节大小数值过大".to_owned())?;
        (
            whole
                .checked_mul(scale)
                .and_then(|value| value.checked_add(fraction))
                .ok_or_else(|| "字节大小数值过大".to_owned())?,
            scale,
        )
    } else {
        (whole, 1)
    };
    let scaled = numerator
        .checked_mul(factor)
        .ok_or_else(|| "字节大小数值过大".to_owned())?;
    if scaled % scale != 0 {
        return Err("字节大小换算后必须是完整字节".to_owned());
    }
    u64::try_from(scaled / scale).map_err(|_| "字节大小超过 u64 范围".to_owned())
}

#[cfg(test)]
mod tests {
    use super::parse_byte_size;

    // 常用十进制和二进制单位都能精确换算。
    #[test]
    fn parses_human_readable_units() {
        assert_eq!(parse_byte_size("20MB").unwrap(), 20_000_000);
        assert_eq!(parse_byte_size("1 GB").unwrap(), 1_000_000_000);
        assert_eq!(parse_byte_size("1.5GiB").unwrap(), 1_610_612_736);
        assert_eq!(parse_byte_size("2mib").unwrap(), 2_097_152);
    }

    // 无效单位、小数和溢出会返回可操作错误。
    #[test]
    fn rejects_invalid_or_inexact_sizes() {
        assert!(parse_byte_size("1XB").is_err());
        assert!(parse_byte_size(".5MB").is_err());
        assert!(parse_byte_size("0.1B").is_err());
        assert!(parse_byte_size("999999999999999999999TB").is_err());
    }
}
