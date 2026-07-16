//! 多格式配置读取、校验和任务图编译。

mod dependency;
mod diff;
mod discovery;
mod error;
mod format;
mod health;
mod loader;
mod python;
mod raw;

pub use dependency::{
    DependencyKind, DependencyVerifySpec, ManagedDependencies, ManagedDependencySpec, UnpackMode,
};
pub use diff::{ProjectDiff, diff_projects};
pub use discovery::{DiscoveredProject, DiscoveryError, discover_path};
pub use error::{ConfigDiagnostic, ConfigError};
pub use format::ConfigFormat;
pub use loader::{CompiledProject, load_path, load_str};
pub(crate) use loader::{ConfigLoadCapture, load_path_capture, load_path_text};
pub use python::{PythonConfigRunner, is_python_config};
