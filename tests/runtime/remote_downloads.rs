//! 远端依赖重试、镜像、资源边界和并发安装测试。

use std::{
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{
        Arc, Barrier,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use procora::{config::load_path, source::DependencyManager};

/// 创建当前测试独占的服务目录。
fn temporary_directory(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory = std::env::temp_dir().join(format!(
        "procora-remote-{label}-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(&directory).unwrap();
    directory
}

/// 单次 HTTP 夹具响应。
struct Response {
    status: u16,
    body: &'static [u8],
    declared_length: Option<usize>,
    required_header: Option<&'static str>,
}

/// 启动会严格处理给定响应数量的回环 HTTP 服务。
fn start_server(
    responses: Vec<Response>,
) -> (
    std::net::SocketAddr,
    Arc<AtomicUsize>,
    thread::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let requests = Arc::new(AtomicUsize::new(0));
    let observed = requests.clone();
    let server = thread::spawn(move || {
        for response in responses {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 4096];
            let read = stream.read(&mut request).unwrap();
            let request = String::from_utf8_lossy(&request[..read]);
            if let Some(required) = response.required_header {
                assert!(
                    request.to_ascii_lowercase().contains(required),
                    "请求没有携带预期请求头：{request}"
                );
            }
            observed.fetch_add(1, Ordering::SeqCst);
            let reason = if response.status == 200 {
                "OK"
            } else {
                "Service Unavailable"
            };
            let length = response.declared_length.unwrap_or(response.body.len());
            write!(
                stream,
                "HTTP/1.1 {} {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                response.status, reason, length
            )
            .unwrap();
            stream.write_all(response.body).unwrap();
        }
    });
    (address, requests, server)
}

/// 写入单文件依赖配置。
fn write_config(root: &Path, source: &str, extra: &str) -> PathBuf {
    let config = root.join("procora.yaml");
    fs::write(
        &config,
        format!(
            "version: 1\nproject: demo\ndependencies:\n  payload:\n    source: \"{source}\"\n    version: v1\n    unpack: never\n    kind: file\n    path: payload.bin\n{extra}tasks: {{}}\n"
        ),
    )
    .unwrap();
    config
}

#[test]
// 瞬时503会在同一来源内重试并最终安装成功。
fn transient_http_failure_is_retried() {
    let root = temporary_directory("retry");
    let (address, requests, server) = start_server(vec![
        Response {
            status: 503,
            body: b"",
            declared_length: None,
            required_header: None,
        },
        Response {
            status: 200,
            body: b"recovered",
            declared_length: None,
            required_header: None,
        },
    ]);
    let config = write_config(
        &root,
        &format!("http://{address}/payload.bin"),
        "    download:\n      retries: 1\n      timeout: 5s\n      max_bytes: 1024\n",
    );
    let compiled = load_path(config).unwrap();

    let resolved = DependencyManager::new(&root)
        .sync(&compiled.dependencies)
        .unwrap();
    assert_eq!(fs::read(&resolved[0].path).unwrap(), b"recovered");
    assert_eq!(requests.load(Ordering::SeqCst), 2);
    server.join().unwrap();
    fs::remove_dir_all(root).unwrap();
}

#[test]
// 一行HTTP来源会自动选择文件名、缓存版本和下载策略。
fn one_line_http_source_downloads_with_defaults() {
    let root = temporary_directory("one-line-http");
    let (address, requests, server) = start_server(vec![Response {
        status: 200,
        body: b"one line http",
        declared_length: None,
        required_header: None,
    }]);
    let config = root.join("procora.yaml");
    fs::write(
        &config,
        format!(
            "version: 1\nproject: demo\ndependencies:\n  payload: http://{address}/payload.bin\ntasks: {{}}\n"
        ),
    )
    .unwrap();
    let compiled = load_path(config).unwrap();

    let resolved = DependencyManager::new(&root)
        .sync(&compiled.dependencies)
        .unwrap();
    assert_eq!(resolved[0].version, "source");
    assert_eq!(fs::read(&resolved[0].path).unwrap(), b"one line http");
    assert_eq!(requests.load(Ordering::SeqCst), 1);
    server.join().unwrap();
    fs::remove_dir_all(root).unwrap();
}

#[test]
// 主来源不可达时会无缝切换到声明顺序中的镜像。
fn unavailable_primary_falls_back_to_mirror() {
    let root = temporary_directory("mirror");
    let unavailable = TcpListener::bind("127.0.0.1:0").unwrap();
    let unavailable_address = unavailable.local_addr().unwrap();
    drop(unavailable);
    let (mirror, requests, server) = start_server(vec![Response {
        status: 200,
        body: b"mirror payload",
        declared_length: None,
        required_header: None,
    }]);
    let config = write_config(
        &root,
        &format!("http://{unavailable_address}/payload.bin"),
        &format!(
            "    mirrors: [\"http://{mirror}/payload.bin\"]\n    download:\n      retries: 0\n      timeout: 1s\n      max_bytes: 1024\n"
        ),
    );
    let compiled = load_path(config).unwrap();

    let resolved = DependencyManager::new(&root)
        .sync(&compiled.dependencies)
        .unwrap();
    assert_eq!(fs::read(&resolved[0].path).unwrap(), b"mirror payload");
    assert_eq!(requests.load(Ordering::SeqCst), 1);
    server.join().unwrap();
    fs::remove_dir_all(root).unwrap();
}

#[test]
// Content-Length超过上限时在读取正文前拒绝安装。
fn oversized_http_response_is_rejected() {
    let root = temporary_directory("limit");
    let (address, _, server) = start_server(vec![Response {
        status: 200,
        body: b"",
        declared_length: Some(4096),
        required_header: None,
    }]);
    let config = write_config(
        &root,
        &format!("http://{address}/payload.bin"),
        "    download:\n      retries: 0\n      timeout: 5s\n      max_bytes: 8\n",
    );
    let compiled = load_path(config).unwrap();

    let error = DependencyManager::new(&root)
        .sync(&compiled.dependencies)
        .unwrap_err();
    assert!(error.to_string().contains("超过配置上限 8 字节"));
    assert!(!root.join(".procora/dependencies/payload/v1").exists());
    server.join().unwrap();
    fs::remove_dir_all(root).unwrap();
}

#[test]
// 私有制品请求头会准确发送且不会写入安装清单。
fn private_http_header_is_sent_but_not_persisted() {
    let root = temporary_directory("header");
    let (address, _, server) = start_server(vec![Response {
        status: 200,
        body: b"private payload",
        declared_length: None,
        required_header: Some("authorization: bearer test-token\r\n"),
    }]);
    let config = write_config(
        &root,
        &format!("http://{address}/payload.bin"),
        "    download:\n      retries: 0\n      timeout: 5s\n      max_bytes: 1024\n      headers:\n        Authorization: Bearer test-token\n",
    );
    let compiled = load_path(config).unwrap();

    DependencyManager::new(&root)
        .sync(&compiled.dependencies)
        .unwrap();
    let manifest =
        fs::read_to_string(root.join(".procora/dependencies/payload/v1/manifest.json")).unwrap();
    assert!(!manifest.contains("test-token"));
    server.join().unwrap();
    fs::remove_dir_all(root).unwrap();
}

#[test]
// 未提供的环境凭据会在联网前给出明确错误且不泄露其他请求头。
fn missing_header_environment_is_actionable() {
    let root = temporary_directory("missing-env");
    let config = write_config(
        &root,
        "http://127.0.0.1:1/payload.bin",
        "    download:\n      retries: 0\n      timeout: 1s\n      max_bytes: 1024\n      headers:\n        Authorization: Bearer ${env.PROCORA_TEST_MISSING_REMOTE_TOKEN_9F8E7D}\n",
    );
    let compiled = load_path(config).unwrap();

    let error = DependencyManager::new(&root)
        .sync(&compiled.dependencies)
        .unwrap_err()
        .to_string();
    assert!(error.contains("PROCORA_TEST_MISSING_REMOTE_TOKEN_9F8E7D"));
    assert!(error.contains("未设置"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
// 两个并发同步者只下载一次并共享完整缓存。
fn concurrent_sync_downloads_once() {
    let root = temporary_directory("concurrent");
    let (address, requests, server) = start_server(vec![Response {
        status: 200,
        body: b"one transfer",
        declared_length: None,
        required_header: None,
    }]);
    let config = write_config(
        &root,
        &format!("http://{address}/payload.bin"),
        "    download:\n      retries: 0\n      timeout: 5s\n      max_bytes: 1024\n",
    );
    let dependencies = Arc::new(load_path(config).unwrap().dependencies);
    let barrier = Arc::new(Barrier::new(2));
    let mut workers = Vec::new();
    for _ in 0..2 {
        let root = root.clone();
        let dependencies = dependencies.clone();
        let barrier = barrier.clone();
        workers.push(thread::spawn(move || {
            barrier.wait();
            DependencyManager::new(root).sync(&dependencies).unwrap()
        }));
    }
    let first = workers.remove(0).join().unwrap();
    let second = workers.remove(0).join().unwrap();
    assert_ne!(first[0].installed, second[0].installed);
    assert_eq!(requests.load(Ordering::SeqCst), 1);
    server.join().unwrap();
    fs::remove_dir_all(root).unwrap();
}

#[test]
// 替换下载失败时保留原安装，不留下半成品正式目录。
fn failed_replacement_preserves_previous_install() {
    let root = temporary_directory("rollback");
    fs::write(root.join("payload.bin"), b"stable payload").unwrap();
    let config = write_config(&root, "payload.bin", "");
    let first = load_path(&config).unwrap();
    let manager = DependencyManager::new(&root);
    let installed = manager.sync(&first.dependencies).unwrap();
    fs::write(
        &config,
        fs::read_to_string(&config).unwrap().replacen(
            "source: \"payload.bin\"",
            "source: \"missing.bin\"",
            1,
        ),
    )
    .unwrap();
    let replacement = load_path(config).unwrap();

    assert!(manager.sync(&replacement.dependencies).is_err());
    assert_eq!(fs::read(&installed[0].path).unwrap(), b"stable payload");
    assert!(installed[0].path.is_file());
    fs::remove_dir_all(root).unwrap();
}

/// 确保测试启动的 sshd 在退出路径上被回收。
#[cfg(unix)]
struct SshdGuard(std::process::Child);

#[cfg(unix)]
impl Drop for SshdGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
#[cfg(unix)]
// 临时OpenSSH服务会完成真实密钥认证和SCP文件下载。
fn ssh_source_downloads_from_real_openssh_server() {
    use std::{os::unix::fs::PermissionsExt, process::Command};

    if !Path::new("/usr/sbin/sshd").is_file()
        || Command::new("ssh-keygen").arg("-V").output().is_err()
    {
        eprintln!("当前系统没有 OpenSSH 服务端，跳过实盘 SSH 测试");
        return;
    }
    let root = temporary_directory("ssh");
    let ssh = root.join("ssh");
    fs::create_dir_all(&ssh).unwrap();
    fs::set_permissions(&ssh, fs::Permissions::from_mode(0o700)).unwrap();
    let identity = ssh.join("identity");
    let host_key = ssh.join("host_key");
    for key in [&identity, &host_key] {
        let status = Command::new("ssh-keygen")
            .args(["-q", "-t", "ed25519", "-N", "", "-f"])
            .arg(key)
            .status()
            .unwrap();
        assert!(status.success());
    }
    let authorized = ssh.join("authorized_keys");
    fs::copy(identity.with_extension("pub"), &authorized).unwrap();
    fs::set_permissions(&authorized, fs::Permissions::from_mode(0o600)).unwrap();
    let reservation = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = reservation.local_addr().unwrap().port();
    drop(reservation);
    let user = String::from_utf8(Command::new("id").arg("-un").output().unwrap().stdout).unwrap();
    let user = user.trim();
    let config_path = ssh.join("sshd_config");
    fs::write(
        &config_path,
        format!(
            "Port {port}\nListenAddress 127.0.0.1\nHostKey {}\nPidFile {}\nAuthorizedKeysFile {}\nPasswordAuthentication no\nKbdInteractiveAuthentication no\nPubkeyAuthentication yes\nStrictModes no\nUsePAM no\nPermitRootLogin yes\nAllowUsers {user}\nLogLevel ERROR\nSubsystem sftp internal-sftp\n",
            host_key.display(),
            ssh.join("sshd.pid").display(),
            authorized.display()
        ),
    )
    .unwrap();
    let validation = Command::new("/usr/sbin/sshd")
        .args(["-t", "-f"])
        .arg(&config_path)
        .output()
        .unwrap();
    assert!(
        validation.status.success(),
        "sshd 配置无效：{}",
        String::from_utf8_lossy(&validation.stderr)
    );
    let child = Command::new("/usr/sbin/sshd")
        .args(["-D", "-e", "-f"])
        .arg(&config_path)
        .spawn()
        .unwrap();
    let mut server = SshdGuard(child);
    let deadline = Instant::now() + Duration::from_secs(5);
    while TcpStream::connect(("127.0.0.1", port)).is_err() {
        assert!(server.0.try_wait().unwrap().is_none(), "sshd 提前退出");
        assert!(Instant::now() < deadline, "sshd 没有按时监听");
        thread::sleep(Duration::from_millis(25));
    }
    let host_public = fs::read_to_string(host_key.with_extension("pub")).unwrap();
    let host_identity = host_public
        .split_whitespace()
        .take(2)
        .collect::<Vec<_>>()
        .join(" ");
    let known_hosts = ssh.join("known_hosts");
    fs::write(
        &known_hosts,
        format!("[127.0.0.1]:{port} {host_identity}\n"),
    )
    .unwrap();
    let remote = root.join("remote");
    fs::create_dir_all(&remote).unwrap();
    fs::write(remote.join("payload.bin"), b"real ssh payload").unwrap();
    let source = format!(
        "ssh://{user}@127.0.0.1:{port}{}/payload.bin",
        remote.display()
    );
    let config = write_config(
        &root,
        &source,
        &format!(
            "    download:\n      retries: 0\n      timeout: 10s\n      max_bytes: 1024\n    ssh:\n      identity_file: \"{}\"\n      known_hosts_file: \"{}\"\n",
            identity.display(),
            known_hosts.display()
        ),
    );
    let compiled = load_path(config).unwrap();

    let resolved = DependencyManager::new(&root)
        .sync(&compiled.dependencies)
        .unwrap();
    assert_eq!(fs::read(&resolved[0].path).unwrap(), b"real ssh payload");
    drop(server);
    fs::remove_dir_all(root).unwrap();
}
