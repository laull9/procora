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
    assert_eq!(tool.download.retries, 2);
    assert_eq!(tool.download.timeout_ms, 120_000);
    assert_eq!(tool.download.max_bytes, 2 * 1024 * 1024 * 1024);
    assert_eq!(tool.verify.as_ref().unwrap().args, ["--version"]);
    assert_eq!(
        compiled.dependencies["assets"].kind,
        DependencyKind::Directory
    );
}

#[test]
// 镜像和下载边界会进入统一领域模型。
fn remote_download_policy_is_normalized() {
    let project = load_str(
        r#"version: 1
project: demo
dependencies:
  asset:
    source: https://primary.example/asset
    mirrors: [https://mirror.example/asset, ssh://files.example/opt/asset]
    version: v1
    download:
      retries: 4
      timeout: 45s
      max_bytes: 4096
      headers:
        Authorization: "Bearer ${env.PROCORA_TOKEN}"
tasks: {}
"#,
        ConfigFormat::Yaml,
    )
    .unwrap();
    let asset = &project.dependencies["asset"];
    assert_eq!(asset.mirrors.len(), 2);
    assert_eq!(asset.download.retries, 4);
    assert_eq!(asset.download.timeout_ms, 45_000);
    assert_eq!(asset.download.max_bytes, 4096);
    assert_eq!(
        asset.download.headers["Authorization"],
        "Bearer ${env.PROCORA_TOKEN}"
    );
}

#[test]
// 一行来源在三种格式中使用同一组开箱即用默认值。
fn one_line_remote_sources_are_equivalent_across_formats() {
    let cases = [
        (
            ConfigFormat::Yaml,
            "version: 1\nproject: demo\ndependencies:\n  http: https://example.com/tool.tar.gz\n  ssh: ssh://release@example.com/opt/tool.tar.gz\ntasks: {}\n",
        ),
        (
            ConfigFormat::Toml,
            "version = 1\nproject = 'demo'\n[dependencies]\nhttp = 'https://example.com/tool.tar.gz'\nssh = 'ssh://release@example.com/opt/tool.tar.gz'\n[tasks]\n",
        ),
        (
            ConfigFormat::Json,
            r#"{"version":1,"project":"demo","dependencies":{"http":"https://example.com/tool.tar.gz","ssh":"ssh://release@example.com/opt/tool.tar.gz"},"tasks":{}}"#,
        ),
    ];
    let compiled = cases
        .into_iter()
        .map(|(format, input)| load_str(input, format).unwrap())
        .collect::<Vec<_>>();

    assert_eq!(compiled[0].dependencies, compiled[1].dependencies);
    assert_eq!(compiled[0].dependencies, compiled[2].dependencies);
    for dependency in compiled[0].dependencies.values() {
        assert_eq!(dependency.version, "source");
        assert_eq!(dependency.kind, DependencyKind::Auto);
        assert_eq!(dependency.unpack, UnpackMode::Auto);
        assert_eq!(dependency.download.retries, 2);
    }
}

#[test]
// 对象写法也可以只声明来源并继承默认版本。
fn object_source_does_not_require_version() {
    let compiled = load_str(
        "version: 1\nproject: demo\ndependencies:\n  asset:\n    source: https://example.com/asset.bin\ntasks: {}\n",
        ConfigFormat::Yaml,
    )
    .unwrap();

    assert_eq!(compiled.dependencies["asset"].version, "source");
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
    mirrors: [ftp://example.com/file]
    download:
      retries: 11
      timeout: 999ms
      max_bytes: 0
      headers:
        "Bad Name": "bad\nvalue"
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
        "dependencies.bad.mirrors.0",
        "dependencies.bad.download.retries",
        "dependencies.bad.download.timeout",
        "dependencies.bad.download.max_bytes",
        "dependencies.bad.download.headers.Bad Name",
    ] {
        assert!(
            diagnostics.iter().any(|item| item.path == path),
            "缺少 {path}"
        );
    }
}
