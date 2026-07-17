//! 多格式配置读取、校验和任务图编译。

mod dependency;
mod diff;
mod discovery;
mod error;
mod format;
mod health;
mod loader;
mod origin;
mod python;
mod raw;
mod task_defaults;

pub use dependency::{
    DependencyKind, DependencyVerifySpec, ManagedDependencies, ManagedDependencySpec, UnpackMode,
};
pub use diff::{ProjectDiff, diff_projects};
pub use discovery::{DiscoveredProject, DiscoveryError, discover_path};
pub use error::{ConfigDiagnostic, ConfigError};
pub use format::ConfigFormat;
pub use loader::{CompiledProject, load_path, load_str};
pub(crate) use loader::{ConfigLoadCapture, load_path_capture, load_path_text};
pub use origin::{TaskConfigOrigins, ValueOrigin};
pub use python::{PythonConfigRunner, is_python_config};
pub(crate) use raw::split_command_text;
pub use task_defaults::TaskDefaultsSpec;
