//! 固定提交 Git 定义源的获取、候选与安全边界测试。

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use procora::source::{GitSource, GitSourceLimits};

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
        "procora-git-{label}-{}-{nonce}-{sequence}",
        std::process::id()
    ));
    fs::create_dir_all(&directory).unwrap();
    directory
}

/// 初始化具备本地提交身份的测试仓库。
fn initialize_repository(root: &Path) {
    git(root, &["init", "--quiet"]);
    git(root, &["config", "user.name", "Procora Test"]);
    git(root, &["config", "user.email", "procora@example.invalid"]);
}

/// 提交当前仓库全部变化并返回完整提交哈希。
fn commit_all(root: &Path, message: &str) -> String {
    git(root, &["add", "."]);
    git(root, &["commit", "--quiet", "-m", message]);
    git_output(root, &["rev-parse", "HEAD"])
}

/// 执行必须成功且无需读取输出的 Git 命令。
fn git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?} 失败：{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// 执行必须成功并返回裁剪后 stdout 的 Git 命令。
fn git_output(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .unwrap();
    assert!(output.status.success());
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

/// 写入包含跨格式 include 且若被启动会留下标记的定义。
fn write_valid_definition(repository: &Path, command: &str) {
    let service = repository.join("service");
    fs::create_dir_all(service.join("fragments")).unwrap();
    fs::write(
        service.join("fragments/tasks.toml"),
        format!("[tasks.worker]\ncommand = '{command}'\ncwd = './work'\n"),
    )
    .unwrap();
    fs::write(
        service.join("procora.yaml"),
        "include: [fragments/tasks.toml]\nversion: 1\nproject: git-demo\ntasks: {}\n",
    )
    .unwrap();
}

#[test]
// 本地仓库固定提交并通过共享include管线生成候选。
fn local_repository_pins_commit_and_uses_include_pipeline() {
    let root = temporary_directory("candidate");
    let repository = root.join("repository");
    let cache = root.join("cache");
    fs::create_dir(&repository).unwrap();
    initialize_repository(&repository);
    write_valid_definition(&repository, "procora-command-that-must-not-run");
    let commit = commit_all(&repository, "initial");

    let candidate = GitSource::local(&repository, "HEAD", "service/procora.yaml", &cache)
        .unwrap()
        .fetch_candidate()
        .unwrap();

    assert_eq!(candidate.commit, commit);
    assert_eq!(candidate.revision.len(), 64);
    #[cfg(windows)]
    assert!(!candidate.repository.starts_with(r"\\?\"));
    assert!(!candidate.checkout_root.join(".git").exists());
    assert!(!root.join("must-not-run").exists());
    let compiled = candidate.compiled.unwrap();
    assert_eq!(compiled.spec.project, "git-demo");
    let task = compiled.spec.tasks.values().next().unwrap();
    assert_eq!(task.command, "procora-command-that-must-not-run");
    assert_eq!(
        task.cwd.as_deref(),
        Some(
            fs::canonicalize(&candidate.checkout_root)
                .unwrap()
                .join("service/fragments/work")
                .as_path()
        )
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
// 可变引用每次解析为不可变提交并改变确认修订。
fn mutable_ref_resolves_to_commit_and_changes_revision() {
    let root = temporary_directory("moving-ref");
    let repository = root.join("repository");
    let cache = root.join("cache");
    fs::create_dir(&repository).unwrap();
    initialize_repository(&repository);
    write_valid_definition(&repository, "first");
    let first_commit = commit_all(&repository, "first");
    let source = GitSource::local(&repository, "HEAD", "service/procora.yaml", &cache).unwrap();
    let first = source.fetch_candidate().unwrap();

    write_valid_definition(&repository, "second");
    let second_commit = commit_all(&repository, "second");
    let error = source
        .confirm_candidate(&first.revision)
        .unwrap_err()
        .to_string();
    let second = source.fetch_candidate().unwrap();

    assert_eq!(first.commit, first_commit);
    assert_eq!(second.commit, second_commit);
    assert_ne!(first.commit, second.commit);
    assert_ne!(first.revision, second.revision);
    assert_ne!(first.checkout_root, second.checkout_root);
    assert!(error.contains("修订已变化"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
// 配置无效仍返回带提交身份的不可应用候选。
fn invalid_config_returns_non_applicable_candidate_with_commit() {
    let root = temporary_directory("invalid-config");
    let repository = root.join("repository");
    fs::create_dir(&repository).unwrap();
    initialize_repository(&repository);
    fs::write(repository.join("procora.yaml"), "version: 1\ntasks: [\n").unwrap();
    let commit = commit_all(&repository, "invalid");

    let candidate = GitSource::local(&repository, "HEAD", "procora.yaml", root.join("cache"))
        .unwrap()
        .fetch_candidate()
        .unwrap();

    assert_eq!(candidate.commit, commit);
    assert!(candidate.compiled.is_err());
    assert_eq!(candidate.revision.len(), 64);
    fs::remove_dir_all(root).unwrap();
}

#[test]
// 相同提交的本地checkout被修改后拒绝复用。
fn modified_checkout_for_same_commit_is_not_reused() {
    let root = temporary_directory("integrity");
    let repository = root.join("repository");
    fs::create_dir(&repository).unwrap();
    initialize_repository(&repository);
    write_valid_definition(&repository, "first");
    commit_all(&repository, "first");
    let source = GitSource::local(
        &repository,
        "HEAD",
        "service/procora.yaml",
        root.join("cache"),
    )
    .unwrap();
    let candidate = source.fetch_candidate().unwrap();
    fs::write(
        &candidate.config_path,
        "version: 1\nproject: changed\ntasks: {}\n",
    )
    .unwrap();

    let error = source.fetch_candidate().unwrap_err().to_string();

    assert!(error.contains("缓存"));
    assert!(error.contains("修改"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
// 归档输出上限在物化前终止获取。
fn archive_output_limit_stops_fetch_before_materialization() {
    let root = temporary_directory("archive-limit");
    let repository = root.join("repository");
    fs::create_dir(&repository).unwrap();
    initialize_repository(&repository);
    write_valid_definition(&repository, "first");
    commit_all(&repository, "first");
    let limits = GitSourceLimits {
        archive_bytes: 512,
        ..GitSourceLimits::default()
    };
    let source = GitSource::local(
        &repository,
        "HEAD",
        "service/procora.yaml",
        root.join("cache"),
    )
    .unwrap()
    .with_limits(limits);

    let error = source.fetch_candidate().unwrap_err().to_string();

    assert!(error.contains("stdout 超过 512 字节"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
// 对象库增长在fetch执行期间触发整树终止。
fn object_store_growth_terminates_fetch_process_tree() {
    let root = temporary_directory("repository-limit");
    let repository = root.join("repository");
    fs::create_dir(&repository).unwrap();
    initialize_repository(&repository);
    write_valid_definition(&repository, "first");
    commit_all(&repository, "first");
    let limits = GitSourceLimits {
        repository_bytes: 1,
        ..GitSourceLimits::default()
    };
    let source = GitSource::local(
        &repository,
        "HEAD",
        "service/procora.yaml",
        root.join("cache"),
    )
    .unwrap()
    .with_limits(limits);

    let error = source.fetch_candidate().unwrap_err().to_string();

    assert!(error.contains("临时对象仓库超过 1 字节"));
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
// checkout中后来插入的符号链接被当作篡改拒绝。
fn checkout_rejects_later_symlink_as_tampering() {
    use std::os::unix::fs::symlink;

    let root = temporary_directory("checkout-symlink");
    let repository = root.join("repository");
    fs::create_dir(&repository).unwrap();
    initialize_repository(&repository);
    write_valid_definition(&repository, "first");
    commit_all(&repository, "first");
    let source = GitSource::local(
        &repository,
        "HEAD",
        "service/procora.yaml",
        root.join("cache"),
    )
    .unwrap();
    let candidate = source.fetch_candidate().unwrap();
    fs::remove_file(&candidate.config_path).unwrap();
    symlink(&repository, &candidate.config_path).unwrap();

    let error = source.fetch_candidate().unwrap_err().to_string();

    assert!(error.contains("不安全"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
// 远端协议引用和配置路径先于git执行被拒绝。
fn unsafe_remote_inputs_are_rejected_before_git_runs() {
    let cache = temporary_directory("validation");
    assert!(GitSource::remote("file:///tmp/repo", "main", "procora.yaml", &cache).is_err());
    assert!(GitSource::remote("ext::command", "main", "procora.yaml", &cache).is_err());
    assert!(
        GitSource::remote(
            "https://token@example.com/repo.git",
            "main",
            "procora.yaml",
            &cache
        )
        .is_err()
    );
    assert!(
        GitSource::remote(
            "https://example.com/repo.git",
            "--upload-pack=evil",
            "procora.yaml",
            &cache
        )
        .is_err()
    );
    assert!(
        GitSource::remote(
            "https://example.com/repo.git",
            "main",
            "../procora.yaml",
            &cache
        )
        .is_err()
    );
    assert!(
        GitSource::remote("https://example.com/repo.git", "main", "procora.py", &cache).is_err()
    );
    fs::remove_dir_all(cache).unwrap();
}
