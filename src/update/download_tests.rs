//! 更新下载器的地址、进度与流式写入测试。

use super::{Downloader, format_progress, validate_mirror};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    path::Path,
    thread,
    time::Duration,
};

/// 创建只用于地址改写测试的下载器。
fn downloader(mirror: Option<&str>) -> Downloader {
    Downloader::new(ureq::Agent::new(), mirror, None).unwrap()
}

#[test]
// 镜像支持前缀和模板两种常见代理格式。
fn github_mirror_supports_prefix_and_template() {
    let original = "https://github.com/laull9/procora/releases/download/v1/a.zip";
    assert_eq!(
        downloader(Some("https://mirror.example")).resolve(original),
        format!("https://mirror.example/{original}")
    );
    assert_eq!(
        downloader(Some("https://mirror.example/fetch?target={url}")).resolve(original),
        format!("https://mirror.example/fetch?target={original}")
    );
}

#[test]
// 镜像不改写显式的非GitHub下载源。
fn github_mirror_preserves_other_origins() {
    assert_eq!(
        downloader(Some("https://mirror.example")).resolve("http://127.0.0.1/release"),
        "http://127.0.0.1/release"
    );
}

#[test]
// 镜像只接受HTTPS且不能含空白。
fn github_mirror_requires_https() {
    assert!(validate_mirror("https://mirror.example").is_ok());
    assert!(validate_mirror("http://mirror.example").is_err());
    assert!(validate_mirror("https://mirror.example/a b").is_err());
}

#[test]
// 进度同时包含百分比、字节量和平均速度。
fn download_progress_contains_percent_bytes_and_speed() {
    let line = format_progress(
        "发布归档",
        5 * 1024 * 1024,
        Some(10 * 1024 * 1024),
        Duration::from_secs(2),
    );
    assert!(line.contains("50.0%"));
    assert!(line.contains("5.0 MiB / 10.0 MiB"));
    assert!(line.contains("2.5 MiB/s"));
}

#[test]
// 下载程序路径按单个程序处理，不解释为shell片段。
fn download_command_remains_program_path() {
    let command = Path::new("custom fetch --unsafe");
    let downloader = Downloader::new(ureq::Agent::new(), None, Some(command)).unwrap();
    assert_eq!(downloader.command.as_deref(), Some(command));
}

#[test]
// 内置下载器流式写入文件、显示进度并同步计算摘要。
fn http_download_streams_file_with_digest() {
    let body = vec![0x5a_u8; 192 * 1024];
    let expected = format!("{:x}", Sha256::digest(&body));
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let address = listener.local_addr().unwrap();
    let server_body = body.clone();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0_u8; 1024];
        let _ = stream.read(&mut request).unwrap();
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            server_body.len()
        )
        .unwrap();
        stream.write_all(&server_body).unwrap();
    });
    let directory =
        crate::platform::temp_dir().join(format!("procora-download-test-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&directory).unwrap();
    let destination = directory.join("archive.bin");

    let actual = downloader(None)
        .file(
            &format!("http://{address}/archive"),
            &destination,
            1024 * 1024,
            Some(u64::try_from(body.len()).unwrap()),
            "测试归档",
            true,
        )
        .unwrap();
    server.join().unwrap();

    assert_eq!(actual, expected);
    assert_eq!(fs::read(&destination).unwrap(), body);
    fs::remove_dir_all(directory).unwrap();
}
