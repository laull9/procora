//! 配置 include 的合并、路径、安全限制、闭包修订与监听测试。

use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use procora::{
    config::{ConfigError, load_path},
    source::LocalFileSource,
};

/// 当前测试进程内的临时目录去重序列。
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// 创建当前测试独占的临时目录。
fn temporary_directory(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let directory = std::env::temp_dir().join(format!(
        "procora-include-{label}-{}-{nonce}-{sequence}",
        std::process::id()
    ));
    fs::create_dir_all(&directory).unwrap();
    directory
}

#[test]
// 三种格式片段按声明顺序合并且入口优先。
fn format_fragments_merge_in_order_with_entry_priority() {
    let root = temporary_directory("merge");
    let fragments = root.join("fragments");
    fs::create_dir_all(&fragments).unwrap();
    fs::write(
        fragments.join("base.toml"),
        "[dependencies.tool]\nsource = './tool.bin'\nversion = '1'\nkind = 'file'\n\n[tasks.worker]\ncommand = 'base'\ncwd = './base-work'\n\n[tasks.api]\ncommand = 'fragment-api'\n[tasks.api.depends_on.worker]\n",
    )
    .unwrap();
    fs::write(
        fragments.join("override.json"),
        r#"{"tasks":{"worker":{"command":"override","cwd":"./override-work"}}}"#,
    )
    .unwrap();
    fs::write(
        root.join("procora.yaml"),
        "include: [fragments/base.toml, fragments/override.json]\nversion: 1\nproject: merged\ntasks:\n  api:\n    command: entry-api\n    depends_on:\n      worker: {}\n",
    )
    .unwrap();

    let compiled = load_path(root.join("procora.yaml")).unwrap();
    let canonical_fragments = fs::canonicalize(&fragments).unwrap();
    let worker = "worker".parse().unwrap();
    let api = "api".parse().unwrap();

    assert_eq!(compiled.spec.tasks[&worker].command, "override");
    assert_eq!(compiled.spec.tasks[&api].command, "entry-api");
    assert_eq!(
        compiled.spec.tasks[&worker].cwd,
        Some(canonical_fragments.join("override-work"))
    );
    assert_eq!(
        compiled.dependencies["tool"].source,
        canonical_fragments.join("tool.bin").to_string_lossy()
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
// include可以跨文件声明依赖边并拒绝身份冲突。
fn include_supports_cross_file_edges_and_rejects_identity_conflicts() {
    let root = temporary_directory("identity");
    fs::write(
        root.join("tasks.yaml"),
        "tasks:\n  database:\n    command: database\n",
    )
    .unwrap();
    let entry = root.join("procora.yaml");
    fs::write(
        &entry,
        "include: [tasks.yaml]\nversion: 1\nproject: demo\ntasks:\n  api:\n    command: api\n    depends_on:\n      database: {}\n",
    )
    .unwrap();
    assert!(load_path(&entry).is_ok());

    fs::write(
        root.join("tasks.yaml"),
        "project: another\ntasks:\n  database:\n    command: database\n",
    )
    .unwrap();
    let error = load_path(&entry).unwrap_err();
    assert!(error.to_string().contains("project 与入口不一致"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
// include拒绝父目录循环和过深闭包。
fn include_rejects_parent_cycles_and_excessive_depth() {
    let root = temporary_directory("limits");
    let outside = root.parent().unwrap().join("outside.yaml");
    fs::write(&outside, "tasks: {}\n").unwrap();
    let entry = root.join("procora.yaml");
    fs::write(
        &entry,
        "include: [../outside.yaml]\nversion: 1\nproject: demo\ntasks: {}\n",
    )
    .unwrap();
    assert!(
        load_path(&entry)
            .unwrap_err()
            .to_string()
            .contains("相对路径")
    );

    fs::write(
        &entry,
        "include: [loop.yaml]\nversion: 1\nproject: demo\ntasks: {}\n",
    )
    .unwrap();
    fs::write(root.join("loop.yaml"), "include: [loop.yaml]\ntasks: {}\n").unwrap();
    assert!(load_path(&entry).unwrap_err().to_string().contains("循环"));

    for depth in 0..=17 {
        let include = if depth == 17 {
            String::new()
        } else {
            format!("include: [depth-{}.yaml]\n", depth + 1)
        };
        fs::write(
            root.join(format!("depth-{depth}.yaml")),
            format!("{include}tasks: {{}}\n"),
        )
        .unwrap();
    }
    fs::write(
        &entry,
        "include: [depth-0.yaml]\nversion: 1\nproject: demo\ntasks: {}\n",
    )
    .unwrap();
    assert!(
        load_path(&entry)
            .unwrap_err()
            .to_string()
            .contains("深度超过")
    );
    fs::remove_file(outside).unwrap();
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
// include拒绝通过符号链接逃逸和超大输入。
fn include_rejects_symlink_escape_and_oversized_input() {
    use std::os::unix::fs::symlink;

    let root = temporary_directory("symlink");
    let outside = temporary_directory("symlink-outside");
    fs::write(outside.join("fragment.yaml"), "tasks: {}\n").unwrap();
    symlink(&outside, root.join("linked")).unwrap();
    let entry = root.join("procora.yaml");
    fs::write(
        &entry,
        "include: [linked/fragment.yaml]\nversion: 1\nproject: demo\ntasks: {}\n",
    )
    .unwrap();
    assert!(
        load_path(&entry)
            .unwrap_err()
            .to_string()
            .contains("符号链接")
    );

    fs::write(root.join("large.yaml"), vec![b' '; 4 * 1024 * 1024 + 1]).unwrap();
    fs::write(
        &entry,
        "include: [large.yaml]\nversion: 1\nproject: demo\ntasks: {}\n",
    )
    .unwrap();
    assert!(
        load_path(&entry)
            .unwrap_err()
            .to_string()
            .contains("字节上限")
    );
    fs::remove_dir_all(root).unwrap();
    fs::remove_dir_all(outside).unwrap();
}

#[test]
// 入口必须显式声明身份。
fn entry_must_declare_identity() {
    let root = temporary_directory("entry-identity");
    fs::write(
        root.join("base.yaml"),
        "version: 1\nproject: inherited\ntasks: {}\n",
    )
    .unwrap();
    let entry = root.join("procora.yaml");
    fs::write(&entry, "include: [base.yaml]\ntasks: {}\n").unwrap();
    let error = load_path(entry).unwrap_err();
    assert!(matches!(error, ConfigError::Include(_)));
    assert!(error.to_string().contains("显式声明 version"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
// 无路径内存加载拒绝静默忽略include。
fn in_memory_load_without_path_rejects_includes() {
    let error = procora::config::load_str(
        "include: [base.yaml]\nversion: 1\nproject: demo\ntasks: {}\n",
        procora::config::ConfigFormat::Yaml,
    )
    .unwrap_err();
    assert!(matches!(error, ConfigError::Include(_)));
    assert!(error.to_string().contains("文件路径加载"));
}

#[test]
// 闭包修订随include内容变化且不依赖入口改写。
fn closure_revision_tracks_include_content() {
    let root = temporary_directory("revision");
    let entry = write_entry(&root, "child.yaml");
    fs::write(
        root.join("child.yaml"),
        "tasks:\n  worker:\n    command: first\n",
    )
    .unwrap();
    let source = LocalFileSource::new(&entry);
    let first = source.read_candidate().revision.unwrap();

    fs::write(
        root.join("child.yaml"),
        "tasks:\n  worker:\n    command: second\n",
    )
    .unwrap();
    let second = source.read_candidate().revision.unwrap();

    assert_ne!(first, second);
    fs::remove_dir_all(root).unwrap();
}

#[test]
// 缺失include创建后监听器会产生完整候选。
fn creating_missing_include_produces_complete_candidate() {
    let root = temporary_directory("watch-missing");
    let entry = write_entry(&root, "missing.yaml");
    let source = LocalFileSource::new(&entry);
    let initial = source.read_candidate();
    assert!(initial.compiled.is_err());
    let mut monitor = source.monitor(Duration::from_millis(80)).unwrap();

    fs::write(
        root.join("missing.yaml"),
        "tasks:\n  worker:\n    command: worker\n",
    )
    .unwrap();
    let candidate = wait_candidate(&mut monitor);

    assert!(candidate.compiled.is_ok());
    assert_eq!(candidate.compiled.unwrap().spec.tasks.len(), 1);
    fs::remove_dir_all(root).unwrap();
}

/// 写入引用单个同目录片段的入口配置。
fn write_entry(root: &Path, include: &str) -> PathBuf {
    let entry = root.join("procora.yaml");
    fs::write(
        &entry,
        format!("include: [{include}]\nversion: 1\nproject: demo\ntasks: {{}}\n"),
    )
    .unwrap();
    entry
}

/// 在有限期限内等待一次防抖后的候选。
fn wait_candidate(
    monitor: &mut procora::source::LocalFileMonitor,
) -> procora::source::DefinitionCandidate {
    let deadline = Instant::now() + Duration::from_secs(4);
    while Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20));
        if let Some(candidate) = monitor.poll() {
            return candidate;
        }
    }
    panic!("include 文件变化应产生候选");
}
