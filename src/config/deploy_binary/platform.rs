//! 部署平台名称、ABI与可移植target边界。

use std::path::{Component, Path};

/// 返回当前编译目标的ABI环境。
pub(super) fn target_environment() -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        linux_runtime_environment().map(str::to_owned)
    }
    #[cfg(not(target_os = "linux"))]
    if cfg!(target_env = "msvc") {
        Some("msvc".to_owned())
    } else {
        None
    }
}

/// 从宿主系统ldd识别Linux实际可用libc，避免误用静态Procora自身ABI。
#[cfg(target_os = "linux")]
fn linux_runtime_environment() -> Option<&'static str> {
    for path in ["/usr/bin/ldd", "/bin/ldd"] {
        if !Path::new(path).is_file() {
            continue;
        }
        let Ok(output) = std::process::Command::new(path)
            .arg("--version")
            .env("LC_ALL", "C")
            .output()
        else {
            continue;
        };
        let text = format!(
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
        .to_ascii_lowercase();
        if text.contains("musl") {
            return Some("musl");
        }
        if text.contains("glibc") || text.contains("gnu libc") {
            return Some("gnu");
        }
    }
    None
}

/// 规范化操作系统名称。
pub(super) fn normalize_os(value: &str) -> Option<&'static str> {
    match value.to_ascii_lowercase().as_str() {
        "linux" => Some("linux"),
        "macos" | "darwin" | "osx" => Some("macos"),
        "windows" | "win" => Some("windows"),
        "freebsd" => Some("freebsd"),
        _ => None,
    }
}

/// 规范化CPU架构名称并避免32/64位歧义。
pub(super) fn normalize_arch(value: &str) -> Option<&'static str> {
    match value.to_ascii_lowercase().as_str() {
        "x86_64" | "amd64" | "x64" => Some("x86_64"),
        "aarch64" | "arm64" => Some("aarch64"),
        "x86" | "i686" | "i386" => Some("x86"),
        "arm" | "armv7" | "armv7l" => Some("arm"),
        _ => None,
    }
}

/// 选择器额外接受只适用于`macOS`双架构产物的`universal`。
pub(super) fn normalize_selector_arch(value: &str) -> Option<&'static str> {
    match value.to_ascii_lowercase().as_str() {
        "universal" | "universal2" => Some("universal"),
        _ => normalize_arch(value),
    }
}

/// 规范化ABI环境名称。
pub(super) fn normalize_environment(value: &str) -> Result<&'static str, String> {
    match value.to_ascii_lowercase().as_str() {
        "gnu" | "glibc" => Ok("gnu"),
        "musl" => Ok("musl"),
        "msvc" => Ok("msvc"),
        other => Err(format!(
            "不支持运行环境 `{other}`；支持 gnu/glibc、musl、msvc"
        )),
    }
}

/// target只能是不会进入内部状态目录的普通相对文件路径。
pub(super) fn safe_target(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && path.components().all(|component| match component {
            Component::Normal(value) => {
                let Some(value) = value.to_str() else {
                    return false;
                };
                value != ".procora"
                    && !value.is_empty()
                    && !value.ends_with(['.', ' '])
                    && !value.chars().any(|character| {
                        character.is_control() || r#"\/<>:"|?*"#.contains(character)
                    })
            }
            _ => false,
        })
}
