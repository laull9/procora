use procora::config::{ConfigFormat, UploadKind, load_str};

#[test]
// Service与Task上传目标会编译为稳定选择器后缀。
fn upload_targets_are_compiled_with_stable_selectors() {
    let compiled = load_str(
        r"
version: 1
project: demo
uploads:
  assets:
    path: shared/assets
    kind: directory
tasks:
  api:
    command: api
    uploads:
      config:
        path: config/api.toml
        kind: file
        max_bytes: 1024
        restart: true
",
        ConfigFormat::Yaml,
    )
    .unwrap();

    assert_eq!(
        compiled.upload_targets["assets"].kind,
        UploadKind::Directory
    );
    assert_eq!(
        compiled.upload_targets["api::config"].kind,
        UploadKind::File
    );
    assert_eq!(compiled.upload_targets["api::config"].max_bytes, 1024);
    assert!(compiled.upload_targets["api::config"].restart);
    assert!(!compiled.upload_targets["assets"].restart);
}

// max_bytes兼容整数、十进制单位与二进制单位文本。
#[test]
fn upload_max_bytes_accepts_human_readable_units() {
    let compiled = load_str(
        r#"
version: 1
project: demo
uploads:
  artifact:
    path: artifact.bin
    kind: file
    max_bytes: 20MB
tasks:
  api:
    command: api
    uploads:
      release:
        path: release
        kind: directory
        max_bytes: "1GiB"
"#,
        ConfigFormat::Yaml,
    )
    .unwrap();

    assert_eq!(compiled.upload_targets["artifact"].max_bytes, 20_000_000);
    assert_eq!(
        compiled.upload_targets["api::release"].max_bytes,
        1_073_741_824
    );
}

#[test]
// 上传目标不能逃逸Service根目录或覆盖Procora运行数据。
fn upload_targets_reject_unsafe_paths() {
    for path in ["../outside", ".", ".procora/logs", "/absolute"] {
        let input = format!(
            "version: 1\nproject: demo\nuploads:\n  bad:\n    path: {path:?}\n    kind: directory\ntasks: {{}}\n"
        );
        let error = load_str(&input, ConfigFormat::Yaml)
            .unwrap_err()
            .to_string();
        assert!(error.contains("uploads.bad.path"), "{path}: {error}");
    }
}
