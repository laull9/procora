use std::{
    collections::{BTreeMap, BTreeSet},
    io::{self, Read},
    path::{Path, PathBuf},
    time::Duration,
};

#[cfg(target_os = "linux")]
use std::thread;

use crate::{
    config::loader::{CapturedConfigInput, ConfigLoadCapture, load_generated_json},
    core::{RestartPolicy, TaskSpec},
    process::{BoundedCommandError, run_bounded_command},
};

use super::{CompiledProject, ConfigError};

/// Python 配置允许占用的最长执行时间。
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);
/// Python 配置 stdout 最大字节数。
const MAX_STDOUT_BYTES: usize = 1024 * 1024;
/// Python 配置 stderr 最大字节数。
const MAX_STDERR_BYTES: usize = 256 * 1024;
/// Python 配置脚本自身最大字节数。
const MAX_SCRIPT_BYTES: u64 = 1024 * 1024;
/// Linux 上刚写入的解释器文件临时占用时的最大重试次数。
#[cfg(target_os = "linux")]
const EXECUTABLE_BUSY_RETRIES: u8 = 3;
/// Linux 上重试解释器启动的基础等待时间。
#[cfg(target_os = "linux")]
const EXECUTABLE_BUSY_RETRY_DELAY: Duration = Duration::from_millis(10);

/// 可注入解释器的受控 Python 配置运行器。
#[derive(Clone, Debug)]
pub struct PythonConfigRunner {
    interpreter: PathBuf,
    timeout: Duration,
}

/// 一次辅助进程执行及其修订所需输出。
struct PythonExecution {
    result: Result<CompiledProject, ConfigError>,
    stdout: Vec<u8>,
    inputs: Vec<CapturedConfigInput>,
    watched_paths: Vec<PathBuf>,
}

/// Python 输出编译成功时携带的修订输入。
type PythonCompileSuccess = (
    CompiledProject,
    Vec<u8>,
    Vec<CapturedConfigInput>,
    Vec<PathBuf>,
);
/// Python 输出编译失败时仍保留的修订输入。
type PythonCompileFailure = Box<(ConfigError, Vec<u8>, Vec<CapturedConfigInput>, Vec<PathBuf>)>;

impl Default for PythonConfigRunner {
    fn default() -> Self {
        Self {
            interpreter: PathBuf::from(default_interpreter()),
            timeout: DEFAULT_TIMEOUT,
        }
    }
}

impl PythonConfigRunner {
    /// 创建使用指定解释器程序且不经过 shell 的运行器。
    pub fn new(interpreter: impl Into<PathBuf>) -> Self {
        Self {
            interpreter: interpreter.into(),
            timeout: DEFAULT_TIMEOUT,
        }
    }

    /// 设置辅助进程执行上限，主要用于嵌入和故障测试。
    #[must_use]
    pub const fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// 执行用户明确选择的 `procora.py` 并编译其单个 JSON 输出。
    ///
    /// # Errors
    ///
    /// 当入口名称不受支持、解释器失败/超时、输出越界或 JSON 无效时返回错误。
    pub fn load(&self, path: &Path) -> Result<CompiledProject, ConfigError> {
        let absolute = std::fs::canonicalize(path).map_err(|source| ConfigError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        read_script(&absolute).map_err(|source| ConfigError::Read {
            path: absolute.clone(),
            source,
        })?;
        self.execute(&absolute).result
    }

    /// 执行受管辅助进程，并保留 stdout 参与候选修订。
    fn execute(&self, path: &Path) -> PythonExecution {
        let result = self.execute_inner(path);
        match result {
            Ok((compiled, stdout, inputs, watched_paths)) => PythonExecution {
                result: Ok(compiled),
                stdout,
                inputs,
                watched_paths,
            },
            Err(failure) => {
                let (error, stdout, inputs, watched_paths) = *failure;
                PythonExecution {
                    result: Err(error),
                    stdout,
                    inputs,
                    watched_paths,
                }
            }
        }
    }

    /// 完成创建、限时等待、整树清理和严格 JSON 编译。
    fn execute_inner(&self, path: &Path) -> Result<PythonCompileSuccess, PythonCompileFailure> {
        if !is_python_config(path) {
            return Err(Box::new((
                python_error(path, "入口文件名必须是 procora.py"),
                Vec::new(),
                Vec::new(),
                Vec::new(),
            )));
        }
        tracing::warn!(
            path = %path.display(),
            "即将以当前用户权限执行显式选择的可信 Python 配置；该机制不是安全沙箱"
        );
        let root = path.parent().unwrap_or_else(|| Path::new("."));
        let task = python_task(&self.interpreter, path, root);
        let output = run_python_task(&task, self.timeout).map_err(|error| {
            Box::new((
                python_error(path, error.to_string()),
                Vec::new(),
                Vec::new(),
                Vec::new(),
            ))
        })?;
        if !output.status.success() {
            let diagnostic = String::from_utf8_lossy(&output.stderr);
            return Err(Box::new((
                python_error(
                    path,
                    format!("解释器退出 {}：{}", output.status, diagnostic.trim()),
                ),
                output.stdout,
                Vec::new(),
                Vec::new(),
            )));
        }
        compile_stdout(path, root, output.stdout)
    }
}

/// 判断显式路径是否采用唯一受支持的 Python 入口名称。
pub fn is_python_config(path: &Path) -> bool {
    path.file_name().is_some_and(|name| name == "procora.py")
}

/// 使用默认解释器执行并构造 `DefinitionSource` 捕获信息。
pub(crate) fn load_capture(path: &Path) -> ConfigLoadCapture {
    let absolute = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let root = absolute
        .parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
    let script = read_script(&absolute);
    let execution = match &script {
        Ok(_) => PythonConfigRunner::default().execute(&absolute),
        Err(error) => PythonExecution {
            result: Err(ConfigError::Read {
                path: absolute.clone(),
                source: io::Error::new(error.kind(), error.to_string()),
            }),
            stdout: Vec::new(),
            inputs: Vec::new(),
            watched_paths: Vec::new(),
        },
    };
    let mut inputs = Vec::new();
    if let Ok(bytes) = script {
        inputs.push(CapturedConfigInput {
            path: absolute.clone(),
            bytes,
        });
    }
    if !execution.stdout.is_empty() {
        inputs.push(CapturedConfigInput {
            path: root.join(".procora-python-output.json"),
            bytes: execution.stdout,
        });
    }
    inputs.extend(execution.inputs);
    let mut watched_paths = BTreeSet::from([absolute]);
    watched_paths.extend(execution.watched_paths);
    ConfigLoadCapture {
        result: execution.result,
        inputs,
        watched_paths: watched_paths.into_iter().collect(),
        root,
        definition_documents: 1,
    }
}

/// 构造清空继承环境、关闭 stdin 且启用 Python 隔离模式的任务规范。
fn python_task(interpreter: &Path, script: &Path, root: &Path) -> TaskSpec {
    let env = BTreeMap::from([("PROCORA_CONFIG".to_owned(), "1".to_owned())]);
    TaskSpec {
        command: interpreter.to_string_lossy().into_owned(),
        args: vec![
            "-I".to_owned(),
            "-S".to_owned(),
            "-X".to_owned(),
            "utf8".to_owned(),
            script.to_string_lossy().into_owned(),
        ],
        cwd: Some(root.to_path_buf()),
        env,
        healthcheck: None,
        success_exit_codes: BTreeSet::from([0]),
        depends_on: BTreeMap::default(),
        restart: RestartPolicy::Never,
        restart_delay_ms: 500,
        max_restarts: 0,
        restart_reset_after_ms: 60_000,
        shutdown_timeout_ms: 100,
    }
}

/// 执行 Python 辅助进程，并在 Linux 临时占用解释器时进行有限重试。
fn run_python_task(
    task: &TaskSpec,
    timeout: Duration,
) -> Result<crate::process::BoundedCommandOutput, BoundedCommandError> {
    #[cfg(target_os = "linux")]
    for attempt in 0..EXECUTABLE_BUSY_RETRIES {
        match run_bounded_command(task, timeout, MAX_STDOUT_BYTES, MAX_STDERR_BYTES) {
            Err(BoundedCommandError::Spawn(error))
                if error.kind() == io::ErrorKind::ExecutableFileBusy =>
            {
                thread::sleep(EXECUTABLE_BUSY_RETRY_DELAY * u32::from(attempt + 1));
            }
            result => return result,
        }
    }
    run_bounded_command(task, timeout, MAX_STDOUT_BYTES, MAX_STDERR_BYTES)
}

/// 在执行前限制脚本输入大小，避免无界文件分配。
fn read_script(path: &Path) -> io::Result<Vec<u8>> {
    let file = std::fs::File::open(path)?;
    let mut bytes = Vec::with_capacity(MAX_SCRIPT_BYTES.min(64 * 1024) as usize);
    file.take(MAX_SCRIPT_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_SCRIPT_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("脚本超过 {MAX_SCRIPT_BYTES} 字节"),
        ));
    }
    Ok(bytes)
}

/// 严格验证单个 JSON 文档后进入共享配置管线。
fn compile_stdout(
    path: &Path,
    root: &Path,
    stdout: Vec<u8>,
) -> Result<PythonCompileSuccess, PythonCompileFailure> {
    let value: serde_json::Value = serde_json::from_slice(&stdout).map_err(|error| {
        Box::new((
            python_error(path, format!("stdout 不是单个 JSON 文档：{error}")),
            stdout.clone(),
            Vec::new(),
            Vec::new(),
        ))
    })?;
    let normalized = serde_json::to_string(&value).expect("JSON Value 序列化不会失败");
    let capture = load_generated_json(&normalized, root);
    match capture.result {
        Ok(compiled) => Ok((compiled, stdout, capture.inputs, capture.watched_paths)),
        Err(error) => Err(Box::new((
            python_error(path, error.to_string()),
            stdout,
            capture.inputs,
            capture.watched_paths,
        ))),
    }
}

/// 创建带入口上下文的 Python 配置错误。
fn python_error(path: &Path, message: impl Into<String>) -> ConfigError {
    ConfigError::Python {
        path: path.to_path_buf(),
        message: message.into(),
    }
}

/// 返回当前平台默认尝试的 Python 3 解释器名。
const fn default_interpreter() -> &'static str {
    if cfg!(windows) { "python" } else { "python3" }
}
