//! 平台基础能力探测测试。

use procora_platform::{PlatformKind, capabilities, data_dir};

#[test]
fn 支持平台声明受管进程树能力() {
    let capabilities = capabilities();

    assert!(matches!(
        capabilities.platform,
        PlatformKind::Linux | PlatformKind::MacOs | PlatformKind::Windows
    ));
    assert!(capabilities.managed_process_tree);
    assert!(data_dir().is_some());
}
