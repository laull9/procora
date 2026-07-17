//! 受控 Python 配置辅助进程的信任边界与故障隔离测试。

#![cfg(unix)]

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use procora::{config::PythonConfigRunner, source::LocalFileSource};

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
        "procora-python-{label}-{}-{nonce}-{sequence}",
        std::process::id()
    ));
    fs::create_dir_all(&directory).unwrap();
    directory
}

/// 创建会把最后一个参数当作 shell 脚本执行的可控假解释器。
fn fake_interpreter(root: &Path) -> PathBuf {
    let path = root.join("fake-python");
    fs::write(
        &path,
        "#!/bin/sh\nscript=''\nfor arg in \"$@\"; do script=$arg; done\nexec /bin/sh \"$script\"\n",
    )
    .unwrap();
    let mut permissions = fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(&path, permissions).unwrap();
    path
}

/// 写入由假解释器执行的 `procora.py`。
fn write_script(root: &Path, content: &str) -> PathBuf {
    let path = root.join("procora.py");
    fs::write(&path, content).unwrap();
    path
}

/// 返回一个最小有效的生成配置 JSON。
fn valid_json(project: &str) -> String {
    format!(
        r#"{{"version":1,"project":"{project}","tasks":{{"api":{{"command":"echo","cwd":"."}}}}}}"#
    )
}

#[test]
// 显式入口通过共享校验并按脚本目录规范化路径。
fn explicit_python_entry_uses_shared_validation_and_script_directory() {
    let root = temporary_directory("success");
    let interpreter = fake_interpreter(&root);
    let script = write_script(&root, &format!("printf '%s' '{}'\n", valid_json("demo")));

    let compiled = PythonConfigRunner::new(interpreter).load(&script).unwrap();

    assert_eq!(compiled.spec.project, "demo");
    assert_eq!(
        compiled.spec.tasks.values().next().unwrap().cwd.as_deref(),
        Some(fs::canonicalize(&root).unwrap().as_path())
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
// 辅助进程不继承宿主环境变量。
fn python_helper_does_not_inherit_host_environment() {
    let root = temporary_directory("environment");
    let interpreter = fake_interpreter(&root);
    let script = write_script(
        &root,
        concat!(
            "if [ -z \"${HOME+x}\" ]; then project=clean; else project=leaked; fi\n",
            "printf '{\"version\":1,\"project\":\"%s\",\"tasks\":{}}' \"$project\"\n",
        ),
    );

    let compiled = PythonConfigRunner::new(interpreter).load(&script).unwrap();

    assert_eq!(compiled.spec.project, "clean");
    fs::remove_dir_all(root).unwrap();
}

#[test]
// 非零退出和严格单文档解析提供可操作诊断。
fn python_failures_and_multi_document_output_are_actionable() {
    let root = temporary_directory("diagnostics");
    let interpreter = fake_interpreter(&root);
    let script = write_script(&root, "printf 'boom' >&2\nexit 7\n");
    let error = PythonConfigRunner::new(&interpreter)
        .load(&script)
        .unwrap_err()
        .to_string();
    assert!(error.contains("boom"));
    assert!(error.contains("退出"));

    fs::write(&script, "printf '{}{}'\n").unwrap();
    let error = PythonConfigRunner::new(interpreter)
        .load(&script)
        .unwrap_err()
        .to_string();
    assert!(error.contains("单个 JSON 文档"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
// 超时会回收脚本创建的整个进程树。
fn python_timeout_reclaims_entire_process_tree() {
    let root = temporary_directory("timeout");
    let interpreter = fake_interpreter(&root);
    let script = write_script(&root, "sleep 30 &\necho $! > child.pid\nsleep 30\n");
    let started = Instant::now();

    let error = PythonConfigRunner::new(interpreter)
        .with_timeout(Duration::from_secs(5))
        .load(&script)
        .unwrap_err()
        .to_string();

    assert!(error.contains("执行超过"));
    assert!(started.elapsed() < Duration::from_secs(7));
    let pid = fs::read_to_string(root.join("child.pid")).unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while process_exists(pid.trim()) && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20));
    }
    assert!(!process_exists(pid.trim()), "超时后代进程仍然存活");
    fs::remove_dir_all(root).unwrap();
}

#[test]
// stdout和脚本体积都有硬上限。
fn python_stdout_and_script_size_have_hard_limits() {
    let root = temporary_directory("limits");
    let interpreter = fake_interpreter(&root);
    let script = write_script(&root, "dd if=/dev/zero bs=1048577 count=1 2>/dev/null\n");
    let error = PythonConfigRunner::new(&interpreter)
        .load(&script)
        .unwrap_err()
        .to_string();
    assert!(error.contains("stdout 超过"));

    fs::write(&script, vec![b'#'; 1024 * 1024 + 1]).unwrap();
    let error = PythonConfigRunner::new(interpreter)
        .load(&script)
        .unwrap_err()
        .to_string();
    assert!(error.contains("脚本超过"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
// 生成输出参与修订以拒绝脚本外部输入导致的过期应用。
fn generated_output_revision_rejects_stale_external_input() {
    let root = temporary_directory("revision");
    let script = write_script(
        &root,
        "from pathlib import Path\nprint(Path('generated.json').read_text())\n",
    );
    fs::write(root.join("generated.json"), valid_json("first")).unwrap();
    let source = LocalFileSource::new(&script);
    let first = source.read_candidate();
    assert!(first.compiled.is_ok());

    fs::write(root.join("generated.json"), valid_json("second")).unwrap();
    let second = source.read_candidate();
    assert!(second.compiled.is_ok());

    assert_ne!(first.revision, second.revision);
    fs::remove_dir_all(root).unwrap();
}

#[test]
// Python显式声明的环境文件参与规范化配置与修订。
fn generated_env_file_participates_in_revision() {
    let root = temporary_directory("env-file-revision");
    let generated = r#"{"version":1,"project":"demo","tasks":{"api":{"command":"echo","env_file":"task.env"}}}"#;
    let script = write_script(&root, &format!("print({generated:?})\n"));
    fs::write(root.join("task.env"), "VALUE=first\n").unwrap();
    let source = LocalFileSource::new(&script);
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

    fs::write(root.join("task.env"), "VALUE=second\n").unwrap();
    let second = source.read_candidate();
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
// Python生成JSON与声明式格式共享项目默认环境和命令文本语义。
fn generated_json_supports_project_env_and_command_text() {
    let root = temporary_directory("config-usability");
    let interpreter = fake_interpreter(&root);
    let generated = include_str!("fixtures/config/equivalent/python-output.json");
    let script = write_script(&root, &format!("printf '%s' '{generated}'\n"));

    let compiled = PythonConfigRunner::new(interpreter).load(&script).unwrap();
    let task = &compiled.spec.tasks[&"api".parse().unwrap()];
    assert_eq!(compiled.project_env["PROJECT"], "global");
    assert_eq!(task.env["COMMON"], "shared");
    assert_eq!(task.command, "api");
    assert_eq!(task.args, ["--port", "8080"]);
    assert_eq!(task.restart, procora::core::RestartPolicy::OnFailure);
    assert_eq!(task.max_restarts, 4);
    fs::remove_dir_all(root).unwrap();
}

/// 判断指定 Unix 进程是否仍可被信号探测。
fn process_exists(pid: &str) -> bool {
    Command::new("kill")
        .args(["-0", pid])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}
