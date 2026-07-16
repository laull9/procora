//! 下载、缓存、解包、占位符和版本验证闭环测试。

use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    path::PathBuf,
    thread,
    time::{SystemTime, UNIX_EPOCH},
};

use procora::config::load_path;
use procora::source::DependencyManager;

/// 创建当前测试独占的服务目录。
fn temporary_directory(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory = std::env::temp_dir().join(format!(
        "procora-source-{label}-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(&directory).unwrap();
    directory
}

/// 写入引用本地脚本的基础配置。
#[cfg(unix)]
fn write_config(root: &std::path::Path) -> PathBuf {
    let config = root.join("procora.yaml");
    fs::write(
        &config,
        r#"version: 1
project: demo
dependencies:
  tool:
    source: tool.sh
    version: 1.2.3
    unpack: never
    kind: binary
    path: tool.sh
    verify:
      contains: 1.2.3
tasks:
  run:
    command: "${dependency.tool}"
"#,
    )
    .unwrap();
    config
}

#[test]
#[cfg(unix)]
fn 本地二进制会安装验证缓存并替换任务占位符() {
    let root = temporary_directory("binary");
    fs::write(root.join("tool.sh"), "#!/bin/sh\necho tool 1.2.3\n").unwrap();
    let config = write_config(&root);
    let manager = DependencyManager::new(&root);

    let mut first = load_path(&config).unwrap();
    let resolved = manager.prepare(&mut first).unwrap();
    assert!(resolved[0].installed);
    assert_eq!(
        first.spec.tasks.values().next().unwrap().command,
        resolved[0].path.to_string_lossy()
    );
    assert!(
        root.join(".procora/dependencies/tool/1.2.3/manifest.json")
            .is_file()
    );

    let mut second = load_path(&config).unwrap();
    let cached = manager.prepare(&mut second).unwrap();
    assert!(!cached[0].installed);
    fs::remove_dir_all(root).unwrap();
}

#[test]
#[cfg(unix)]
fn 损坏的版本输出会触发重新安装() {
    let root = temporary_directory("repair");
    fs::write(root.join("tool.sh"), "#!/bin/sh\necho tool 1.2.3\n").unwrap();
    let config = write_config(&root);
    let manager = DependencyManager::new(&root);
    let mut compiled = load_path(&config).unwrap();
    let installed = manager.prepare(&mut compiled).unwrap();
    fs::write(&installed[0].path, "#!/bin/sh\necho broken\n").unwrap();

    let repaired = manager.sync(&compiled.dependencies).unwrap();
    assert!(repaired[0].installed);
    let checked = manager.check(&compiled.dependencies).unwrap();
    assert_eq!(checked[0].version, "1.2.3");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn sha256不匹配会拒绝安装() {
    let root = temporary_directory("checksum");
    fs::write(root.join("tool.sh"), "content").unwrap();
    let config = root.join("procora.yaml");
    fs::write(
        &config,
        r#"version: 1
project: demo
dependencies:
  tool:
    source: tool.sh
    version: "1"
    checksum: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
    unpack: never
tasks: {}
"#,
    )
    .unwrap();
    let compiled = load_path(config).unwrap();

    let error = DependencyManager::new(&root)
        .sync(&compiled.dependencies)
        .unwrap_err();
    assert!(error.to_string().contains("SHA-256 不匹配"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn http来源会下载并安装管理文件() {
    let root = temporary_directory("http");
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0_u8; 1024];
        let _ = stream.read(&mut request).unwrap();
        let body = b"remote payload";
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .unwrap();
        stream.write_all(body).unwrap();
    });
    let config = root.join("procora.yaml");
    fs::write(
        &config,
        format!(
            "version: 1\nproject: demo\ndependencies:\n  payload:\n    source: http://{address}/payload.bin\n    version: v1\n    unpack: never\n    kind: file\n    path: payload.bin\ntasks: {{}}\n"
        ),
    )
    .unwrap();
    let compiled = load_path(config).unwrap();

    let resolved = DependencyManager::new(&root)
        .sync(&compiled.dependencies)
        .unwrap();
    assert_eq!(fs::read(&resolved[0].path).unwrap(), b"remote payload");
    server.join().unwrap();
    fs::remove_dir_all(root).unwrap();
}

#[test]
#[cfg(unix)]
fn targz会自动解包并选择声明的二进制() {
    let root = temporary_directory("archive");
    let archive = fs::File::create(root.join("tool.tar.gz")).unwrap();
    let encoder = flate2::write::GzEncoder::new(archive, flate2::Compression::default());
    let mut builder = tar::Builder::new(encoder);
    let body = b"#!/bin/sh\necho archived 2.0\n";
    let mut header = tar::Header::new_gnu();
    header.set_size(body.len() as u64);
    header.set_mode(0o755);
    header.set_cksum();
    builder
        .append_data(&mut header, "package/bin/tool", &body[..])
        .unwrap();
    builder.into_inner().unwrap().finish().unwrap();
    let config = root.join("procora.yaml");
    fs::write(
        &config,
        r#"version: 1
project: demo
dependencies:
  tool:
    source: tool.tar.gz
    version: "2.0"
    kind: binary
    path: package/bin/tool
    verify:
      contains: "2.0"
tasks: {}
"#,
    )
    .unwrap();
    let compiled = load_path(config).unwrap();

    let resolved = DependencyManager::new(&root)
        .sync(&compiled.dependencies)
        .unwrap();
    assert!(resolved[0].path.ends_with("package/bin/tool"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn zip会自动解包并验证归档内目录内容() {
    let root = temporary_directory("zip");
    let archive = fs::File::create(root.join("assets.zip")).unwrap();
    let mut writer = zip::ZipWriter::new(archive);
    writer
        .start_file("bundle/data.txt", zip::write::SimpleFileOptions::default())
        .unwrap();
    writer.write_all(b"zip payload").unwrap();
    writer.finish().unwrap();
    let config = root.join("procora.yaml");
    fs::write(
        &config,
        r#"version: 1
project: demo
dependencies:
  assets:
    source: assets.zip
    version: "1"
    kind: directory
    path: bundle
tasks: {}
"#,
    )
    .unwrap();
    let compiled = load_path(config).unwrap();

    let resolved = DependencyManager::new(&root)
        .sync(&compiled.dependencies)
        .unwrap();
    assert_eq!(
        fs::read_to_string(resolved[0].path.join("data.txt")).unwrap(),
        "zip payload"
    );
    fs::write(resolved[0].path.join("data.txt"), "changed").unwrap();
    let error = DependencyManager::new(&root)
        .check(&compiled.dependencies)
        .unwrap_err();
    assert!(error.to_string().contains("已安装内容已变化"));
    fs::remove_dir_all(root).unwrap();
}
