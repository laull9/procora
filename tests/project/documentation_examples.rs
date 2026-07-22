//! 文档中的综合配置必须持续通过真实加载管线。

use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use procora::config::load_path;

/// 同一进程内并行创建文档示例目录时使用的去重序号。
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// 创建当前测试独占的临时目录。
fn temporary_directory() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let directory = std::env::temp_dir().join(format!(
        "procora-documentation-example-{}-{nonce}-{sequence}",
        std::process::id()
    ));
    fs::create_dir_all(&directory).unwrap();
    directory
}

/// 从带名称标记的 Markdown 代码块中读取配置内容。
fn fenced_block<'a>(document: &'a str, marker: &str) -> &'a str {
    let start = document
        .find(marker)
        .unwrap_or_else(|| panic!("文档缺少标记 `{marker}`"));
    let fenced = &document[start + marker.len()..];
    let content_start = fenced
        .find('\n')
        .and_then(|index| fenced[index + 1..].find('\n').map(|next| index + next + 2))
        .unwrap_or_else(|| panic!("标记 `{marker}` 后缺少代码块"));
    let content = &fenced[content_start..];
    let end = content
        .find("\n```")
        .unwrap_or_else(|| panic!("标记 `{marker}` 的代码块未关闭"));
    &content[..end]
}

#[test]
// 文档中的综合配置与环境文件可以共同完成结构、语义和任务图校验。
fn comprehensive_documentation_example_is_valid() {
    let document = include_str!("../../docs/example.md");
    let directory = temporary_directory();
    let configuration = fenced_block(document, "<!-- 配置块：comprehensive.yaml -->");
    let environment = fenced_block(document, "<!-- 配置块：comprehensive.env -->");
    let path = directory.join("procora.yaml");
    fs::write(&path, configuration).unwrap();
    fs::write(directory.join("comprehensive.env"), environment).unwrap();

    let compiled = load_path(&path)
        .unwrap_or_else(|error| panic!("综合示例 `{}` 无效：{error}", path.display()));
    assert!(!compiled.spec.tasks.is_empty());
    assert!(compiled.spec.tasks.contains_key(&"api".parse().unwrap()));
    fs::remove_dir_all(directory).unwrap();
}
