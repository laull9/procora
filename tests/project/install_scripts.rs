//! 安装、卸载脚本与公开文档地址的契约测试。

#[cfg(unix)]
use std::{
    env, fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

#[cfg(unix)]
use sha2::{Digest, Sha256};

/// Unix 安装脚本文本。
const INSTALL_SH: &str = include_str!("../../scripts/install.sh");

/// Windows 安装脚本文本。
const INSTALL_PS1: &str = include_str!("../../scripts/install.ps1");

/// Unix 卸载脚本文本。
const UNINSTALL_SH: &str = include_str!("../../scripts/uninstall.sh");

/// Windows 卸载脚本文本。
const UNINSTALL_PS1: &str = include_str!("../../scripts/uninstall.ps1");

/// 根目录公开说明文本。
const README: &str = include_str!("../../README.md");

#[test]
// 公开脚本统一指向真实仓库和raw地址。
fn public_scripts_use_real_repository_and_raw_urls() {
    for script in [INSTALL_SH, INSTALL_PS1] {
        assert!(script.contains("laull9/procora"));
        assert!(!script.contains("laull/procora"));
    }
    for script in [UNINSTALL_SH, UNINSTALL_PS1] {
        assert!(script.contains("disable"));
        assert!(script.contains("PROCORA_FORCE_UNINSTALL"));
    }
    assert!(README.contains("raw.githubusercontent.com/laull9/procora/main/scripts/install.sh"));
    assert!(README.contains("raw.githubusercontent.com/laull9/procora/main/scripts/uninstall.sh"));
    assert!(!README.contains("/blob/main/scripts/"));
    assert!(!README.contains("raw.githubusercontent.com/laull/procora"));
}

#[cfg(unix)]
#[test]
// Unix安装与卸载脚本形成可离线验证的完整闭环。
fn unix_install_and_uninstall_scripts_form_offline_lifecycle() {
    let root = temporary_directory("install scripts");
    let assets = root.join("assets");
    let payload = root.join("payload");
    let mock_bin = root.join("mock-bin");
    let install_dir = root.join("install dir");
    fs::create_dir_all(&assets).unwrap();
    fs::create_dir_all(&payload).unwrap();
    fs::create_dir_all(&mock_bin).unwrap();

    let payload_binary = payload.join("procora");
    write_executable(&payload_binary, "#!/bin/sh\nprintf 'procora fixture\\n'\n");
    let asset_name = "procora-x86_64-unknown-linux-musl.tar.gz";
    let asset = assets.join(asset_name);
    let archived = Command::new("tar")
        .args(["-C", payload.to_str().unwrap(), "-czf"])
        .arg(&asset)
        .arg("procora")
        .output()
        .unwrap();
    assert!(
        archived.status.success(),
        "{}",
        String::from_utf8_lossy(&archived.stderr)
    );
    let digest = Sha256::digest(fs::read(&asset).unwrap());
    fs::write(
        assets.join(format!("{asset_name}.sha256")),
        format!("{digest:x}  {asset_name}\n"),
    )
    .unwrap();

    write_executable(
        &mock_bin.join("uname"),
        "#!/bin/sh\ncase \"$1\" in\n  -s) printf 'Linux\\n' ;;\n  -m) printf 'x86_64\\n' ;;\nesac\n",
    );
    write_executable(
        &mock_bin.join("curl"),
        "#!/bin/sh\nset -eu\noutput=\nurl=\nwhile [ \"$#\" -gt 0 ]; do\n  case \"$1\" in\n    --output) shift; output=$1 ;;\n    https://*) url=$1 ;;\n  esac\n  shift\ndone\nprintf '%s\\n' \"$url\" >> \"$PROCORA_TEST_URL_LOG\"\ncp \"$PROCORA_TEST_ASSET_DIR/${url##*/}\" \"$output\"\n",
    );

    let path = env::join_paths(
        std::iter::once(mock_bin.clone()).chain(env::split_paths(&env::var_os("PATH").unwrap())),
    )
    .unwrap();
    let url_log = root.join("urls.log");
    let installed = Command::new("sh")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/install.sh"))
        .env("PATH", path)
        .env("PROCORA_INSTALL_DIR", &install_dir)
        .env("PROCORA_TEST_ASSET_DIR", &assets)
        .env("PROCORA_TEST_URL_LOG", &url_log)
        .output()
        .unwrap();
    assert!(
        installed.status.success(),
        "{}",
        String::from_utf8_lossy(&installed.stderr)
    );

    let binary = install_dir.join("procora");
    assert_eq!(
        fs::read_to_string(&binary).unwrap(),
        fs::read_to_string(&payload_binary).unwrap()
    );
    assert_ne!(
        fs::metadata(&binary).unwrap().permissions().mode() & 0o111,
        0
    );
    let requested_urls = fs::read_to_string(url_log).unwrap();
    assert!(requested_urls.contains("github.com/laull9/procora/releases/latest/download"));

    assert_uninstall_safety(&root, &install_dir, &binary);

    fs::remove_dir_all(root).unwrap();
}

/// 验证卸载器先停用托管，并只在显式强制时忽略失败。
#[cfg(unix)]
fn assert_uninstall_safety(root: &Path, install_dir: &Path, binary: &Path) {
    let disable_log = root.join("disable.log");
    write_executable(
        binary,
        "#!/bin/sh\nprintf '%s\\n' \"$1\" > \"$PROCORA_TEST_DISABLE_LOG\"\n",
    );
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/uninstall.sh");
    let uninstalled = Command::new("sh")
        .arg(&script)
        .env("PROCORA_INSTALL_DIR", install_dir)
        .env("PROCORA_TEST_DISABLE_LOG", &disable_log)
        .output()
        .unwrap();
    assert!(
        uninstalled.status.success(),
        "{}",
        String::from_utf8_lossy(&uninstalled.stderr)
    );
    assert!(!binary.exists());
    assert_eq!(fs::read_to_string(disable_log).unwrap(), "disable\n");

    write_executable(binary, "#!/bin/sh\nexit 9\n");
    let refused = Command::new("sh")
        .arg(&script)
        .env("PROCORA_INSTALL_DIR", install_dir)
        .output()
        .unwrap();
    assert!(!refused.status.success());
    assert!(binary.exists());

    let forced = Command::new("sh")
        .arg(script)
        .env("PROCORA_INSTALL_DIR", install_dir)
        .env("PROCORA_FORCE_UNINSTALL", "1")
        .output()
        .unwrap();
    assert!(forced.status.success());
    assert!(!binary.exists());
}

/// 创建当前测试独占的临时目录。
#[cfg(unix)]
fn temporary_directory(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory = env::temp_dir().join(format!("procora-{label}-{}-{nonce}", std::process::id()));
    fs::create_dir_all(&directory).unwrap();
    directory
}

/// 写入 Unix 可执行脚本。
#[cfg(unix)]
fn write_executable(path: &Path, content: &str) {
    fs::write(path, content).unwrap();
    let mut permissions = fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).unwrap();
}
