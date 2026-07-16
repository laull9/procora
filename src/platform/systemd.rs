//! Linux 上与 systemd 的可选交互。

use sd_notify::NotifyState;
use zbus_systemd::systemd1::ManagerProxy;

/// systemd 管理器使用的显式 D-Bus 范围。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SystemdBus {
    /// 当前用户的 session bus，对应 Procora 用户级 Center。
    User,
    /// 系统级 system bus，仅供显式系统集成使用。
    System,
}

impl SystemdBus {
    /// 返回适合错误和审计输出的总线名称。
    pub const fn label(self) -> &'static str {
        match self {
            Self::User => "用户总线",
            Self::System => "系统总线",
        }
    }
}

/// 向 systemd 报告 Procora daemon 已经就绪。
///
/// # Errors
///
/// 当通知套接字不可用或消息发送失败时返回 I/O 错误。
pub fn notify_ready() -> std::io::Result<()> {
    sd_notify::notify(&[NotifyState::Ready])
}

/// 通过显式选择的 systemd D-Bus 返回当前已加载单元的名称。
///
/// # Errors
///
/// 当总线连接、代理创建、权限检查或 unit 查询失败时返回 D-Bus 错误。
pub async fn list_unit_names(bus: SystemdBus) -> zbus_systemd::zbus::Result<Vec<String>> {
    let connection = match bus {
        SystemdBus::User => zbus_systemd::zbus::Connection::session().await?,
        SystemdBus::System => zbus_systemd::zbus::Connection::system().await?,
    };
    let manager = ManagerProxy::new(&connection).await?;
    manager
        .list_units()
        .await
        .map(|units| units.into_iter().map(|(name, ..)| name).collect::<Vec<_>>())
}
