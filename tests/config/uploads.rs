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
