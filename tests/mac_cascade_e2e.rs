//! macOS 本机中心服务、HTTP 依赖和任务级联完整联调。

#![cfg(target_os = "macos")]

use std::{
    collections::BTreeMap,
    fs,
    io::{Cursor, Read, Write},
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Command, Output},
    thread,
    time::{Duration, Instant},
};

use flate2::{Compression, write::GzEncoder};
use sha2::{Digest, Sha256};
use uuid::Uuid;

/// 保证测试成功或断言失败时都停止隔离中心并删除临时目录。
struct TestSandbox {
    service: PathBuf,
    home: PathBuf,
    binary: &'static str,
}

impl TestSandbox {
    /// 创建一组独占的服务、中心和二进制测试上下文。
    fn new() -> Self {
        Self {
            service: temporary_directory("service"),
            home: temporary_directory("home"),
            binary: env!("CARGO_BIN_EXE_procora"),
        }
    }
}

impl Drop for TestSandbox {
    /// 尽力停止中心并回收本次测试创建的全部文件。
    fn drop(&mut self) {
        let _ = Command::new(self.binary)
            .arg("down")
            .env("PROCORA_HOME", &self.home)
            .output();
        let _ = fs::remove_dir_all(&self.home);
        let _ = fs::remove_dir_all(&self.service);
    }
}

/// 创建测试独占的服务目录或中心目录。
fn temporary_directory(label: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("procora-mac-e2e-{label}-{}", Uuid::new_v4()));
    fs::create_dir_all(&path).unwrap();
    path
}

/// 构造带可执行权限的远程 runner tar.gz。
fn runner_archive() -> Vec<u8> {
    let script = br#"#!/bin/sh
set -eu
case "$1" in
  --version)
    echo "cascade-runner 3.1.4"
    ;;
  bootstrap)
    assets="$2"
    output="$3"
    mkdir -p "$output"
    cp "$assets/payload.txt" "$output/prepared.txt"
    cp "$REMOTE_CONFIG" "$output/remote.conf"
    pwd > "$output/cwd.txt"
    echo "bootstrap-complete"
    ;;
  worker)
    output="$2"
    echo "worker-started" >> "$output/events.log"
    trap 'echo worker-stopped >> "$output/events.log"; exit 0' TERM INT
    while :; do sleep 1; done
    ;;
  started-gate)
    output="$2"
    test -f "$output/prepared.txt"
    echo "started-gate-complete" >> "$output/events.log"
    ;;
  healthy-gate)
    output="$2"
    test -f "$output/prepared.txt"
    echo "healthy-gate-complete" >> "$output/events.log"
    ;;
  retry)
    output="$2"
    if [ ! -f "$output/retry.marker" ]; then
      touch "$output/retry.marker"
      echo "retry-first-failure" >&2
      exit 9
    fi
    echo "retry-recovered"
    touch "$output/retry.done"
    ;;
  finalize)
    output="$2"
    grep -q "downloaded asset payload" "$output/prepared.txt"
    grep -q "remote configuration v5" "$output/remote.conf"
    test -f "$output/retry.done"
    echo "final-complete" >> "$output/events.log"
    ;;
  *)
    echo "unknown mode: $1" >&2
    exit 64
    ;;
esac
"#;
    let encoder = GzEncoder::new(Vec::new(), Compression::default());
    let mut archive = tar::Builder::new(encoder);
    let mut header = tar::Header::new_gnu();
    header.set_size(script.len() as u64);
    header.set_mode(0o755);
    header.set_cksum();
    archive
        .append_data(&mut header, "package/bin/runner", &script[..])
        .unwrap();
    archive.into_inner().unwrap().finish().unwrap()
}

/// 构造包含目录资源的远程 ZIP。
fn assets_archive() -> Vec<u8> {
    let cursor = Cursor::new(Vec::new());
    let mut archive = zip::ZipWriter::new(cursor);
    archive
        .start_file(
            "bundle/payload.txt",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
    archive.write_all(b"downloaded asset payload\n").unwrap();
    archive.finish().unwrap().into_inner()
}

/// 返回内容的十六进制 SHA-256。
fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// 启动只监听回环地址的多文件 HTTP 下载源。
fn download_server(
    expected_requests: usize,
) -> (
    String,
    thread::JoinHandle<Vec<String>>,
    BTreeMap<String, String>,
) {
    let runner = runner_archive();
    let assets = assets_archive();
    let remote = b"remote configuration v5\n".to_vec();
    let checksums = BTreeMap::from([
        ("runner".to_owned(), sha256(&runner)),
        ("assets".to_owned(), sha256(&assets)),
        ("remote".to_owned(), sha256(&remote)),
    ]);
    let payloads = BTreeMap::from([
        ("/runner.tar.gz".to_owned(), runner),
        ("/assets.zip".to_owned(), assets),
        ("/remote.conf".to_owned(), remote),
    ]);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let server = thread::spawn(move || {
        let mut requests = Vec::new();
        for _ in 0..expected_requests {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = [0_u8; 4096];
            let read = stream.read(&mut request).unwrap();
            let request = String::from_utf8_lossy(&request[..read]);
            let path = request
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .unwrap()
                .to_owned();
            let body = payloads.get(&path).unwrap();
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            stream.write_all(body).unwrap();
            requests.push(path);
        }
        requests
    });
    (address, server, checksums)
}

/// 写入包含三种依赖和多条件级联的服务配置。
fn write_service(root: &Path, address: &str, checksums: &BTreeMap<String, String>) -> PathBuf {
    let output = root.join("output");
    let config = format!(
        r#"version: 1
project: mac-cascade-e2e
dependencies:
  assets:
    source: "http://{address}/assets.zip"
    version: "2026.07"
    checksum: "{}"
    kind: directory
    path: bundle
  remote:
    source: "http://{address}/remote.conf"
    version: "v5"
    checksum: "{}"
    unpack: never
    kind: file
    path: remote.conf
  runner:
    source: "http://{address}/runner.tar.gz"
    version: "3.1.4"
    checksum: "{}"
    kind: binary
    path: package/bin/runner
    verify:
      args: ["--version"]
      contains: "cascade-runner 3.1.4"
tasks:
  bootstrap:
    command: "${{dependency.runner}}"
    args: ["bootstrap", "${{dependency.assets}}", "{}"]
    cwd: "."
    env:
      REMOTE_CONFIG: "${{dependency.remote}}"
  worker:
    command: "${{dependency.runner}}"
    args: ["worker", "{}"]
    shutdown_timeout_ms: 1000
    depends_on:
      bootstrap:
        condition: completed_successfully
  started-gate:
    command: "${{dependency.runner}}"
    args: ["started-gate", "{}"]
    depends_on:
      worker:
        condition: started
  healthy-gate:
    command: "${{dependency.runner}}"
    args: ["healthy-gate", "{}"]
    depends_on:
      worker:
        condition: healthy
  retry-once:
    command: "${{dependency.runner}}"
    args: ["retry", "{}"]
    restart: on-failure
    restart_delay_ms: 50
    depends_on:
      bootstrap:
        condition: completed_successfully
  finalize:
    command: "${{dependency.runner}}"
    args: ["finalize", "{}"]
    depends_on:
      started-gate:
        condition: completed_successfully
      healthy-gate:
        condition: completed_successfully
      retry-once:
        condition: completed_successfully
"#,
        checksums["assets"],
        checksums["remote"],
        checksums["runner"],
        output.display(),
        output.display(),
        output.display(),
        output.display(),
        output.display(),
        output.display(),
    );
    let path = root.join("procora.yaml");
    fs::write(&path, config).unwrap();
    path
}

/// 执行隔离中心环境中的 Procora CLI 命令。
fn procora(binary: &str, home: &Path, args: &[&str]) -> Output {
    Command::new(binary)
        .args(args)
        .env("PROCORA_HOME", home)
        .output()
        .unwrap()
}

/// 断言命令成功，并在失败时展示完整输出。
fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// 等待事件日志中出现指定次数的最终级联完成标记。
fn wait_for_final(events: &Path, expected: usize) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let count = fs::read_to_string(events)
            .unwrap_or_default()
            .lines()
            .filter(|line| *line == "final-complete")
            .count();
        if count >= expected {
            return;
        }
        assert!(Instant::now() < deadline, "任务级联未在期限内完成");
        thread::sleep(Duration::from_millis(50));
    }
}

/// 核对首次下载、单项修复和三个版本清单。
fn assert_dependency_downloads(requests: &[String], service: &Path) {
    assert_eq!(requests.len(), 4);
    for (path, expected) in [
        ("/assets.zip", 1),
        ("/runner.tar.gz", 1),
        ("/remote.conf", 2),
    ] {
        assert_eq!(
            requests.iter().filter(|request| *request == path).count(),
            expected
        );
    }
    for manifest in [
        ".procora/dependencies/assets/2026.07/manifest.json",
        ".procora/dependencies/remote/v5/manifest.json",
        ".procora/dependencies/runner/3.1.4/manifest.json",
    ] {
        assert!(service.join(manifest).is_file());
    }
}

/// 核对首轮任务级联顺序、重试日志和中心可观察状态。
fn assert_first_cascade(binary: &str, home: &Path, service: &Path) -> PathBuf {
    let opened = procora(binary, home, &["add", service.to_str().unwrap()]);
    assert_success(&opened);
    let events = service.join("output/events.log");
    wait_for_final(&events, 1);
    let event_text = fs::read_to_string(&events).unwrap();
    let final_position = event_text.find("final-complete").unwrap();
    for prerequisite in [
        "worker-started",
        "started-gate-complete",
        "healthy-gate-complete",
    ] {
        assert!(event_text.find(prerequisite).unwrap() < final_position);
    }
    assert_eq!(
        fs::read_to_string(service.join("output/cwd.txt"))
            .unwrap()
            .trim(),
        service.canonicalize().unwrap().to_string_lossy()
    );
    let retry_log = fs::read_to_string(service.join(".procora/logs/tasks/retry-once.log")).unwrap();
    assert!(retry_log.contains("retry-first-failure"));
    assert!(retry_log.contains("retry-recovered"));
    for task in ["bootstrap", "retry-once"] {
        assert!(
            service
                .join(format!(".procora/logs/tasks/{task}.log"))
                .is_file()
        );
    }
    let listed = procora(binary, home, &["list"]);
    assert_success(&listed);
    assert!(String::from_utf8_lossy(&listed.stdout).contains("mac-cascade-e2e\t运行中"));
    let history = procora(binary, home, &["history", "mac-cascade-e2e"]);
    assert_success(&history);
    assert!(String::from_utf8_lossy(&history.stdout).contains("运行中"));
    let status = procora(binary, home, &["status"]);
    assert_success(&status);
    assert!(
        String::from_utf8_lossy(&status.stdout)
            .contains(&format!("版本：{}", env!("CARGO_PKG_VERSION")))
    );
    events
}

#[test]
fn mac本机完成下载修复任务级联中心恢复和删除() {
    let sandbox = TestSandbox::new();
    let service = &sandbox.service;
    let home = &sandbox.home;
    let binary = sandbox.binary;
    let (address, server, checksums) = download_server(4);
    let config = write_service(service, &address, &checksums);

    let validated = procora(binary, home, &["validate", config.to_str().unwrap()]);
    assert_success(&validated);
    let graph = procora(binary, home, &["graph", config.to_str().unwrap()]);
    assert_success(&graph);
    assert!(String::from_utf8_lossy(&graph.stdout).contains("6. finalize"));

    let synced = procora(binary, home, &["deps", config.to_str().unwrap()]);
    assert_success(&synced);
    assert_eq!(
        String::from_utf8_lossy(&synced.stdout)
            .matches("已安装")
            .count(),
        3
    );
    let checked = procora(binary, home, &["deps", config.to_str().unwrap(), "--check"]);
    assert_success(&checked);

    let remote = service.join(".procora/dependencies/remote/v5/content/remote.conf");
    fs::write(&remote, "corrupted cache\n").unwrap();
    let broken = procora(binary, home, &["deps", config.to_str().unwrap(), "--check"]);
    assert!(!broken.status.success());
    assert!(String::from_utf8_lossy(&broken.stderr).contains("已安装内容已变化"));
    let repaired = procora(binary, home, &["deps", config.to_str().unwrap()]);
    assert_success(&repaired);
    assert!(String::from_utf8_lossy(&repaired.stdout).contains("已安装 remote v5"));
    let requests = server.join().unwrap();
    assert_dependency_downloads(&requests, service);
    let events = assert_first_cascade(binary, home, service);

    let down = procora(binary, home, &["down"]);
    assert_success(&down);
    let up = procora(binary, home, &["up"]);
    assert_success(&up);
    wait_for_final(&events, 2);

    let removed = procora(binary, home, &["remove", "mac-cascade-e2e"]);
    assert_success(&removed);
    assert!(config.is_file());
    let after_remove = procora(binary, home, &["list"]);
    assert_success(&after_remove);
    assert!(!String::from_utf8_lossy(&after_remove.stdout).contains("mac-cascade-e2e"));
    assert_success(&procora(binary, home, &["down"]));
}
