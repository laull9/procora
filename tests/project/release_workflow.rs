//! 发布工作流依赖与失败恢复契约测试。

/// 发布工作流文本，用于锁定发布依赖与失败恢复契约。
const RELEASE_WORKFLOW: &str = include_str!("../../.github/workflows/release.yml");

/// CI 工作流文本，用于锁定通用检出动作的运行时版本。
const CI_WORKFLOW: &str = include_str!("../../.github/workflows/ci.yml");

/// 安全工作流文本，用于锁定审计动作的运行时版本。
const SECURITY_WORKFLOW: &str = include_str!("../../.github/workflows/security.yml");

/// Cargo 目标链接配置。
const CARGO_CONFIG: &str = include_str!("../../.cargo/config.toml");

/// Unix 发布二进制依赖检查脚本。
const UNIX_DEPENDENCY_CHECK: &str = include_str!("../../scripts/check-runtime-deps.sh");

/// Windows 发布二进制依赖检查脚本。
const WINDOWS_DEPENDENCY_CHECK: &str = include_str!("../../scripts/check-runtime-deps.ps1");

/// Node 24 版 checkout 的固定提交。
const CHECKOUT_NODE24: &str = "actions/checkout@9c091bb21b7c1c1d1991bb908d89e4e9dddfe3e0";

/// Node 24 版 artifact 上传动作的固定提交。
const UPLOAD_NODE24: &str = "actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a";

/// 校验全部工作流不再使用旧版 checkout。
#[test]
// 全部工作流使用node24版checkout。
fn all_workflows_use_node24_checkout() {
    for workflow in [RELEASE_WORKFLOW, CI_WORKFLOW, SECURITY_WORKFLOW] {
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

/// 校验常规 CI 和安全工作流的审计动作显式固定到 Node 24 实现。
#[test]
// CI与安全审计动作固定node24实现。
fn release_and_audit_actions_use_node24() {
    let audit = "rustsec/audit-check@858dc40f52ca2b8570b7a997c1c4e35c6fc9a432";
    assert!(CI_WORKFLOW.contains(audit));
    assert!(SECURITY_WORKFLOW.contains(audit));
    assert!(!RELEASE_WORKFLOW.contains(audit));
    assert!(
        RELEASE_WORKFLOW
            .contains("actions/download-artifact@3e5f45b2cfb9172054b4087a40e8e0b5a5461e7c")
    );
    assert!(
        RELEASE_WORKFLOW
            .contains("softprops/action-gh-release@3d0d9888cb7fd7b750713d6e236d1fcb99157228")
    );
}

/// 校验常规 CI 只在 dev 和 main 提交时运行，PR 不重复执行。
#[test]
// CI只在dev和main提交时运行。
fn ci_avoids_duplicate_pull_request_runs() {
    let workflow = CI_WORKFLOW.replace("\r\n", "\n");
    assert!(workflow.contains("      - dev\n      - main"));
    assert!(!workflow.contains("pull_request:"));
}

/// 校验标签发布复用 main 成功 CI，只执行多平台打包。
#[test]
// 标签发布不重复运行源码测试。
fn release_reuses_successful_main_ci() {
    assert!(RELEASE_WORKFLOW.contains("actions: read"));
    assert!(RELEASE_WORKFLOW.contains("--workflow ci.yml"));
    assert!(RELEASE_WORKFLOW.contains("--branch main"));
    assert!(RELEASE_WORKFLOW.contains("for attempt in {1..60}"));
    assert!(RELEASE_WORKFLOW.contains(".conclusion == \"success\""));
    assert!(RELEASE_WORKFLOW.contains("等待 main 提交"));
    assert!(RELEASE_WORKFLOW.contains("needs: prepare"));
    assert!(!RELEASE_WORKFLOW.contains("cargo test"));
    assert!(!RELEASE_WORKFLOW.contains("cargo clippy"));
}

#[test]
// Linux发布目标使用musl并拒绝动态加载器和共享库。
fn linux_release_is_static_musl() {
    for target in ["x86_64-unknown-linux-musl", "aarch64-unknown-linux-musl"] {
        assert!(RELEASE_WORKFLOW.contains(target));
        assert!(CARGO_CONFIG.contains(&format!("[target.{target}]")));
    }
    assert_eq!(CARGO_CONFIG.matches("linker = \"musl-gcc\"").count(), 2);
    assert!(!RELEASE_WORKFLOW.contains("unknown-linux-gnu"));
    assert!(RELEASE_WORKFLOW.contains("musl-tools"));
    assert!(RELEASE_WORKFLOW.contains("CC_${target//-/_}=musl-gcc"));
    assert!(RELEASE_WORKFLOW.contains("scripts/check-runtime-deps.sh"));
    assert!(UNIX_DEPENDENCY_CHECK.contains("grep -q '(NEEDED)'"));
    assert!(UNIX_DEPENDENCY_CHECK.contains("grep -q 'INTERP'"));
}

#[test]
// Windows发布目标静态链接C运行时并检查最终导入表。
fn windows_release_uses_static_crt() {
    for target in ["x86_64-pc-windows-msvc", "aarch64-pc-windows-msvc"] {
        assert!(CARGO_CONFIG.contains(&format!("[target.{target}]")));
    }
    assert_eq!(
        CARGO_CONFIG.matches("target-feature=+crt-static").count(),
        4
    );
    assert!(RELEASE_WORKFLOW.contains("scripts/check-runtime-deps.ps1"));
    for runtime in ["msvcp", "vcruntime", "ucrtbase.dll", "api-ms-win-crt"] {
        assert!(WINDOWS_DEPENDENCY_CHECK.contains(runtime));
    }
}

#[test]
// macOS发布目标只允许链接Apple系统动态库。
fn macos_release_allows_only_system_libraries() {
    assert!(RELEASE_WORKFLOW.contains("MACOSX_DEPLOYMENT_TARGET=11.0"));
    assert!(UNIX_DEPENDENCY_CHECK.contains("otool -L"));
    assert!(UNIX_DEPENDENCY_CHECK.contains("^(/usr/lib/|/System/Library/)"));
}
