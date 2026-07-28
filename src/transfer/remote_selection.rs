use std::{
    io::{self, IsTerminal, Write},
    path::Path,
};

use anyhow::{Context, bail};

use crate::config::UploadKind;

use super::{protocol::TransferTarget, remote::human_bytes};

/// 远端候选列表或手动选择器输入。
#[derive(Clone)]
enum TargetChoice {
    Listed(String),
    Manual,
}

/// 在交互终端列出远端候选目标并读取选择。
pub(super) fn choose_target(
    targets: &[TransferTarget],
    batch: bool,
    preferred_target: Option<&str>,
) -> anyhow::Result<String> {
    if batch || !io::stdin().is_terminal() || !io::stderr().is_terminal() {
        let selectors = targets
            .iter()
            .map(|target| target.selector.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        bail!("远端有多个兼容上传目标：{selectors}；请用 `--target <选择器>` 明确指定");
    }
    let mut targets = targets.iter().collect::<Vec<_>>();
    if let Some(preferred) = preferred_target
        && let Some(index) = targets
            .iter()
            .position(|target| target.selector == preferred)
    {
        targets.swap(0, index);
    }
    let mut items = targets
        .into_iter()
        .map(|target| {
            crate::tui::SelectionItem::new(
                &target.selector,
                format!(
                    "{} · {} · 上限 {} · {}",
                    target_path_label(&target.path),
                    kind_label(target.kind),
                    human_bytes(target.max_bytes),
                    restart_label(target.restart)
                ),
                TargetChoice::Listed(target.selector.clone()),
            )
        })
        .collect::<Vec<_>>();
    items.push(crate::tui::SelectionItem::new(
        "手动输入选择器",
        "输入 service::name 或 service::task::name",
        TargetChoice::Manual,
    ));
    let choice = crate::tui::select_inline(
        "选择远端上传项目",
        "已从远端 Procora 拉取与本机来源兼容的活动上传项。",
        items,
    )?
    .context("已取消上传目标选择")?;
    match choice {
        TargetChoice::Listed(selector) => Ok(selector),
        TargetChoice::Manual => {
            eprint!("远端上传目标（service::name 或 service::task::name）：");
            io::stderr().flush()?;
            let mut selector = String::new();
            if io::stdin().read_line(&mut selector)? == 0 {
                bail!("输入已结束");
            }
            let selector = selector.trim();
            if selector.is_empty() {
                bail!("远端上传目标不能为空");
            }
            Ok(selector.to_owned())
        }
    }
}

/// 兼容旧协议未返回目标路径的选择项。
fn target_path_label(path: &Path) -> String {
    if path.as_os_str().is_empty() {
        "路径未提供（旧协议）".to_owned()
    } else {
        path.display().to_string()
    }
}

/// 返回面向 CLI 的上传目标类型名称。
const fn kind_label(kind: UploadKind) -> &'static str {
    match kind {
        UploadKind::File => "文件",
        UploadKind::Directory => "目录",
    }
}

/// 返回上传目标的远端默认重启行为。
const fn restart_label(restart: bool) -> &'static str {
    if restart {
        "提交后自动重启"
    } else {
        "仅覆盖"
    }
}
