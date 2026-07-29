//! `.pcpkg` 构建、验证、检查和平台物化的 CLI 集成测试。

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    time::{SystemTime, UNIX_EPOCH},
};

use crate::command_support::{remove_directory_when_released, run_background_cli};

/// 创建包含当前与另一平台二进制、普通文件和导出项的测试 Service。
fn write_service(root: &Path) -> (String, String) {
    let current = procora::config::DeployPlatform::current()
        .normalized()
        .unwrap()
        .key();
    let foreign = if current.starts_with("windows-") {
        "linux-x86_64-gnu"
    } else {
        "windows-x86_64-msvc"
    };
    fs::create_dir_all(root.join("dist")).unwrap();
    fs::create_dir_all(root.join("public/嵌套")).unwrap();
    fs::write(
        root.join("procora.yaml"),
        format!(
            r#"version: 1
project: package-demo
binaries:
  api:
    target: bin/api
    variants:
      "{current}": dist/api-current
      "{foreign}":
        source: dist/api-foreign
        target: bin/api-foreign
uploads:
  assets:
    path: public
    kind: directory
tasks:
  api:
    command: "${{binary.api}}"
"#
        ),
    )
    .unwrap();
    fs::write(root.join("dist/api-current"), b"current-binary\n").unwrap();
    fs::write(root.join("dist/api-foreign"), b"foreign-binary\n").unwrap();
    fs::write(root.join("public/嵌套/index.txt"), "内容\n").unwrap();
    fs::write(root.join("README.txt"), "package fixture\n").unwrap();
    fs::create_dir(root.join("ignored")).unwrap();
    fs::write(root.join("ignored/cache.txt"), "ignored\n").unwrap();
    fs::write(root.join("credentials.secret"), "ignored\n").unwrap();
    fs::write(root.join(".procoraignore"), "ignored/\n*.secret\n").unwrap();
    (current, foreign.to_owned())
}

/// 执行 Procora 并保留便于失败诊断的完整输出。
fn run(arguments: &[&str], current_dir: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_procora"))
        .args(arguments)
        .current_dir(current_dir)
        .output()
        .unwrap()
}

/// 创建当前测试独占的临时目录。
fn temporary_directory(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory = std::env::temp_dir().join(format!(
        "procora-package-{label}-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(&directory).unwrap();
    directory
}

#[test]
// 同一输入两次构建得到逐字节一致的胖包，并可检查、验证和按当前平台物化。
fn package_round_trip_is_deterministic_and_platform_aware() {
    let directory = temporary_directory("round-trip");
    let service = directory.join("service");
    fs::create_dir(&service).unwrap();
    let (current, foreign) = write_service(&service);
    let first = directory.join("first.pcpkg");
    let second = directory.join("second.pcpkg");

    for output in [&first, &second] {
        let result = run(
            &[
                "package",
                "build",
                service.to_str().unwrap(),
                "--output",
                output.to_str().unwrap(),
            ],
            &directory,
        );
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    assert_eq!(fs::read(&first).unwrap(), fs::read(&second).unwrap());

    let inspected = run(
        &["package", "inspect", first.to_str().unwrap(), "--json"],
        &directory,
    );
    assert!(
        inspected.status.success(),
        "{}",
        String::from_utf8_lossy(&inspected.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&inspected.stdout).unwrap();
    assert_eq!(json["manifest"]["project"], "package-demo");
    assert!(
        json["manifest"]["binaries"]["api"]["variants"][&current].is_object(),
        "{json}"
    );
    assert!(
        json["manifest"]["binaries"]["api"]["variants"][&foreign].is_object(),
        "{json}"
    );
    assert_eq!(json["manifest"]["exports"]["assets"]["path"], "public");
    let paths = json["manifest"]["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|file| file["path"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(!paths.contains(&"ignored/cache.txt"));
    assert!(!paths.contains(&"credentials.secret"));
    assert!(!paths.contains(&".procoraignore"));

    let verified = run(&["package", "verify", first.to_str().unwrap()], &directory);
    assert!(
        verified.status.success(),
        "{}",
        String::from_utf8_lossy(&verified.stderr)
    );

    let extracted = directory.join("extracted");
    let result = run(
        &[
            "package",
            "extract",
            first.to_str().unwrap(),
            "--output",
            extracted.to_str().unwrap(),
        ],
        &directory,
    );
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        fs::read(extracted.join("bin/api")).unwrap(),
        b"current-binary\n"
    );
    assert!(!extracted.join("bin/api-foreign").exists());
    assert_eq!(
        fs::read_to_string(extracted.join("public/嵌套/index.txt")).unwrap(),
        "内容\n"
    );
    assert!(!extracted.join("dist/api-current").exists());
    remove_directory_when_released(&directory);
}

#[test]
// current薄包只携带本平台变体，默认输出名称来自Service名称。
fn package_build_current_creates_thin_default_output() {
    let directory = temporary_directory("thin");
    let service = directory.join("service");
    fs::create_dir(&service).unwrap();
    let (current, foreign) = write_service(&service);

    let built = run(
        &[
            "package",
            "build",
            service.to_str().unwrap(),
            "--platform",
            "current",
        ],
        &directory,
    );
    assert!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );
    let package = directory.join("package-demo.pcpkg");
    assert!(package.is_file());

    let inspected = run(
        &["package", "inspect", package.to_str().unwrap(), "--json"],
        &directory,
    );
    let json: serde_json::Value = serde_json::from_slice(&inspected.stdout).unwrap();
    let variants = json["manifest"]["binaries"]["api"]["variants"]
        .as_object()
        .unwrap();
    assert_eq!(variants.len(), 1);
    assert!(variants.contains_key(&current));
    assert!(!variants.contains_key(&foreign));
    remove_directory_when_released(&directory);
}

#[test]
// 内容损坏的包不会被验证或解包。
fn package_verify_rejects_corrupted_content() {
    let directory = temporary_directory("corrupt");
    let service = directory.join("service");
    fs::create_dir(&service).unwrap();
    write_service(&service);
    let package = directory.join("demo.pcpkg");
    let built = run(
        &[
            "package",
            "build",
            service.to_str().unwrap(),
            "--output",
            package.to_str().unwrap(),
        ],
        &directory,
    );
    assert!(built.status.success());

    let mut bytes = fs::read(&package).unwrap();
    let index = bytes.len() / 2;
    bytes[index] ^= 0x5a;
    fs::write(&package, bytes).unwrap();
    let verified = run(
        &["package", "verify", package.to_str().unwrap()],
        &directory,
    );
    assert!(!verified.status.success());

    let extracted = directory.join("extracted");
    let result = run(
        &[
            "package",
            "extract",
            package.to_str().unwrap(),
            "--output",
            extracted.to_str().unwrap(),
        ],
        &directory,
    );
    assert!(!result.status.success());
    assert!(!extracted.exists());
    remove_directory_when_released(&directory);
}

#[test]
// 包安装复用全托管不可变release，并允许add命令直觉化接收同一个包。
fn package_install_uses_managed_release_and_add_is_idempotent() {
    let directory = temporary_directory("install");
    let service = directory.join("service");
    let home = directory.join("home");
    fs::create_dir(&service).unwrap();
    fs::write(
        service.join("procora.yaml"),
        "version: 1\nproject: install-demo\ntasks: {}\n",
    )
    .unwrap();
    fs::write(service.join("data.txt"), "installed\n").unwrap();
    let package = directory.join("install-demo.pcpkg");
    let built = run(
        &[
            "package",
            "build",
            service.to_str().unwrap(),
            "--output",
            package.to_str().unwrap(),
        ],
        &directory,
    );
    assert!(built.status.success());

    let installed = run_background_cli(
        Command::new(env!("CARGO_BIN_EXE_procora"))
            .args([
                "package",
                "install",
                package.to_str().unwrap(),
                "--timeout",
                "5s",
                "--stable-for",
                "0ms",
            ])
            .env("PROCORA_HOME", &home),
        &directory,
        "package-install",
    );
    assert!(
        installed.status.success(),
        "{}",
        String::from_utf8_lossy(&installed.stderr)
    );
    assert!(
        String::from_utf8_lossy(&installed.stdout).contains("安装完成：install-demo"),
        "{}",
        String::from_utf8_lossy(&installed.stdout)
    );
    let state: serde_json::Value =
        serde_json::from_slice(&fs::read(home.join("services/install-demo/state.json")).unwrap())
            .unwrap();
    let active = state["active_release"].as_str().unwrap();
    let release = home.join("services/install-demo/releases").join(active);
    assert_eq!(
        fs::read_to_string(release.join("data.txt")).unwrap(),
        "installed\n"
    );
    assert_eq!(state["deployments"][0]["outcome"], "succeeded");
    assert_eq!(
        fs::read_dir(home.join("services/install-demo/packages"))
            .unwrap()
            .count(),
        1
    );

    let repeated = run_background_cli(
        Command::new(env!("CARGO_BIN_EXE_procora"))
            .args(["add", package.to_str().unwrap()])
            .env("PROCORA_HOME", &home),
        &directory,
        "package-add",
    );
    assert!(
        repeated.status.success(),
        "{}",
        String::from_utf8_lossy(&repeated.stderr)
    );
    assert!(
        String::from_utf8_lossy(&repeated.stdout).contains("无需更新：install-demo"),
        "{}",
        String::from_utf8_lossy(&repeated.stdout)
    );

    let down = run_background_cli(
        Command::new(env!("CARGO_BIN_EXE_procora"))
            .arg("down")
            .env("PROCORA_HOME", &home),
        &directory,
        "package-down",
    );
    assert!(down.status.success());
    remove_directory_when_released(&directory);
}

#[cfg(unix)]
#[test]
// deploy探测远端后只从胖包物化匹配平台，并沿用现有SSH托管部署协议。
fn package_deploy_selects_remote_platform_without_leaking_other_binaries() {
    use std::io::Read;

    use flate2::read::GzDecoder;

    let directory = temporary_directory("deploy");
    crate::cli_uploads::install_fake_ssh(&directory);
    let service = directory.join("service");
    fs::create_dir_all(service.join("dist")).unwrap();
    fs::write(
        service.join("procora.yaml"),
        r"version: 1
project: demo
binaries:
  api:
    target: bin/api
    variants:
      linux-amd64: dist/api-linux
      macos-universal: dist/api-macos
      windows-amd64:
        source: dist/api-windows.exe
        target: bin/api.exe
tasks: {}
",
    )
    .unwrap();
    fs::write(service.join("dist/api-linux"), "linux\n").unwrap();
    fs::write(service.join("dist/api-macos"), "macos\n").unwrap();
    fs::write(service.join("dist/api-windows.exe"), "windows\n").unwrap();
    let package = directory.join("demo.pcpkg");
    let built = run(
        &[
            "package",
            "build",
            service.to_str().unwrap(),
            "--output",
            package.to_str().unwrap(),
        ],
        &directory,
    );
    assert!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );

    let path = format!(
        "{}:{}",
        directory.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let archive_path = directory.join("deploy.tar.gz");
    let deployed = Command::new(env!("CARGO_BIN_EXE_procora"))
        .args([
            "deploy",
            package.to_str().unwrap(),
            "--ssh",
            "package-host",
            "--batch",
            "--timeout",
            "1s",
            "--stable-for",
            "0ms",
        ])
        .env("PATH", path)
        .env("PROCORA_HOME", directory.join("home"))
        .env("FAKE_SSH_LOG", directory.join("ssh.log"))
        .env("FAKE_SSH_HEADER_LOG", directory.join("ssh-header.log"))
        .env("FAKE_SSH_ARCHIVE_LOG", &archive_path)
        .output()
        .unwrap();
    assert!(
        deployed.status.success(),
        "{}",
        String::from_utf8_lossy(&deployed.stderr)
    );
    let header = fs::read_to_string(directory.join("ssh-header.log")).unwrap();
    assert!(header.contains(r#""target_platform":{"os":"linux","arch":"x86_64""#));
    assert!(header.contains(r#""selector":"linux-x86_64""#));

    let decoder = GzDecoder::new(fs::File::open(archive_path).unwrap());
    let mut archive = tar::Archive::new(decoder);
    let mut files = std::collections::BTreeMap::new();
    for entry in archive.entries().unwrap() {
        let mut entry = entry.unwrap();
        if entry.header().entry_type().is_file() {
            let path = entry.path().unwrap().to_string_lossy().into_owned();
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes).unwrap();
            files.insert(path, bytes);
        }
    }
    assert_eq!(files["bin/api"], b"linux\n");
    assert!(!files.contains_key("bin/api.exe"));
    assert!(!files.keys().any(|path| path.starts_with("dist/")));
    remove_directory_when_released(&directory);
}

#[cfg(unix)]
#[test]
// push可直接选择包清单导出项，并默认映射到同名Service上传目标。
fn package_entry_push_materializes_only_exported_content() {
    use std::io::Read;

    use flate2::read::GzDecoder;

    let directory = temporary_directory("push-entry");
    crate::cli_uploads::install_fake_ssh(&directory);
    let service = directory.join("service");
    fs::create_dir(&service).unwrap();
    write_service(&service);
    let package = directory.join("package-demo.pcpkg");
    let built = run(
        &[
            "package",
            "build",
            service.to_str().unwrap(),
            "--output",
            package.to_str().unwrap(),
        ],
        &directory,
    );
    assert!(built.status.success());

    let path = format!(
        "{}:{}",
        directory.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let archive_path = directory.join("push.tar.gz");
    let pushed = Command::new(env!("CARGO_BIN_EXE_procora"))
        .args([
            "push",
            package.to_str().unwrap(),
            "--package-entry",
            "assets",
            "--ssh",
            "package-host",
            "--batch",
        ])
        .env("PATH", path)
        .env("FAKE_SSH_LOG", directory.join("ssh.log"))
        .env("FAKE_SSH_HEADER_LOG", directory.join("ssh-header.log"))
        .env("FAKE_SSH_ARCHIVE_LOG", &archive_path)
        .output()
        .unwrap();
    assert!(
        pushed.status.success(),
        "{}",
        String::from_utf8_lossy(&pushed.stderr)
    );
    let header = fs::read_to_string(directory.join("ssh-header.log")).unwrap();
    assert!(header.contains(r#""target":"package-demo::assets""#));
    assert!(header.contains(r#""source_kind":"directory""#));

    let decoder = GzDecoder::new(fs::File::open(archive_path).unwrap());
    let mut archive = tar::Archive::new(decoder);
    let mut files = std::collections::BTreeMap::new();
    for entry in archive.entries().unwrap() {
        let mut entry = entry.unwrap();
        if entry.header().entry_type().is_file() {
            let path = entry.path().unwrap().to_string_lossy().into_owned();
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes).unwrap();
            files.insert(path, bytes);
        }
    }
    assert_eq!(files.len(), 1);
    assert_eq!(files["嵌套/index.txt"], "内容\n".as_bytes());
    remove_directory_when_released(&directory);
}

#[cfg(unix)]
#[test]
// 构建阶段拒绝会在Windows上改变含义的保留文件名。
fn package_build_rejects_windows_reserved_paths() {
    let directory = temporary_directory("reserved-path");
    let service = directory.join("service");
    fs::create_dir(&service).unwrap();
    fs::write(
        service.join("procora.yaml"),
        "version: 1\nproject: reserved-demo\ntasks: {}\n",
    )
    .unwrap();
    fs::write(service.join("CON.txt"), "not portable\n").unwrap();
    let package = directory.join("reserved.pcpkg");
    let built = run(
        &[
            "package",
            "build",
            service.to_str().unwrap(),
            "--output",
            package.to_str().unwrap(),
        ],
        &directory,
    );
    assert!(!built.status.success());
    assert!(
        String::from_utf8_lossy(&built.stderr).contains("Windows 保留名称"),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );
    assert!(!package.exists());
    remove_directory_when_released(&directory);
}

#[cfg(unix)]
#[test]
// 包不跟随符号链接，避免把Service根目录外内容带入产物。
fn package_build_rejects_symlinks() {
    use std::os::unix::fs::symlink;

    let directory = temporary_directory("symlink");
    let service = directory.join("service");
    fs::create_dir(&service).unwrap();
    fs::write(
        service.join("procora.yaml"),
        "version: 1\nproject: symlink-demo\ntasks: {}\n",
    )
    .unwrap();
    fs::write(directory.join("outside.txt"), "outside\n").unwrap();
    symlink(directory.join("outside.txt"), service.join("linked.txt")).unwrap();
    let package = directory.join("symlink.pcpkg");
    let built = run(
        &[
            "package",
            "build",
            service.to_str().unwrap(),
            "--output",
            package.to_str().unwrap(),
        ],
        &directory,
    );
    assert!(!built.status.success());
    assert!(
        String::from_utf8_lossy(&built.stderr).contains("不支持符号链接"),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );
    assert!(!package.exists());
    remove_directory_when_released(&directory);
}
