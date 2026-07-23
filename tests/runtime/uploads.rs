use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    path::PathBuf,
    process::{Command, Output, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

use flate2::{Compression, write::GzEncoder};
use sha2::{Digest, Sha256};

use crate::command_support::{remove_directory_when_released, run_background_cli};

/// 创建当前测试独占的目录。
fn temporary_directory(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory = std::env::temp_dir().join(format!(
        "procora-upload-{label}-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(&directory).unwrap();
    directory
}

/// 创建包含两个普通文件的目录上传归档。
fn directory_archive() -> Vec<u8> {
    let encoder = GzEncoder::new(Vec::new(), Compression::fast());
    let mut archive = tar::Builder::new(encoder);
    let entries = [
        ("app.txt", b"new app".as_slice()),
        ("nested/data.txt", b"data"),
    ];
    for (path, bytes) in entries {
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        archive.append_data(&mut header, path, bytes).unwrap();
    }
    archive.into_inner().unwrap().finish().unwrap()
}

/// 创建符合单文件上传约定的 payload 归档。
fn file_archive(bytes: &[u8]) -> Vec<u8> {
    let encoder = GzEncoder::new(Vec::new(), Compression::fast());
    let mut archive = tar::Builder::new(encoder);
    let mut header = tar::Header::new_gnu();
    header.set_size(bytes.len() as u64);
    header.set_mode(0o600);
    header.set_cksum();
    archive.append_data(&mut header, "payload", bytes).unwrap();
    archive.into_inner().unwrap().finish().unwrap()
}

/// 向隐藏接收器发送一条完整协议流。
fn receive(
    binary: &str,
    home: &std::path::Path,
    target: Option<&str>,
    kind: &str,
    archive: &[u8],
    content_bytes: u64,
    selection: Option<&str>,
) -> Output {
    let digest = format!("{:x}", Sha256::digest(archive));
    let header = serde_json::json!({
        "protocol": 1,
        "target": target,
        "source_kind": kind,
        "archive_bytes": archive.len(),
        "content_bytes": content_bytes,
        "sha256": digest,
    });
    let mut child = Command::new(binary)
        .arg("__receive")
        .env("PROCORA_HOME", home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    writeln!(stdin, "{header}").unwrap();
    stdin.flush().unwrap();
    let mut output_bytes = Vec::new();
    stdout.read_until(b'\n', &mut output_bytes).unwrap();
    let response: serde_json::Value = serde_json::from_slice(&output_bytes).unwrap();
    if response["type"] == "choose" {
        let selection = selection.expect("多个候选目标时测试必须提供选择");
        writeln!(stdin, "{}", serde_json::json!({ "target": selection })).unwrap();
        stdin.flush().unwrap();
        stdout.read_until(b'\n', &mut output_bytes).unwrap();
    }
    stdin.write_all(archive).unwrap();
    drop(stdin);
    stdout.read_to_end(&mut output_bytes).unwrap();
    let mut stderr = Vec::new();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_end(&mut stderr)
        .unwrap();
    Output {
        status: child.wait().unwrap(),
        stdout: output_bytes,
        stderr,
    }
}

#[test]
// 远端接收器校验归档后完整替换声明文件与目录并删除旧内容。
fn receiver_replaces_declared_targets_atomically() {
    let home = temporary_directory("home");
    let service = temporary_directory("service");
    fs::write(
        service.join("procora.yaml"),
        "version: 1\nproject: demo\nuploads:\n  release:\n    path: deployed\n    kind: directory\n    max_bytes: 1024\n  assets:\n    path: public\n    kind: directory\n    max_bytes: 1024\n  config:\n    path: config/app.toml\n    kind: file\n    max_bytes: 1024\ntasks: {}\n",
    )
    .unwrap();
    fs::create_dir(service.join("deployed")).unwrap();
    fs::write(service.join("deployed/stale.txt"), "stale").unwrap();
    let binary = env!("CARGO_BIN_EXE_procora");
    let opened = run_background_cli(
        Command::new(binary)
            .arg("add")
            .arg(&service)
            .env("PROCORA_HOME", &home),
        &home,
        "upload-add",
    );
    assert!(
        opened.status.success(),
        "{}",
        String::from_utf8_lossy(&opened.stderr)
    );

    let archive = directory_archive();
    let output = receive(
        binary,
        &home,
        None,
        "directory",
        &archive,
        11,
        Some("demo::release"),
    );
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::read_to_string(service.join("deployed/app.txt")).unwrap(),
        "new app"
    );
    assert_eq!(
        fs::read_to_string(service.join("deployed/nested/data.txt")).unwrap(),
        "data"
    );
    assert!(!service.join("deployed/stale.txt").exists());

    let archive = file_archive(b"port = 8080\n");
    let output = receive(binary, &home, None, "file", &archive, 12, None);
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::read_to_string(service.join("config/app.toml")).unwrap(),
        "port = 8080\n"
    );

    let down = Command::new(binary)
        .arg("down")
        .env("PROCORA_HOME", &home)
        .output()
        .unwrap();
    assert!(down.status.success());
    remove_directory_when_released(&home);
    fs::remove_dir_all(service).unwrap();
}
