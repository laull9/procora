//! 显式 Task 环境文件的解析、边界、修订与监听测试。

use std::{
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use procora::{
    config::{ConfigFormat, load_path, load_str},
    source::{DefinitionCandidate, LocalFileSource},
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
        "procora-env-{label}-{}-{nonce}-{sequence}",
        std::process::id()
    ));
    fs::create_dir_all(&directory).unwrap();
    directory
}

/// 写入引用指定环境文件的最小 YAML 配置。
fn write_yaml(root: &Path, env_file: &str) -> PathBuf {
    let entry = root.join("procora.yaml");
    fs::write(
        &entry,
        format!(
            "version: 1\nproject: demo\ntasks:\n  api:\n    command: echo\n    env_file: {env_file}\n"
        ),
    )
    .unwrap();
    entry
}

/// 等待监听器返回下一个候选，避免依赖固定文件事件延迟。
fn wait_for_candidate(source: &mut procora::source::LocalFileMonitor) -> DefinitionCandidate {
    let deadline = Instant::now() + Duration::from_secs(4);
    loop {
        if let Some(candidate) = source.poll() {
            return candidate;
        }
        assert!(Instant::now() < deadline, "没有收到环境文件候选事件");
        thread::sleep(Duration::from_millis(20));
    }
}

#[test]
// 三种声明式格式读取同一环境文件并让内联env取得最高优先级。
fn formats_load_explicit_env_file_with_inline_precedence() {
    let root = temporary_directory("formats");
    fs::write(
        root.join("task.env"),
        concat!(
            "\u{feff}# comment\n",
            "PLAIN=from-file\n",
            "INLINE=from-file\n",
            "export SINGLE='literal # value'\n",
            "DOUBLE=\"line\\nquote\\\"\" # trailing comment\n",
            "HASH=value#literal\n",
            "COMMENT=value # removed\n",
            "EMPTY=\n",
            "DUP=first\n",
            "DUP=second\n",
        ),
    )
    .unwrap();
    fs::write(root.join(".env"), "IMPLICIT=must-not-load\n").unwrap();
    fs::write(
        root.join("procora.yaml"),
        "version: 1\nproject: demo\ntasks:\n  api:\n    command: echo\n    env_file: task.env\n    env:\n      INLINE: direct\n",
    )
    .unwrap();
    fs::write(
        root.join("procora.toml"),
        "version = 1\nproject = 'demo'\n[tasks.api]\ncommand = 'echo'\nenv_file = 'task.env'\n[tasks.api.env]\nINLINE = 'direct'\n",
    )
    .unwrap();
    fs::write(
        root.join("procora.json"),
        r#"{"version":1,"project":"demo","tasks":{"api":{"command":"echo","env_file":"task.env","env":{"INLINE":"direct"}}}}"#,
    )
    .unwrap();

    let yaml = load_path(root.join("procora.yaml")).unwrap();
    let toml = load_path(root.join("procora.toml")).unwrap();
    let json = load_path(root.join("procora.json")).unwrap();
    assert_eq!(yaml.spec, toml.spec);
    assert_eq!(toml.spec, json.spec);
    let task = yaml.spec.tasks.values().next().unwrap();
    assert_eq!(task.env["PLAIN"], "from-file");
    assert_eq!(task.env["INLINE"], "direct");
    assert_eq!(task.env["SINGLE"], "literal # value");
    assert_eq!(task.env["DOUBLE"], "line\nquote\"");
    assert_eq!(task.env["HASH"], "value#literal");
    assert_eq!(task.env["COMMENT"], "value");
    assert_eq!(task.env["EMPTY"], "");
    assert_eq!(task.env["DUP"], "second");
    assert!(!task.env.contains_key("IMPLICIT"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
// env_file按声明它的include文件解析，并参与修订与热更新监听。
fn included_env_file_changes_revision_and_produces_candidate() {
    let root = temporary_directory("revision");
    let fragments = root.join("fragments");
    fs::create_dir_all(&fragments).unwrap();
    fs::write(
        fragments.join("task.yaml"),
        "tasks:\n  api:\n    command: echo\n    env_file: task.env\n",
    )
    .unwrap();
    fs::write(fragments.join("task.env"), "VALUE=first\n").unwrap();
    let entry = root.join("procora.yaml");
    fs::write(
        &entry,
        "include: [fragments/task.yaml]\nversion: 1\nproject: demo\ntasks: {}\n",
    )
    .unwrap();
    let source = LocalFileSource::new(&entry);
    let first = source.read_candidate();
    let first_revision = first.revision.unwrap();
    assert_eq!(
        first
            .compiled
            .unwrap()
            .spec
            .tasks
            .values()
            .next()
            .unwrap()
            .env["VALUE"],
        "first"
    );
    let mut monitor = source.monitor(Duration::from_millis(80)).unwrap();

    fs::write(fragments.join("task.env"), "VALUE=second\n").unwrap();
    let second = wait_for_candidate(&mut monitor);
    assert_ne!(first_revision, second.revision.unwrap());
    assert_eq!(
        second
            .compiled
            .unwrap()
            .spec
            .tasks
            .values()
            .next()
            .unwrap()
            .env["VALUE"],
        "second"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
// 缺失环境文件创建后可由监听器恢复为有效候选。
fn creating_missing_env_file_recovers_candidate() {
    let root = temporary_directory("missing");
    let entry = write_yaml(&root, "task.env");
    let source = LocalFileSource::new(&entry);
    assert!(source.read_candidate().compiled.is_err());
    let mut monitor = source.monitor(Duration::from_millis(80)).unwrap();

    fs::write(root.join("task.env"), "READY=yes\n").unwrap();
    let candidate = wait_for_candidate(&mut monitor);
    assert_eq!(
        candidate
            .compiled
            .unwrap()
            .spec
            .tasks
            .values()
            .next()
            .unwrap()
            .env["READY"],
        "yes"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
// 无路径文本拒绝相对env_file，非法dotenv行返回字段与行号。
fn relative_or_invalid_env_file_is_actionable() {
    let error = load_str(
        "version: 1\nproject: demo\ntasks:\n  api:\n    command: echo\n    env_file: task.env\n",
        ConfigFormat::Yaml,
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("tasks.api.env_file"));
    assert!(error.contains("配置文件路径"));

    let root = temporary_directory("invalid");
    fs::write(root.join("task.env"), "GOOD=yes\nBROKEN\n").unwrap();
    let error = load_path(write_yaml(&root, "task.env"))
        .unwrap_err()
        .to_string();
    assert!(error.contains("tasks.api.env_file"));
    assert!(error.contains("第 2 行"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
// 环境文件字节数和变量数量都有硬上限。
fn env_file_size_and_variable_count_are_bounded() {
    let root = temporary_directory("limits");
    let entry = write_yaml(&root, "task.env");
    fs::write(root.join("task.env"), vec![b'A'; 1024 * 1024 + 1]).unwrap();
    let error = load_path(&entry).unwrap_err().to_string();
    assert!(error.contains("1048576"));

    let mut variables = String::new();
    for index in 0..=4096 {
        writeln!(variables, "VALUE_{index}=yes").unwrap();
    }
    fs::write(root.join("task.env"), variables).unwrap();
    let error = load_path(entry).unwrap_err().to_string();
    assert!(error.contains("4096"));
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
// 环境文件不能通过符号链接逃出服务根目录。
fn env_file_rejects_symlink_escape() {
    use std::os::unix::fs::symlink;

    let root = temporary_directory("symlink");
    let outside = temporary_directory("outside");
    fs::write(outside.join("secret.env"), "SECRET=value\n").unwrap();
    symlink(outside.join("secret.env"), root.join("task.env")).unwrap();

    let error = load_path(write_yaml(&root, "task.env"))
        .unwrap_err()
        .to_string();
    assert!(error.contains("符号链接"));
    fs::remove_dir_all(root).unwrap();
    fs::remove_dir_all(outside).unwrap();
}
