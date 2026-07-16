//! 发布工作流依赖与失败恢复契约测试。

/// 发布工作流文本，用于锁定发布依赖与失败恢复契约。
const RELEASE_WORKFLOW: &str = include_str!("../.github/workflows/release.yml");

/// CI 工作流文本，用于锁定通用检出动作的运行时版本。
const CI_WORKFLOW: &str = include_str!("../.github/workflows/ci.yml");

/// 安全工作流文本，用于锁定审计动作的运行时版本。
const SECURITY_WORKFLOW: &str = include_str!("../.github/workflows/security.yml");

/// 浸泡测试工作流文本，用于锁定通用检出动作的运行时版本。
const SOAK_WORKFLOW: &str = include_str!("../.github/workflows/soak.yml");

/// Node 24 版 checkout 的固定提交。
const CHECKOUT_NODE24: &str = "actions/checkout@9c091bb21b7c1c1d1991bb908d89e4e9dddfe3e0";

/// Node 24 版 artifact 上传动作的固定提交。
const UPLOAD_NODE24: &str = "actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a";

/// 校验全部工作流不再使用旧版 checkout。
#[test]
// 全部工作流使用node24版checkout。
fn all_workflows_use_node24_checkout() {
    for workflow in [
        RELEASE_WORKFLOW,
        CI_WORKFLOW,
        SECURITY_WORKFLOW,
        SOAK_WORKFLOW,
    ] {
        assert!(workflow.contains(CHECKOUT_NODE24));
        assert!(!workflow.contains("actions/checkout@34e114876b0b11c390a56381ad16ebd13914f8d5"));
    }
}

/// 校验产物上传仅在首次失败后等待并覆盖重试一次。
#[test]
// 产物上传失败后覆盖重试一次。
fn artifact_upload_retries_once_with_overwrite() {
    assert_eq!(RELEASE_WORKFLOW.matches(UPLOAD_NODE24).count(), 2);
    assert!(RELEASE_WORKFLOW.contains("id: upload_artifact"));
    assert!(RELEASE_WORKFLOW.contains("if: steps.upload_artifact.outcome == 'failure'"));
    assert!(RELEASE_WORKFLOW.contains("run: sleep 10"));
    assert!(RELEASE_WORKFLOW.contains("overwrite: true"));
}

/// 校验发布和安全审计动作都显式固定到 Node 24 实现。
#[test]
// 发布与审计动作固定node24实现。
fn release_and_audit_actions_use_node24() {
    let audit = "rustsec/audit-check@858dc40f52ca2b8570b7a997c1c4e35c6fc9a432";
    assert!(RELEASE_WORKFLOW.contains(audit));
    assert!(CI_WORKFLOW.contains(audit));
    assert!(SECURITY_WORKFLOW.contains(audit));
    assert!(
        RELEASE_WORKFLOW
            .contains("actions/download-artifact@3e5f45b2cfb9172054b4087a40e8e0b5a5461e7c")
    );
    assert!(
        RELEASE_WORKFLOW
            .contains("softprops/action-gh-release@3d0d9888cb7fd7b750713d6e236d1fcb99157228")
    );
}
