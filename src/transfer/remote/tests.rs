use super::{
    LoginFailure, classify_login_failure, remote_command_missing, transfer_protocol_incompatible,
    validate_remote_bin,
};

// 远端可执行文件兼容 Unix 与 Windows 的无空格绝对路径。
#[test]
fn remote_binary_accepts_cross_platform_paths() {
    assert!(validate_remote_bin("/home/demo/.local/bin/procora").is_ok());
    assert!(validate_remote_bin("C:/Tools/procora.exe").is_ok());
    assert!(validate_remote_bin(r"C:\Tools\procora.exe").is_ok());
    assert!(validate_remote_bin("C:/工具/进程管理/procora.exe").is_ok());
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

// GBK编码的Windows命令缺失诊断解码后仍能触发远端路径发现。
#[test]
fn gbk_windows_missing_command_is_recognized() {
    let (encoded, _, had_errors) =
        encoding_rs::GBK.encode("'procora' 不是内部或外部命令，也不是可运行的程序");
    assert!(!had_errors);
    let message = crate::platform::decode_external_output(&encoded);
    assert!(remote_command_missing(Some(1), &message));
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

// 只有认证失败与未知主机允许进入交互回退，网络错误和主机密钥变更直接失败。
#[test]
fn ssh_login_failures_are_safely_classified() {
    assert_eq!(
        classify_login_failure(Some(255), b"Permission denied (publickey,password)."),
        LoginFailure::Authentication
    );
    assert_eq!(
        classify_login_failure(
            Some(255),
            b"No ED25519 host key is known and you have requested strict checking.\nHost key verification failed."
        ),
        LoginFailure::HostKey
    );
    assert_eq!(
        classify_login_failure(
            Some(255),
            b"WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED!"
        ),
        LoginFailure::None
    );
    assert_eq!(
        classify_login_failure(Some(255), b"Connection refused"),
        LoginFailure::None
    );
}
