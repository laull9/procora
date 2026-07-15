//! 多格式配置读取、校验和任务图编译。

mod discovery;
mod error;
mod format;
mod loader;
mod raw;

pub use discovery::{DiscoveredProject, DiscoveryError, discover_path};
pub use error::{ConfigDiagnostic, ConfigError};
pub use format::ConfigFormat;
pub use loader::{CompiledProject, load_path, load_str};
