use std::{
    io::{Read, Write},
    net::TcpListener,
    process::Command,
    thread,
};

/// 启动只响应一次最新 `Release` 查询的本地 `GitHub API` 替身。
fn release_api(tag: &str) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let address = listener.local_addr().unwrap();
    let body = serde_json::json!({
        "tag_name": tag,
        "assets": [],
    })
    .to_string();
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0_u8; 4096];
        let count = stream.read(&mut request).unwrap();
        let request = String::from_utf8_lossy(&request[..count]);
        assert!(request.starts_with("GET /release HTTP/1.1\r\n"));
        assert!(
            request
                .to_ascii_lowercase()
                .contains("user-agent: procora/0.6.0\r\n")
        );
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        )
        .unwrap();
    });
    (format!("http://{address}/release"), handle)
}

#[test]
// update的check模式只报告新版本，不要求下载资产或替换测试二进制。
fn update_check_reports_new_release_without_installing() {
    let (url, server) = release_api("v0.7.0");
    let output = Command::new(env!("CARGO_BIN_EXE_procora"))
        .args(["update", "--check"])
        .env("PROCORA_UPDATE_API_URL", url)
        .output()
        .unwrap();
    server.join().unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("v0.6.0 → v0.7.0"));
}

#[test]
// 已处于最新版本时update直接成功且不访问任何发布资产。
fn update_check_reports_current_version() {
    let (url, server) = release_api("v0.6.0");
    let output = Command::new(env!("CARGO_BIN_EXE_procora"))
        .args(["update", "--check"])
        .env("PROCORA_UPDATE_API_URL", url)
        .output()
        .unwrap();
    server.join().unwrap();

    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("已是最新版本：v0.6.0"));
}
