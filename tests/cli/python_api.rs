//! Python 配置 API、包构建 API 与安装命令端到端测试。

use std::{fs, path::Path, process::Command};

use crate::{cli_uploads::temporary_directory, command_support::remove_directory_when_released};

/// 返回当前平台约定的 Python 解释器。
fn interpreter() -> &'static str {
    if cfg!(windows) { "python" } else { "python3" }
}

/// 写入使用官方 Python API 的约定配置。
fn write_python_service(root: &Path) {
    fs::write(
        root.join("service_name.py"),
        "SERVICE_NAME = 'python-demo'\n",
    )
    .unwrap();
    fs::write(
        root.join("procora.py"),
        r#"from procora import Project
from service_name import SERVICE_NAME

app = Project(SERVICE_NAME, env={"GLOBAL": "yes"})
app.task("api", ["python", "-m", "http.server"], cwd=".", restart="on-failure")
"#,
    )
    .unwrap();
}

#[test]
// Python项目可按目录发现、校验并构建为机器可读包结果。
fn python_project_is_discovered_and_built_as_package() {
    if Command::new(interpreter())
        .arg("--version")
        .output()
        .is_err()
    {
        return;
    }
    let directory = temporary_directory("python-api-package");
    let service = directory.join("service");
    fs::create_dir(&service).unwrap();
    write_python_service(&service);

    let validated = Command::new(env!("CARGO_BIN_EXE_procora"))
        .args(["validate", service.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        validated.status.success(),
        "{}",
        String::from_utf8_lossy(&validated.stderr)
    );
    assert!(String::from_utf8_lossy(&validated.stdout).contains("python-demo"));

    let package = directory.join("python-demo.pcpkg");
    let built = Command::new(env!("CARGO_BIN_EXE_procora"))
        .args(["package", "build"])
        .arg(&service)
        .arg("--output")
        .arg(&package)
        .arg("--json")
        .output()
        .unwrap();
    assert!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&built.stdout).unwrap();
    assert_eq!(result["project"], "python-demo");
    assert_eq!(result["path"], package.to_string_lossy().as_ref());
    procora::package::verify(&package).unwrap();

    remove_directory_when_released(&directory);
}

#[test]
#[cfg(unix)]
// Python build函数遵循dist目录约定并返回结构化结果。
fn python_build_api_uses_dist_convention() {
    if Command::new(interpreter())
        .arg("--version")
        .output()
        .is_err()
    {
        return;
    }
    let directory = temporary_directory("python-build-function");
    let service = directory.join("service");
    fs::create_dir(&service).unwrap();
    write_python_service(&service);
    let prepare = service.join("prepare.py");
    fs::write(
        &prepare,
        "from pathlib import Path\nprint('prepare diagnostic')\nPath('generated.txt').write_text('generated')\n",
    )
    .unwrap();
    let script = directory.join("build.py");
    fs::write(
        &script,
        format!(
            "from procora import build\nresult = build(source={:?}, prepare=[{:?}], procora_bin={:?})\nprint(result.package_digest)\n",
            service.to_string_lossy(),
            format!("{} {}", interpreter(), prepare.display()),
            env!("CARGO_BIN_EXE_procora"),
        ),
    )
    .unwrap();
    let package_root = procora::python::ensure_package().unwrap();
    let output = Command::new(interpreter())
        .arg(&script)
        .current_dir(&directory)
        .env("PYTHONPATH", package_root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).starts_with("sha256:"));
    assert!(String::from_utf8_lossy(&output.stderr).contains("prepare diagnostic"));
    assert!(directory.join("dist/python-demo.pcpkg").is_file());

    remove_directory_when_released(&directory);
}

#[test]
#[cfg(unix)]
// Python安装命令在解释器用户目录写入可导入路径文件。
fn python_install_command_writes_user_path_file() {
    use std::os::unix::fs::PermissionsExt;

    let directory = temporary_directory("python-install-command");
    let site = directory.join("site packages");
    let fake = directory.join("fake-python");
    fs::write(
        &fake,
        format!("#!/bin/sh\nprintf '%s\\n' '{}'\n", site.display()),
    )
    .unwrap();
    let mut permissions = fs::metadata(&fake).unwrap().permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(&fake, permissions).unwrap();

    let installed = Command::new(env!("CARGO_BIN_EXE_procora"))
        .args(["python", "install", "--interpreter"])
        .arg(&fake)
        .output()
        .unwrap();
    assert!(
        installed.status.success(),
        "{}",
        String::from_utf8_lossy(&installed.stderr)
    );
    let path_file = site.join("procora.pth");
    let path_declaration = fs::read_to_string(&path_file).unwrap();
    assert!(path_declaration.starts_with("import sys; sys.path.insert(0, "));
    assert!(
        procora::python::ensure_package()
            .unwrap()
            .join("procora/__init__.py")
            .is_file()
    );

    let uninstalled = Command::new(env!("CARGO_BIN_EXE_procora"))
        .args(["python", "uninstall", "--interpreter"])
        .arg(&fake)
        .output()
        .unwrap();
    assert!(uninstalled.status.success());
    assert!(!path_file.exists());

    remove_directory_when_released(&directory);
}

#[test]
// 零依赖Python MCP客户端可完成握手、列举和结构化工具调用。
fn python_mcp_client_calls_structured_tool() {
    if Command::new(interpreter())
        .arg("--version")
        .output()
        .is_err()
    {
        return;
    }
    let directory = temporary_directory("python-mcp-client");
    let service = directory.join("service");
    fs::create_dir(&service).unwrap();
    write_python_service(&service);
    let script = directory.join("mcp_client.py");
    fs::write(
        &script,
        format!(
            "from procora import McpClient\nwith McpClient({:?}) as client:\n    assert any(tool['name'] == 'build_package' for tool in client.tools())\n    result = client.call('validate_project', path={:?}, allow_python=True)\n    print(result['project'])\n",
            env!("CARGO_BIN_EXE_procora"),
            service.to_string_lossy(),
        ),
    )
    .unwrap();
    let output = Command::new(interpreter())
        .arg(&script)
        .env("PYTHONPATH", procora::python::ensure_package().unwrap())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "python-demo"
    );

    remove_directory_when_released(&directory);
}
