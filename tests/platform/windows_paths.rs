//! Windows 路径展示与系统内建命令回归测试。

#![cfg(windows)]

use std::{
    ffi::OsString,
    fs,
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};

use procora::{
    config::discover_path,
    daemon::ServiceHost,
    platform::autostart::DaemonAutostart,
    platform::{canonicalize, current_dir, current_exe, simplify_path, temp_dir},
    protocol::{SnapshotSourceDto, TaskStatusDto},
};
use uuid::Uuid;

/// 创建当前测试独占的临时服务目录。
fn temporary_service(label: &str) -> PathBuf {
    let directory =
        std::env::temp_dir().join(format!("procora-windows-{label}-{}", Uuid::new_v4()));
    fs::create_dir_all(&directory).unwrap();
    directory
}

/// 解码 Windows Script Host 使用的 UTF-16LE 启动脚本。
fn decode_windows_launcher_script(definition: &DaemonAutostart) -> String {
    let bytes = definition.windows_launcher_script();
    assert_eq!(&bytes[..2], &[0xff, 0xfe]);
    let units = bytes[2..]
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect::<Vec<_>>();
    String::from_utf16(&units).unwrap()
}

#[test]
// 扩展驱动器与unc路径转换为常规展示路径。
fn extended_drive_and_unc_paths_are_simplified() {
    assert_eq!(
        simplify_path(std::path::Path::new(r"\\?\C:\Users\tester\service")),
        PathBuf::from(r"C:\Users\tester\service")
    );
    assert_eq!(
        simplify_path(std::path::Path::new(r"\\?\UNC\server\share\service")),
        PathBuf::from(r"\\server\share\service")
    );
    assert_eq!(
        simplify_path(std::path::Path::new(r"\\?\unc\server\share\service")),
        PathBuf::from(r"\\server\share\service")
    );
    let ordinary = PathBuf::from(r"C:\Users\tester\service");
    assert_eq!(simplify_path(&simplify_path(&ordinary)), ordinary);
}

#[test]
// 设备路径和驱动器相对路径不能被错误降级。
fn device_and_drive_relative_paths_are_preserved() {
    for value in [
        r"\\.\PhysicalDrive0",
        r"\\?\Volume{00000000-0000-0000-0000-000000000000}\service",
        r"\\?\C:relative",
    ] {
        assert_eq!(
            simplify_path(std::path::Path::new(value)),
            PathBuf::from(value)
        );
    }
}

#[test]
// UTF-16中文和未配对代理项都不经过UTF-8有损往返。
fn unicode_and_non_utf8_wide_paths_are_preserved() {
    let chinese = PathBuf::from(r"C:\工具\服务\进程.exe");
    assert_eq!(simplify_path(&chinese), chinese);

    let raw = [
        67_u16, 58, 92, 116, 111, 111, 108, 115, 92, 0xD800, 92, 97, 112, 112,
    ];
    let unusual = PathBuf::from(OsString::from_wide(&raw));
    let simplified = simplify_path(&unusual);
    assert_eq!(
        simplified.as_os_str().encode_wide().collect::<Vec<_>>(),
        raw
    );
}

#[test]
// 平台目录和可执行文件入口不会泄漏Windows扩展前缀。
fn platform_path_sources_hide_windows_verbatim_prefix() {
    for path in [
        current_dir().unwrap(),
        current_exe().unwrap(),
        temp_dir(),
        canonicalize(".").unwrap(),
    ] {
        assert!(
            !path.to_string_lossy().starts_with(r"\\?\"),
            "路径仍包含 Windows 扩展前缀：{}",
            path.display()
        );
    }
}

#[test]
// Windows计划任务动作与启动脚本都不会传播扩展路径前缀。
fn windows_task_action_hides_verbatim_prefix() {
    let definition = DaemonAutostart::new(
        r"\\?\C:\Program Files\Procora\procora.exe",
        "procora-center-test",
        r"\\?\C:\ProgramData\Procora\procora.sqlite3",
    );
    let action = definition.windows_task_action();
    let script = decode_windows_launcher_script(&definition);

    assert!(!action.contains(r"\\?\"));
    assert!(!script.contains(r"\\?\"));
    assert!(action.contains(r"C:\ProgramData\Procora\center-start.vbs"));
    assert!(script.contains(r"C:\Program Files\Procora\procora.exe"));
    assert!(script.contains(r"C:\ProgramData\Procora\procora.sqlite3"));
}

#[test]
// 计划任务动作与UTF-16脚本保留中文参数，不依赖活动代码页。
fn windows_task_action_preserves_chinese_paths() {
    let definition = DaemonAutostart::new(
        r"C:\程序\Procora 工具\procora.exe",
        "procora-center-北京",
        r"C:\数据\Procora\中心.sqlite3",
    );
    let action = definition.windows_task_action();
    let script = decode_windows_launcher_script(&definition);

    assert!(action.contains(r"C:\数据\Procora\center-start.vbs"));
    assert!(script.contains(r"C:\程序\Procora 工具\procora.exe"));
    assert!(script.contains("procora-center-北京"));
    assert!(script.contains(r"C:\数据\Procora\中心.sqlite3"));
}

#[test]
// 配置发现不会暴露windows扩展路径前缀。
fn config_discovery_hides_windows_verbatim_prefix() {
    let service = temporary_service("path");
    fs::write(
        service.join("procora.yaml"),
        "version: 1\nproject: windows-path\ntasks: {}\n",
    )
    .unwrap();

    let discovered = discover_path(&service).unwrap();
    assert!(!discovered.root.to_string_lossy().starts_with(r"\\?\"));
    assert!(
        !discovered
            .config_path
            .to_string_lossy()
            .starts_with(r"\\?\")
    );

    fs::remove_dir_all(service).unwrap();
}

#[test]
// 配置工作目录和外部程序路径都会在运行边界清理扩展前缀。
fn configured_cwd_and_program_hide_windows_verbatim_prefix() {
    let service = temporary_service("configured-boundary");
    let canonical_service = canonicalize(&service).unwrap();
    let executable = current_exe().unwrap();
    let extended_service = format!(r"\\?\{}", canonical_service.display());
    let extended_executable = format!(r"\\?\{}", executable.display());
    let config = service.join("procora.json");
    fs::write(
        &config,
        serde_json::json!({
            "version": 1,
            "project": "windows-configured-path",
            "tasks": {
                "probe": {
                    "command": extended_executable,
                    "args": ["--help"],
                    "cwd": extended_service
                }
            }
        })
        .to_string(),
    )
    .unwrap();

    let discovered = discover_path(&config).unwrap();
    let task = discovered.compiled.spec.tasks.values().next().unwrap();
    assert_eq!(task.cwd.as_deref(), Some(canonical_service.as_path()));
    assert!(!task.command.starts_with(r"\\?\"));

    let mut host = ServiceHost::from_compiled_at(discovered.compiled, &discovered.root);
    host.start().unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let snapshot = host.snapshot(SnapshotSourceDto::CenterLive, true);
        if snapshot
            .tasks
            .iter()
            .all(|task| task.status == TaskStatusDto::Stopped)
        {
            assert!(
                snapshot
                    .tasks
                    .iter()
                    .all(|task| task.message.as_deref() == Some("Task 已退出，退出码 0"))
            );
            break;
        }
        assert!(Instant::now() < deadline, "扩展路径程序没有按时完成");
        thread::sleep(Duration::from_millis(10));
    }
    host.stop().unwrap();
    fs::remove_dir_all(service).unwrap();
}

#[test]
// echo内建命令可完成依赖任务图。
fn echo_builtin_completes_dependency_graph() {
    let service = temporary_service("echo");
    let config = service.join("procora.yaml");
    fs::write(
        &config,
        "version: 1\nproject: download\ntasks:\n  prepare:\n    command: echo\n    args: ['Preparing...']\n  app:\n    command: echo\n    args: ['Running app...']\n    depends_on:\n      prepare:\n        condition: completed_successfully\n",
    )
    .unwrap();
    let discovered = discover_path(&config).unwrap();
    let mut host = ServiceHost::from_compiled_at(discovered.compiled, &discovered.root);

    host.start().unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let snapshot = host.snapshot(SnapshotSourceDto::CenterLive, true);
        if snapshot
            .tasks
            .iter()
            .all(|task| task.status == TaskStatusDto::Stopped)
        {
            assert!(
                snapshot
                    .tasks
                    .iter()
                    .all(|task| task.message.as_deref() == Some("Task 已退出，退出码 0"))
            );
            break;
        }
        assert!(Instant::now() < deadline, "echo 任务图没有按时完成");
        thread::sleep(Duration::from_millis(10));
    }

    assert!(
        fs::read_to_string(service.join(".procora/logs/tasks/prepare.log"))
            .unwrap()
            .contains("Preparing...")
    );
    assert!(
        fs::read_to_string(service.join(".procora/logs/tasks/app.log"))
            .unwrap()
            .contains("Running app...")
    );
    host.stop().unwrap();
    fs::remove_dir_all(service).unwrap();
}
