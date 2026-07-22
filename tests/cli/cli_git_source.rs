//! Git 定义源 CLI 的只读预览、重新确认和无启动副作用测试。

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
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
        "procora-cli-git-{label}-{}-{nonce}-{sequence}",
        std::process::id()
    ));
    fs::create_dir_all(&directory).unwrap();
    directory
}

/// 初始化本地测试仓库并固定提交身份。
fn initialize_repository(repository: &Path) {
    fs::create_dir(repository).unwrap();
    git(repository, &["init", "--quiet"]);
    git(repository, &["config", "user.name", "Procora Test"]);
    git(
        repository,
        &["config", "user.email", "procora@example.invalid"],
    );
}

/// 写入定义并提交全部变化。
fn commit_definition(repository: &Path, project: &str) {
    fs::create_dir_all(repository.join("service")).unwrap();
    fs::write(
        repository.join("service/procora.yaml"),
        format!(
            "version: 1\nproject: {project}\ntasks:\n  guarded:\n    command: sh\n    args: ['-c', 'touch SHOULD_NOT_EXIST']\n"
        ),
    )
    .unwrap();
    git(repository, &["add", "."]);
    git(repository, &["commit", "--quiet", "-m", project]);
}

/// 执行必须成功的仓库准备命令。
fn git(repository: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(repository)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?} 失败：{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// 执行 Git 来源 CLI 并使用隔离 Procora 数据目录。
fn procora(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_procora"))
        .args(args)
        .env("PROCORA_HOME", home)
        .output()
        .unwrap()
}

/// 从人类输出中提取指定中文字段。
fn output_field(output: &Output, field: &str) -> String {
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .find_map(|line| line.strip_prefix(field))
        .unwrap_or_else(|| panic!("输出缺少字段 {field}"))
        .to_owned()
}

#[test]
// source帮助暴露git预览和确认层级。
fn source_help_exposes_git_preview_and_confirm() {
    let output = procora(Path::new("."), &["source", "git", "--help"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("preview"));
    assert!(stdout.contains("confirm"));
}

#[test]
// git预览和确认不启动task且拒绝前移引用的旧修订。
fn git_preview_and_confirm_do_not_start_tasks_or_accept_stale_refs() {
    let root = temporary_directory("workflow");
    let repository = root.join("repository");
    let home = root.join("home");
    fs::create_dir(&home).unwrap();
    initialize_repository(&repository);
    commit_definition(&repository, "first");
    let repository_text = repository.to_string_lossy();
    let preview = procora(
        &home,
        &[
            "source",
            "git",
            "preview",
            &repository_text,
            "--local",
            "--config",
            "service/procora.yaml",
        ],
    );
    assert!(
        preview.status.success(),
        "{}",
        String::from_utf8_lossy(&preview.stderr)
    );
    let revision = output_field(&preview, "修订：");
    assert_eq!(revision.len(), 64);
    assert!(String::from_utf8_lossy(&preview.stdout).contains("未启动 Task"));
    assert!(!directory_contains(&home, "SHOULD_NOT_EXIST"));

    let confirmed = procora(
        &home,
        &[
            "source",
            "git",
            "confirm",
            &repository_text,
            &revision,
            "--local",
            "--config",
            "service/procora.yaml",
        ],
    );
    assert!(
        confirmed.status.success(),
        "{}",
        String::from_utf8_lossy(&confirmed.stderr)
    );
    assert!(String::from_utf8_lossy(&confirmed.stdout).contains("确认完成"));
    assert!(!directory_contains(&home, "SHOULD_NOT_EXIST"));

    commit_definition(&repository, "second");
    let stale = procora(
        &home,
        &[
            "source",
            "git",
            "confirm",
            &repository_text,
            &revision,
            "--local",
            "--config",
            "service/procora.yaml",
        ],
    );
    assert!(!stale.status.success());
    assert!(String::from_utf8_lossy(&stale.stderr).contains("修订已变化"));
    assert!(!directory_contains(&home, "SHOULD_NOT_EXIST"));
    fs::remove_dir_all(root).unwrap();
}

/// 递归判断目录中是否出现指定文件名。
fn directory_contains(root: &Path, name: &str) -> bool {
    let Ok(entries) = fs::read_dir(root) else {
        return false;
    };
    entries.filter_map(Result::ok).any(|entry| {
        entry.file_name() == name
            || (entry.path().is_dir() && directory_contains(&entry.path(), name))
    })
}
