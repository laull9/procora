//! 单服务页进入内嵌配置管理的状态访问。

use super::App;

/// 单服务页内嵌配置管理入口的可用与待打开状态。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum ConfigEditState {
    /// 当前会话没有可编辑的声明式本地配置。
    #[default]
    Unavailable,
    /// 快捷键可随时打开配置管理。
    Ready,
    /// 已请求在下一轮事件循环打开配置管理。
    Pending,
}

impl App {
    /// 取出一次等待打开的内嵌配置编辑请求。
    pub fn take_pending_config_edit(&mut self) -> bool {
        if self.config_edit == ConfigEditState::Pending {
            self.config_edit = ConfigEditState::Ready;
            true
        } else {
            false
        }
    }

    /// 设置当前服务是否具有可编辑且可应用的本地配置。
    pub const fn set_config_edit_allowed(&mut self, allowed: bool) {
        self.config_edit = if allowed {
            ConfigEditState::Ready
        } else {
            ConfigEditState::Unavailable
        };
    }

    /// 返回当前服务是否允许进入配置编辑管理模式。
    pub const fn config_edit_allowed(&self) -> bool {
        !matches!(self.config_edit, ConfigEditState::Unavailable)
    }
}
