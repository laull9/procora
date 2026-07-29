//! 外部程序输出的兼容解码。

use std::borrow::Cow;

/// 以 UTF-8 优先、GB18030 兜底的顺序解码完整外部程序输出。
///
/// Windows 路径继续由 `Path`/`OsStr` 保持 UTF-16；本函数只用于旧控制台、
/// SSH 诊断和子进程日志等字节输出，不得用于 JSON、MCP 或部署协议正文。
pub fn decode_external_output(bytes: &[u8]) -> Cow<'_, str> {
    if let Ok(text) = std::str::from_utf8(bytes) {
        return Cow::Borrowed(text);
    }
    encoding_rs::GB18030.decode_without_bom_handling(bytes).0
}

#[cfg(test)]
mod tests {
    use super::decode_external_output;

    // 已经是UTF-8的中文必须原样保留，不能被误判为本地代码页。
    #[test]
    fn utf8_output_has_priority() {
        let input = "路径：C:\\工具\\服务";
        assert_eq!(decode_external_output(input.as_bytes()), input);
    }

    // Windows代码页936常见中文诊断可通过GB18030超集恢复。
    #[test]
    fn gbk_output_is_decoded() {
        let expected = "不是内部或外部命令";
        let (encoded, _, had_errors) = encoding_rs::GBK.encode(expected);
        assert!(!had_errors);
        assert_eq!(decode_external_output(&encoded), expected);
    }

    // GB18030四字节扩展字符不会退化成替换字符。
    #[test]
    fn gb18030_four_byte_output_is_decoded() {
        let expected = "扩展字符：𠀀";
        let (encoded, _, had_errors) = encoding_rs::GB18030.encode(expected);
        assert!(!had_errors);
        assert_eq!(decode_external_output(&encoded), expected);
    }

    // 截断或非法字节只影响损坏片段，不能导致诊断路径崩溃。
    #[test]
    fn malformed_output_uses_replacement_character() {
        let decoded = decode_external_output(&[0x81]);
        assert_eq!(decoded, "\u{fffd}");
    }
}
