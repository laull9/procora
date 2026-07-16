//! 项目管理依赖配置的解析与字段诊断测试。

use procora::config::{ConfigError, ConfigFormat, DependencyKind, UnpackMode, load_str};

#[test]
// 网络与ssh依赖会规范化为统一模型。
fn network_and_ssh_dependencies_normalize_to_one_model() {
    let input = r#"
version: 1
project: demo
dependencies:
  tool:
    source: https://example.com/tool.tar.gz
    version: 1.2.3
    checksum: sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
    kind: binary
    path: tool/bin/tool
    verify:
      args: ["--version"]
  assets:
    source: ssh://user@example.com/opt/assets.zip
    version: "2026.07"
    kind: directory
tasks:
  run:
    command: "${dependency.tool}"
"#;

    let compiled = load_str(input, ConfigFormat::Yaml).unwrap();
    let tool = &compiled.dependencies["tool"];
    assert_eq!(tool.kind, DependencyKind::Binary);
    assert_eq!(tool.unpack, UnpackMode::Auto);
    assert_eq!(tool.verify.as_ref().unwrap().args, ["--version"]);
    assert_eq!(
        compiled.dependencies["assets"].kind,
        DependencyKind::Directory
    );
}

#[test]
// 依赖字段错误会一次返回。
fn dependency_field_errors_are_reported_together() {
    let input = r#"
version: 1
project: demo
dependencies:
  bad/name:
    source: local
    version: "1"
  bad:
    source: ftp://example.com/tool
    version: ../escape
    checksum: nope
    path: /absolute
tasks: {}
"#;

    let error = load_str(input, ConfigFormat::Yaml).unwrap_err();
    let ConfigError::Validation { diagnostics, .. } = error else {
        panic!("应返回字段诊断");
    };
    for path in [
        "dependencies.bad/name",
        "dependencies.bad.source",
        "dependencies.bad.version",
        "dependencies.bad.checksum",
        "dependencies.bad.path",
    ] {
        assert!(
            diagnostics.iter().any(|item| item.path == path),
            "缺少 {path}"
        );
    }
}
