//! 裸机部署二进制矩阵的多格式解析与平台选择测试。

use std::path::Path;

use procora::config::{ConfigFormat, DeployPlatform, load_str, select_deploy_binaries};
use procora::core::TaskId;

#[test]
// 三种格式会把常用平台别名规范化为同一个二进制矩阵。
fn deploy_binary_matrix_is_equivalent_across_formats() {
    let cases = [
        (
            ConfigFormat::Yaml,
            r#"
version: 1
project: demo
binaries:
  api:
    target: bin/api
    variants:
      linux-amd64: dist/api-linux-x86
      macos-arm64:
        source: dist/api-arm64-macos
tasks:
  api:
    command: "${binary.api}"
"#,
        ),
        (
            ConfigFormat::Toml,
            r#"
version = 1
project = "demo"

[binaries.api]
target = "bin/api"

[binaries.api.variants]
linux-amd64 = "dist/api-linux-x86"
macos-arm64 = { source = "dist/api-arm64-macos" }

[tasks.api]
command = "${binary.api}"
"#,
        ),
        (
            ConfigFormat::Json,
            r#"{
  "version": 1,
  "project": "demo",
  "binaries": {
    "api": {
      "target": "bin/api",
      "variants": {
        "linux-amd64": "dist/api-linux-x86",
        "macos-arm64": {"source": "dist/api-arm64-macos"}
      }
    }
  },
  "tasks": {"api": {"command": "${binary.api}"}}
}"#,
        ),
    ];

    for (format, input) in cases {
        let compiled = load_str(input, format).unwrap();
        let binary = &compiled.deploy_binaries["api"];
        assert_eq!(binary.target, Path::new("bin/api"));
        assert_eq!(binary.variants.len(), 2);
        let task = "api".parse::<TaskId>().unwrap();
        assert_eq!(compiled.spec.tasks[&task].command, "${binary.api}");

        let selected = select_deploy_binaries(
            &compiled.deploy_binaries,
            &DeployPlatform {
                os: "darwin".to_owned(),
                arch: "aarch64".to_owned(),
                environment: None,
            }
            .normalized()
            .unwrap(),
        )
        .unwrap();
        assert_eq!(selected[0].selector, "macos-aarch64");
        assert_eq!(selected[0].source, Path::new("dist/api-arm64-macos"));
        assert_eq!(selected[0].target, Path::new("bin/api"));
    }
}

#[test]
// ABI精确变体优先于通用OS与架构变体。
fn deploy_binary_environment_variant_has_priority() {
    let compiled = load_str(
        r"
version: 1
project: demo
binaries:
  api:
    target: bin/api
    variants:
      linux-amd64: dist/api-generic
      linux-x86_64-musl: dist/api-musl
tasks: {}
",
        ConfigFormat::Yaml,
    )
    .unwrap();

    let selected = select_deploy_binaries(
        &compiled.deploy_binaries,
        &DeployPlatform {
            os: "linux".to_owned(),
            arch: "x86_64".to_owned(),
            environment: Some("musl".to_owned()),
        },
    )
    .unwrap();

    assert_eq!(selected[0].source, Path::new("dist/api-musl"));
}

#[test]
// Linux精确ABI、macOS universal和Windows变体target覆盖形成完整三平台矩阵。
fn deploy_binary_selects_three_platform_specific_targets() {
    let compiled = load_str(
        r"
version: 1
project: demo
binaries:
  api:
    target: bin/api
    variants:
      linux-amd64: dist/api-linux
      linux-x86_64-musl: dist/api-linux-musl
      macos-universal: dist/api-macos-universal
      windows-amd64:
        source: dist/api-windows.exe
        target: bin/api.exe
tasks: {}
",
        ConfigFormat::Yaml,
    )
    .unwrap();

    let cases = [
        (
            DeployPlatform {
                os: "linux".to_owned(),
                arch: "x86_64".to_owned(),
                environment: Some("musl".to_owned()),
            },
            "linux-x86_64-musl",
            "bin/api",
        ),
        (
            DeployPlatform {
                os: "macos".to_owned(),
                arch: "aarch64".to_owned(),
                environment: None,
            },
            "macos-universal",
            "bin/api",
        ),
        (
            DeployPlatform {
                os: "windows".to_owned(),
                arch: "x86_64".to_owned(),
                environment: Some("msvc".to_owned()),
            },
            "windows-x86_64",
            "bin/api.exe",
        ),
    ];

    for (platform, selector, target) in cases {
        let selected = select_deploy_binaries(&compiled.deploy_binaries, &platform).unwrap();
        assert_eq!(selected[0].selector, selector);
        assert_eq!(selected[0].target, Path::new(target));
    }
}

#[test]
// 精确macOS架构必须压过universal，且Windows ARM64保留Unicode路径和exe target。
fn deploy_binary_prefers_exact_macos_and_supports_windows_arm64_unicode() {
    let compiled = load_str(
        r#"
version: 1
project: demo
binaries:
  worker:
    target: "程序/worker"
    variants:
      macos-universal: "构建/worker-universal"
      macos-arm64: "构建/worker-apple-silicon"
      windows-arm64:
        source: "构建/worker-windows-arm64.exe"
        target: "程序/worker.exe"
tasks: {}
"#,
        ConfigFormat::Yaml,
    )
    .unwrap();

    let macos = select_deploy_binaries(
        &compiled.deploy_binaries,
        &DeployPlatform {
            os: "Darwin".to_owned(),
            arch: "ARM64".to_owned(),
            environment: None,
        }
        .normalized()
        .unwrap(),
    )
    .unwrap();
    assert_eq!(macos[0].selector, "macos-aarch64");
    assert_eq!(macos[0].source, Path::new("构建/worker-apple-silicon"));

    let windows = select_deploy_binaries(
        &compiled.deploy_binaries,
        &DeployPlatform {
            os: "windows".to_owned(),
            arch: "aarch64".to_owned(),
            environment: Some("msvc".to_owned()),
        },
    )
    .unwrap();
    assert_eq!(windows[0].target, Path::new("程序/worker.exe"));
}

#[test]
// 远端实际平台不能伪装成仅供配置选择器使用的macOS universal。
fn deploy_platform_rejects_universal_as_runtime_architecture() {
    let error = DeployPlatform {
        os: "macos".to_owned(),
        arch: "universal2".to_owned(),
        environment: None,
    }
    .normalized()
    .unwrap_err();

    assert!(error.contains("不支持的架构"), "{error}");
}

#[test]
// 缺少远端平台变体时列出当前平台和已经声明的矩阵。
fn deploy_binary_missing_platform_is_actionable() {
    let compiled = load_str(
        r"
version: 1
project: demo
binaries:
  api:
    target: bin/api
    variants:
      macos-arm64: dist/api
tasks: {}
",
        ConfigFormat::Yaml,
    )
    .unwrap();

    let error = select_deploy_binaries(
        &compiled.deploy_binaries,
        &DeployPlatform {
            os: "linux".to_owned(),
            arch: "x86_64".to_owned(),
            environment: Some("gnu".to_owned()),
        },
    )
    .unwrap_err();

    assert!(error.contains("linux-x86_64-gnu"), "{error}");
    assert!(error.contains("macos-aarch64"), "{error}");
}

#[test]
// 不安全target和规范化后重复的平台键会在配置阶段同时报错。
fn deploy_binary_rejects_unsafe_target_and_duplicate_aliases() {
    let error = load_str(
        r"
version: 1
project: demo
binaries:
  api:
    target: ../api
    variants:
      linux-amd64: dist/one
      linux-x86_64: dist/two
tasks: {}
",
        ConfigFormat::Yaml,
    )
    .unwrap_err()
    .to_string();

    assert!(error.contains("binaries.api.target"), "{error}");
    assert!(error.contains("规范化后重复"), "{error}");
}

#[test]
// universal只用于macOS且变体级target遵守可移植路径限制。
fn deploy_binary_rejects_invalid_universal_and_variant_target() {
    let error = load_str(
        r"
version: 1
project: demo
binaries:
  api:
    target: bin/api
    variants:
      linux-universal: dist/api
      windows-amd64:
        source: dist/api.exe
        target: ../api.exe
tasks: {}
",
        ConfigFormat::Yaml,
    )
    .unwrap_err()
    .to_string();

    assert!(error.contains("universal"), "{error}");
    assert!(error.contains("windows-amd64.target"), "{error}");
}
