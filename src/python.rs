//! 与 Procora 二进制同版本分发的 Python API 安装支持。

use std::{fs, path::PathBuf, process::Command};

use anyhow::{Context, bail};
use directories::ProjectDirs;
use fs2::FileExt as _;
use serde::Serialize;

/// 嵌入二进制的 Python 包文件。
const PACKAGE_FILES: &[(&str, &str)] = &[
    ("__init__.py", include_str!("../python/procora/__init__.py")),
    ("__main__.py", include_str!("../python/procora/__main__.py")),
    ("model.py", include_str!("../python/procora/model.py")),
    ("package.py", include_str!("../python/procora/package.py")),
    ("mcp.py", include_str!("../python/procora/mcp.py")),
];

/// Python API 安装后的稳定路径摘要。
#[derive(Clone, Debug, Serialize)]
pub struct PythonInstallReport {
    /// 用于查询用户 site-packages 的解释器。
    pub interpreter: PathBuf,
    /// 包含 `procora` 模块的 Procora 数据目录。
    pub package_root: PathBuf,
    /// 当前解释器的用户 site-packages。
    pub site_packages: PathBuf,
    /// 让普通 Python 脚本发现 Procora API 的 `.pth` 文件。
    pub path_file: PathBuf,
}

/// 返回当前平台默认 Python 3 解释器名。
pub const fn default_interpreter() -> &'static str {
    if cfg!(windows) { "python" } else { "python3" }
}

/// 确保嵌入式 Python 包已在当前用户 Procora 数据目录物化。
///
/// # Errors
///
/// 当数据目录不可用或包文件无法完整写入时返回错误。
pub fn ensure_package() -> anyhow::Result<PathBuf> {
    let root = python_root()?;
    let package = root.join("procora");
    fs::create_dir_all(&package).context("无法创建 Procora Python 包目录")?;
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(root.join(".install.lock"))?;
    lock.lock_exclusive()
        .context("无法锁定 Procora Python 包更新")?;
    for (name, content) in PACKAGE_FILES {
        write_package_file(&package, name, content)?;
    }
    let version = format!("VERSION = {:?}\n", env!("CARGO_PKG_VERSION"));
    write_package_file(&package, "_version.py", &version)?;
    Ok(root)
}

/// 在内容变化时原子替换一个嵌入式 Python 文件。
fn write_package_file(package: &std::path::Path, name: &str, content: &str) -> anyhow::Result<()> {
    let path = package.join(name);
    if fs::read_to_string(&path).is_ok_and(|existing| existing == content) {
        return Ok(());
    }
    let temporary = package.join(format!(".{name}.tmp-{}", uuid::Uuid::new_v4()));
    fs::write(&temporary, content)
        .with_context(|| format!("无法写入 Python 包临时文件 `{}`", temporary.display()))?;
    replace_file(&temporary, &path)
        .with_context(|| format!("无法更新 Python 包文件 `{}`", path.display()))
}

/// 为指定解释器安装嵌入式包并写入用户 `.pth`。
///
/// # Errors
///
/// 当解释器不可用、用户 site-packages 无效或文件无法写入时返回错误。
pub fn install(interpreter: Option<PathBuf>) -> anyhow::Result<PythonInstallReport> {
    let interpreter = interpreter.unwrap_or_else(|| PathBuf::from(default_interpreter()));
    let package_root = ensure_package()?;
    let site_packages = query_user_site(&interpreter)?;
    fs::create_dir_all(&site_packages).with_context(|| {
        format!(
            "无法创建 Python 用户 site-packages `{}`",
            site_packages.display()
        )
    })?;
    let path_file = site_packages.join("procora.pth");
    let content = path_file_content(&package_root)?;
    fs::write(&path_file, content)
        .with_context(|| format!("无法写入 Python 路径文件 `{}`", path_file.display()))?;
    Ok(PythonInstallReport {
        interpreter,
        package_root,
        site_packages,
        path_file,
    })
}

/// 移除指定解释器的 Procora `.pth`，保留可由升级继续复用的包文件。
///
/// # Errors
///
/// 当解释器不可用或已有路径文件无法移除时返回错误。
pub fn uninstall(interpreter: Option<PathBuf>) -> anyhow::Result<Option<PathBuf>> {
    let interpreter = interpreter.unwrap_or_else(|| PathBuf::from(default_interpreter()));
    let path = query_user_site(&interpreter)?.join("procora.pth");
    if !path.exists() {
        return Ok(None);
    }
    let expected = path_file_content(&python_root()?)?;
    let existing = fs::read_to_string(&path)
        .with_context(|| format!("无法读取 Python 路径文件 `{}`", path.display()))?;
    if existing != expected {
        bail!("Python 路径文件 `{}` 已被修改，拒绝删除", path.display());
    }
    fs::remove_file(&path)
        .with_context(|| format!("无法删除 Python 路径文件 `{}`", path.display()))?;
    Ok(Some(path))
}

/// 返回包含 `procora` 包的稳定用户数据目录。
fn python_root() -> anyhow::Result<PathBuf> {
    let data = if let Some(home) = std::env::var_os("PROCORA_HOME") {
        PathBuf::from(home)
    } else {
        ProjectDirs::from("dev", "procora", "Procora")
            .context("当前平台没有可用的 Procora 用户数据目录")?
            .data_local_dir()
            .to_path_buf()
    };
    Ok(crate::platform::simplify_path(&data).join("python"))
}

/// 生成只含 ASCII 的 `.pth` 注入行，兼容 Windows 非 UTF-8 用户路径。
fn path_file_content(package_root: &std::path::Path) -> anyhow::Result<String> {
    let path = package_root
        .to_str()
        .context("Procora Python 包路径不是有效 Unicode")?;
    let mut encoded = String::from("\"");
    for character in path.chars() {
        match character {
            '\\' => encoded.push_str("\\\\"),
            '"' => encoded.push_str("\\\""),
            '\n' => encoded.push_str("\\n"),
            '\r' => encoded.push_str("\\r"),
            '\t' => encoded.push_str("\\t"),
            value if value.is_ascii_control() => {
                use std::fmt::Write as _;
                write!(encoded, "\\x{:02x}", u32::from(value))?;
            }
            value if value.is_ascii() => encoded.push(value),
            value if u32::from(value) <= 0xffff => {
                use std::fmt::Write as _;
                write!(encoded, "\\u{:04x}", u32::from(value))?;
            }
            value => {
                use std::fmt::Write as _;
                write!(encoded, "\\U{:08x}", u32::from(value))?;
            }
        }
    }
    encoded.push('"');
    Ok(format!("import sys; sys.path.insert(0, {encoded})\n"))
}

/// 通过解释器标准库查询用户 site-packages。
fn query_user_site(interpreter: &std::path::Path) -> anyhow::Result<PathBuf> {
    let mut command = Command::new(interpreter);
    command.args([
        "-X",
        "utf8",
        "-c",
        "import site,sys; sys.version_info >= (3, 9) or sys.exit('Procora Python API requires Python 3.9+'); print(site.getsitepackages()[0] if sys.prefix != sys.base_prefix else site.getusersitepackages())",
    ]);
    crate::process::configure_background_command(&mut command);
    let output = command
        .output()
        .with_context(|| format!("无法启动 Python 解释器 `{}`", interpreter.display()))?;
    if !output.status.success() {
        bail!(
            "Python 解释器 `{}` 无法查询用户 site-packages：{}",
            interpreter.display(),
            crate::platform::decode_external_output(&output.stderr).trim()
        );
    }
    let value = crate::platform::decode_external_output(&output.stdout)
        .trim()
        .to_owned();
    if value.is_empty() || value.lines().count() != 1 {
        bail!("Python 解释器返回了无效的用户 site-packages 路径");
    }
    let path = PathBuf::from(value);
    if !path.is_absolute() {
        bail!("Python 用户 site-packages 必须是绝对路径");
    }
    Ok(crate::platform::simplify_path(&path))
}

/// 以跨平台方式替换单个嵌入式包文件。
fn replace_file(source: &std::path::Path, destination: &std::path::Path) -> std::io::Result<()> {
    #[cfg(windows)]
    if destination.exists() {
        fs::remove_file(destination)?;
    }
    fs::rename(source, destination)
}

#[cfg(test)]
mod tests {
    use super::path_file_content;

    #[test]
    // Python路径文件保持纯ASCII并安全表达非ASCII和反斜杠。
    fn path_file_uses_ascii_python_literal() {
        let content = path_file_content(std::path::Path::new("C:\\用户\\Procora")).unwrap();

        assert!(content.is_ascii());
        assert!(content.contains("C:\\\\"));
        assert!(content.contains("\\u7528\\u6237"));
    }
}
