//! 本机托管服务名称约束测试。

use procora::core::{ServiceName, ServiceNameError};

#[test]
// 接受可稳定用于命令定位的名称。
fn service_name_accepts_stable_command_identifiers() {
    let name: ServiceName = "api.worker-1".parse().unwrap();
    assert_eq!(name.as_str(), "api.worker-1");
}

#[test]
// 拒绝空白和路径字符。
fn service_name_rejects_whitespace_and_path_characters() {
    assert_eq!("".parse::<ServiceName>(), Err(ServiceNameError::Empty));
    assert!(matches!(
        "api/service".parse::<ServiceName>(),
        Err(ServiceNameError::InvalidCharacters(_))
    ));
    assert!(matches!(
        "api service".parse::<ServiceName>(),
        Err(ServiceNameError::InvalidCharacters(_))
    ));
}
