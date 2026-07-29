//! 多格式配置读取、校验和任务图编译。

mod byte_size;
mod dependency;
mod deploy_binary;
mod diff;
mod discovery;
mod duration;
mod error;
mod format;
mod health;
mod loader;
mod origin;
mod python;
mod raw;
mod task_defaults;
mod upload;

pub(crate) use byte_size::deserialize_byte_size;
pub use dependency::{
    DependencyDownloadSpec, DependencyKind, DependencySshSpec, DependencyVerifySpec,
    ManagedDependencies, ManagedDependencySpec, UnpackMode,
};
pub use deploy_binary::{
    DeployBinaries, DeployBinarySpec, DeployBinaryVariantSpec, DeployPlatform,
    SelectedDeployBinary, select_deploy_binaries,
};
pub(crate) use deploy_binary::{RawDeployBinary, apply_deploy_binary_placeholders};
pub use diff::{ProjectDiff, diff_projects};
pub use discovery::{DiscoveredProject, DiscoveryError, discover_path};
pub(crate) use duration::{
    deserialize_duration, deserialize_optional_duration, format_duration, parse_duration,
    serialize_duration, serialize_optional_duration,
};
pub use error::{ConfigDiagnostic, ConfigError};
pub use format::ConfigFormat;
pub use loader::{CompiledProject, load_path, load_str};
pub(crate) use loader::{ConfigLoadCapture, load_path_capture, load_path_text};
pub use origin::{TaskConfigOrigins, ValueOrigin};
pub use python::{PythonConfigRunner, is_python_config};
pub(crate) use raw::split_command_text;
pub use task_defaults::TaskDefaultsSpec;
pub use upload::{UploadKind, UploadTargetSpec};
