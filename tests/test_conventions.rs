//! 测试代码命名约束回归测试。

use std::{fs, path::Path};

#[test]
// 防止测试函数名重新混入中文或非 snake_case 字符。
fn test_function_names_are_english_snake_case() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let mut violations = Vec::new();
    inspect_test_names(&root, &mut violations);

    assert!(
        violations.is_empty(),
        "测试函数名必须使用英文 snake_case：\n{}",
        violations.join("\n")
    );
}

// 递归检查 Rust 测试文件中紧随测试属性的函数声明。
fn inspect_test_names(directory: &Path, violations: &mut Vec<String>) {
    for entry in fs::read_dir(directory).expect("应能读取 tests 目录") {
        let path = entry.expect("测试目录项应可读取").path();
        if path.is_dir() {
            inspect_test_names(&path, violations);
            continue;
        }
        if path.extension().and_then(|value| value.to_str()) != Some("rs") {
            continue;
        }
        let content = fs::read_to_string(&path).expect("Rust 测试文件应为 UTF-8");
        let mut awaiting_function = false;
        for (index, line) in content.lines().enumerate() {
            let trimmed = line.trim();
            if trimmed.starts_with("#[test") || trimmed.starts_with("#[tokio::test") {
                awaiting_function = true;
            }
            if !awaiting_function {
                continue;
            }
            let Some(declaration) = trimmed.strip_prefix("fn ") else {
                continue;
            };
            awaiting_function = false;
            let name = declaration.split('(').next().unwrap_or_default();
            if !is_english_snake_case(name) {
                violations.push(format!("{}:{}: {name}", path.display(), index + 1));
            }
        }
    }
}

// 判断标识符是否只含小写 ASCII 字母、数字和下划线。
fn is_english_snake_case(name: &str) -> bool {
    name.starts_with(|character: char| character.is_ascii_lowercase())
        && name.chars().all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '_'
        })
}
