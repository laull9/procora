//! 包工作台的动作确认、导出选择与水平视口计算。

use std::path::PathBuf;

use crossterm::event::KeyCode;

use super::{ExportPicker, PackageWorkspaceApp, PackageWorkspaceExit, PackageWorkspaceTab};
use crate::tui::text_view;

impl PackageWorkspaceApp {
    /// 对当前选中包生成一个单路径动作。
    pub(super) fn package_action(&mut self, action: fn(PathBuf) -> PackageWorkspaceExit) {
        if self.tab != PackageWorkspaceTab::Packages {
            self.feedback = Some("切换到“包文件”后再执行该操作".to_owned());
            return;
        }
        let Some(entry) = self.selected_package() else {
            self.feedback = Some("没有可操作的包；按 o 打开，或按 b 构建".to_owned());
            return;
        };
        if entry.info.is_none() {
            self.feedback = Some("包清单不可读；仍可按 Delete Delete 或 X X 删除".to_owned());
            return;
        }
        self.exit = Some(action(entry.path.clone()));
    }

    /// 单个导出直接执行，多个导出进入内联选择。
    pub(super) fn begin_export(&mut self) {
        if self.tab != PackageWorkspaceTab::Packages {
            self.feedback = Some("切换到“包文件”后再选择导出项".to_owned());
            return;
        }
        let Some(entry) = self.selected_package() else {
            self.feedback = Some("没有可导出的包".to_owned());
            return;
        };
        let Some(info) = &entry.info else {
            self.feedback = Some("包清单不可读，无法列出导出项".to_owned());
            return;
        };
        let entries = info.manifest.exports.keys().cloned().collect::<Vec<_>>();
        match entries.as_slice() {
            [] => self.feedback = Some("该包没有命名导出项".to_owned()),
            [only] => {
                self.exit = Some(PackageWorkspaceExit::PushExport {
                    package: entry.path.clone(),
                    entry: only.clone(),
                });
            }
            _ => {
                self.export_picker = Some(ExportPicker {
                    package: entry.path.clone(),
                    entries,
                    selected: 0,
                });
            }
        }
    }

    /// 对当前已安装 Service 生成需要完整状态的管理动作。
    pub(super) fn installed_action(
        &mut self,
        action: impl FnOnce(&str) -> PackageWorkspaceExit,
        empty_message: &str,
    ) {
        if self.tab != PackageWorkspaceTab::Installed {
            self.feedback = Some("切换到“已安装”后再执行 release 管理".to_owned());
            return;
        }
        let Some(service) = self.selected_installed() else {
            self.feedback = Some(empty_message.to_owned());
            return;
        };
        if service.error.is_some() {
            self.feedback =
                Some("安装状态损坏；可按 U U 解除，或按 D D 永久删除安装数据".to_owned());
            return;
        }
        self.exit = Some(action(&service.project));
    }

    /// 用两次 Delete 或大写 X 确认永久删除当前包文件。
    pub(super) fn confirm_package_delete(&mut self) {
        if self.tab != PackageWorkspaceTab::Packages {
            self.feedback = Some("切换到“包文件”后再删除本地包".to_owned());
            return;
        }
        let Some(path) = self.selected_package().map(|entry| entry.path.clone()) else {
            self.feedback = Some("没有可删除的包文件".to_owned());
            return;
        };
        if self.package_delete_confirmation.as_deref() == Some(path.as_path()) {
            self.package_delete_confirmation = None;
            self.exit = Some(PackageWorkspaceExit::DeletePackage(path));
        } else {
            self.package_delete_confirmation = Some(path.clone());
            self.feedback = Some(format!(
                "将永久删除包文件 `{}`；再次按 Delete 或大写 X 确认",
                path.display()
            ));
        }
    }

    /// 用两次大写 D 确认永久删除安装数据。
    pub(super) fn confirm_purge(&mut self) {
        if self.tab != PackageWorkspaceTab::Installed {
            self.feedback = Some("切换到“已安装”后再永久删除安装数据".to_owned());
            return;
        }
        let Some(project) = self
            .selected_installed()
            .map(|service| service.project.clone())
        else {
            self.feedback = Some("没有可删除的已安装 Service".to_owned());
            return;
        };
        if self.purge_confirmation.as_deref() == Some(project.as_str()) {
            self.purge_confirmation = None;
            self.exit = Some(PackageWorkspaceExit::Purge(project));
        } else {
            self.purge_confirmation = Some(project.clone());
            self.feedback = Some(format!(
                "将永久删除 `{project}` 的全部 release、状态和原始包；同名普通 Service 不受影响；再次按大写 D 确认"
            ));
        }
    }

    /// 用两次大写 U 确认仅解除包托管注册并保留安装数据。
    pub(super) fn confirm_uninstall(&mut self) {
        if self.tab != PackageWorkspaceTab::Installed {
            self.feedback = Some("切换到“已安装”后再解除安装".to_owned());
            return;
        }
        let Some(project) = self
            .selected_installed()
            .map(|service| service.project.clone())
        else {
            self.feedback = Some("没有可解除的已安装 Service".to_owned());
            return;
        };
        if self.uninstall_confirmation.as_deref() == Some(project.as_str()) {
            self.uninstall_confirmation = None;
            self.exit = Some(PackageWorkspaceExit::Uninstall(project));
        } else {
            self.uninstall_confirmation = Some(project.clone());
            self.feedback = Some(format!(
                "将解除 `{project}` 的包托管注册并保留 release 和原始包；同名普通 Service 不受影响；再次按大写 U 确认"
            ));
        }
    }

    /// 处理导出项选择弹层。
    pub(super) fn handle_export_key(&mut self, key: KeyCode) -> bool {
        let Some(picker) = self.export_picker.as_mut() else {
            return false;
        };
        match key {
            KeyCode::Esc | KeyCode::Char('q') => self.export_picker = None,
            KeyCode::Down | KeyCode::Char('j') => {
                picker.selected = (picker.selected + 1) % picker.entries.len();
            }
            KeyCode::Up | KeyCode::Char('k') => {
                picker.selected = picker
                    .selected
                    .checked_sub(1)
                    .unwrap_or(picker.entries.len() - 1);
            }
            KeyCode::Enter => {
                self.exit = Some(PackageWorkspaceExit::PushExport {
                    package: picker.package.clone(),
                    entry: picker.entries[picker.selected].clone(),
                });
                self.export_picker = None;
            }
            _ => return false,
        }
        true
    }

    /// 水平移动当前高亮项及其详情文本。
    pub(super) fn scroll_horizontal(&mut self, forward: bool) {
        let maximum = self.selected_text_maximum();
        self.horizontal_scroll.scroll_manual(forward, maximum);
    }

    /// 返回当前选中项全部可能折叠文本的最大字符偏移。
    fn selected_text_maximum(&self) -> usize {
        self.common_text_length()
            .max(match self.tab {
                PackageWorkspaceTab::Packages => {
                    self.selected_package().map_or(0, package_entry_text_length)
                }
                PackageWorkspaceTab::Installed => self
                    .selected_installed()
                    .map_or(0, installed_service_text_length),
            })
            .saturating_sub(1)
    }

    /// 返回工作台全部折叠文本的最大字符偏移。
    pub(super) fn all_text_maximum(&self) -> usize {
        self.common_text_length()
            .max(
                self.packages
                    .iter()
                    .map(package_entry_text_length)
                    .max()
                    .unwrap_or(0),
            )
            .max(
                self.installed
                    .iter()
                    .map(installed_service_text_length)
                    .max()
                    .unwrap_or(0),
            )
            .saturating_sub(1)
    }

    /// 返回上下文与反馈中的最长文本。
    fn common_text_length(&self) -> usize {
        self.context_source
            .as_ref()
            .map_or(0, |path| text_view::width(&path.to_string_lossy()))
            .max(self.feedback.as_deref().map_or(0, text_view::width))
    }
}

/// 返回一个包文件相关文本的最大字符数。
fn package_entry_text_length(entry: &super::PackageWorkspaceEntry) -> usize {
    let mut maximum = text_view::width(&entry.path.to_string_lossy());
    maximum = maximum.max(entry.error.as_deref().map_or(0, text_view::width));
    if let Some(info) = &entry.info {
        maximum = maximum
            .max(text_view::width(&info.manifest.project))
            .max(text_view::width(&info.package_digest))
            .max(
                info.manifest
                    .exports
                    .keys()
                    .map(|entry| text_view::width(entry))
                    .max()
                    .unwrap_or(0),
            )
            .max(
                info.manifest
                    .binaries
                    .values()
                    .flat_map(|binary| binary.variants.keys())
                    .map(|platform| text_view::width(platform))
                    .max()
                    .unwrap_or(0),
            );
    }
    maximum
}

/// 返回一个安装项相关文本的最大字符数。
fn installed_service_text_length(service: &crate::package::InstalledService) -> usize {
    let mut maximum = text_view::width(&service.project)
        .max(text_view::width(&service.root.to_string_lossy()))
        .max(service.error.as_deref().map_or(0, text_view::width));
    for release in &service.releases {
        maximum = maximum
            .max(text_view::width(&release.id))
            .max(text_view::width(&release.sha256))
            .max(text_view::width(&release.config_path.to_string_lossy()));
    }
    for package in &service.packages {
        maximum = maximum
            .max(text_view::width(&package.path.to_string_lossy()))
            .max(
                package
                    .package_digest
                    .as_deref()
                    .map_or(0, text_view::width),
            )
            .max(package.error.as_deref().map_or(0, text_view::width));
    }
    maximum
}
