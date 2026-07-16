//! 本机托管服务名称约束测试。

use procora::core::{ServiceName, ServiceNameError};

#[test]
fn 接受可稳定用于命令定位的名称() {
    let name: ServiceName = "api.worker-1".parse().unwrap();
    assert_eq!(name.as_str(), "api.worker-1");
}

#[test]
fn 拒绝空白和路径字符() {
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
