use super::{remote_command_missing, transfer_protocol_incompatible, validate_remote_bin};

// 远端可执行文件兼容 Unix 与 Windows 的无空格绝对路径。
#[test]
fn remote_binary_accepts_cross_platform_paths() {
    assert!(validate_remote_bin("/home/demo/.local/bin/procora").is_ok());
    assert!(validate_remote_bin("C:/Tools/procora.exe").is_ok());
    assert!(validate_remote_bin(r"C:\Tools\procora.exe").is_ok());
}

// 远端可执行文件仍拒绝会改变 shell 命令边界的字符。
#[test]
fn remote_binary_rejects_shell_metacharacters() {
    assert!(validate_remote_bin("procora;whoami").is_err());
    assert!(validate_remote_bin("C:/Program Files/procora.exe").is_err());
}

// Windows shell 不依赖 Unix 退出码也能识别命令缺失。
#[test]
fn windows_shell_missing_command_is_recognized() {
    assert!(remote_command_missing(
        Some(1),
        "CommandNotFoundException: procora was not found"
    ));
    assert!(remote_command_missing(
        Some(1),
        "'procora' is not recognized as an internal or external command"
    ));
    assert!(!remote_command_missing(Some(1), "Permission denied"));
}

// 协议拒绝可与普通认证或远端命令错误区分。
#[test]
fn transfer_protocol_rejection_is_recognized() {
    assert!(transfer_protocol_incompatible(
        "错误：不支持上传协议版本 2，当前为 1".as_bytes()
    ));
    assert!(transfer_protocol_incompatible(
        "unsupported transfer protocol 3".as_bytes()
    ));
    assert!(!transfer_protocol_incompatible(
        "Permission denied".as_bytes()
    ));
}
