use crossterm::event::KeyEvent;

use super::{
    config_form_state::{FormEvent, FormState},
    selection::{SelectionEvent, SelectionItem, SelectionState},
};

/// 编辑弹窗退出确认提供的动作。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DialogExitChoice {
    Save,
    Discard,
    Cancel,
}

impl FormState {
    /// 返回字段弹窗上方的未保存退出选择栏。
    pub(super) fn dialog_exit_prompt(&self) -> Option<&SelectionState<DialogExitChoice>> {
        self.dialog_exit_prompt.as_ref()
    }

    /// 处理字段弹窗的保存、放弃和取消退出选择。
    pub(super) fn handle_dialog_exit(&mut self, key: KeyEvent) -> FormEvent {
        let prompt = self
            .dialog_exit_prompt
            .as_mut()
            .expect("弹窗退出选择状态存在");
        match prompt.handle_key(key) {
            SelectionEvent::Pending => FormEvent::None,
            SelectionEvent::Cancelled | SelectionEvent::Selected(DialogExitChoice::Cancel) => {
                self.dialog_exit_prompt = None;
                FormEvent::Message("继续编辑".to_owned())
            }
            SelectionEvent::Selected(DialogExitChoice::Discard) => {
                self.dialog_exit_prompt = None;
                self.dialog = None;
                FormEvent::Message("已放弃本轮编辑".to_owned())
            }
            SelectionEvent::Selected(DialogExitChoice::Save) => {
                self.dialog_exit_prompt = None;
                self.save_dialog()
            }
        }
    }
}

/// 创建字段弹窗通用的未保存退出选择栏。
pub(super) fn dialog_exit_selection() -> SelectionState<DialogExitChoice> {
    SelectionState::new(vec![
        SelectionItem::new(
            "保存并退出",
            "提交本轮修改并写入配置文件",
            DialogExitChoice::Save,
        ),
        SelectionItem::new("不保存退出", "放弃本轮字段修改", DialogExitChoice::Discard),
        SelectionItem::new("取消", "返回当前编辑弹窗", DialogExitChoice::Cancel),
    ])
}
